//! Sound playback (AU-002, AU-010, AU-011, EX-002, EX-019): the output
//! engine, keeping what it plays in step with the open project, and the
//! playback clock.
//!
//! The engine mixes on its own threads; everything here is cheap and runs
//! on the UI thread. While the engine plays, it is the master clock: the
//! playhead follows the time being heard. Without an output device the
//! system clock drives playback instead, silently.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Instant;

use crossbeam_channel::{Receiver, TryRecvError, bounded};
use freecut_audio::{
    AudioEngine, DeviceIssue, DocSchedule, EngineConfig, EngineStatus, FileOpener, SourceOpener,
    gain_amplitude,
};
use freecut_timeline::ids::TrackId;
use freecut_timeline::model::Document;
use freecut_timeline::time::{TICKS_PER_SECOND, Tick};

use crate::session::Project;

/// Shown when no output device could be opened (AU-010).
pub const NO_DEVICE: &str = "No audio output device — playing without sound";

/// Called from helper threads to repaint the window.
type Wake = Arc<dyn Fn() + Send + Sync>;

/// The audio output and what it plays.
pub struct Sound {
    engine: Option<AudioEngine>,
    /// The device is being opened on a helper thread, with this config.
    starting: Option<(Receiver<Result<AudioEngine, String>>, EngineConfig)>,
    /// Why there is no engine.
    unavailable: Option<String>,
    /// The device and buffer asked for (Preferences, UI-029).
    config: EngineConfig,
    /// [`Sound::start`] was called: until then the settings aren't read
    /// yet and nothing is known about the device.
    started: bool,
    wake: Wake,
    /// Kept across edits so open readers stay cached; rebuilt only when the
    /// media files change.
    opener: Arc<dyn SourceOpener>,
    /// (project serial, media revision) the opener was built for.
    opener_key: Option<(u64, u64)>,
    /// What the engine plays: (project serial, document revision, solo set).
    schedule_key: Option<(u64, u64, BTreeSet<TrackId>)>,
    /// Preview mute (EX-019): app state, never saved in the project.
    muted: bool,
}

impl std::fmt::Debug for Sound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sound")
            .field("engine", &self.engine.is_some())
            .field("unavailable", &self.unavailable)
            .field("config", &self.config)
            .field("muted", &self.muted)
            .finish_non_exhaustive()
    }
}

/// What the output is doing, for the banner and Preferences (AU-010).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputState {
    /// The settings aren't read yet, or the device is opening.
    Opening,
    /// No device could be opened; the cause, when known.
    NoDevice(Option<String>),
    /// An engine runs: which device plays, and the error of a lost device
    /// that couldn't be reopened.
    Running {
        status: EngineStatus,
        lost: Option<String>,
    },
}

/// The banner above the editor (AU-010): why sound is missing or comes
/// from another device than the one chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Banner {
    pub text: String,
    /// More detail on hover.
    pub detail: Option<String>,
}

impl Sound {
    /// Sound that opens no device until [`Sound::start`]. `wake` is called
    /// from helper threads when something finished.
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Sound {
        Sound {
            engine: None,
            starting: None,
            unavailable: None,
            config: EngineConfig::default(),
            started: false,
            wake: Arc::new(wake),
            opener: Arc::new(FileOpener::default()),
            opener_key: None,
            schedule_key: None,
            muted: false,
        }
    }

    /// Opens the configured output device on a helper thread, so a slow
    /// device never stalls the window.
    pub fn start(&mut self, config: EngineConfig) {
        self.config = config;
        self.started = true;
        self.spawn_start();
    }

    fn spawn_start(&mut self) {
        let (tx, rx) = bounded(1);
        let config = self.config.clone();
        let wake = Arc::clone(&self.wake);
        let spawned = std::thread::Builder::new()
            .name("freecut-audio-start".into())
            .spawn(move || {
                let engine = AudioEngine::start(config).map_err(|e| e.to_string());
                let _ = tx.send(engine);
                wake();
            });
        match spawned {
            Ok(_) => {
                self.starting = Some((rx, self.config.clone()));
                self.unavailable = None;
            }
            Err(e) => {
                self.starting = None;
                self.unavailable = Some(e.to_string());
            }
        }
    }

    /// Applies new Preferences (UI-029): a running engine switches device
    /// or buffer and keeps its position; without one, opening is tried
    /// again with the new choice.
    pub fn configure(&mut self, config: EngineConfig) {
        if !self.started {
            self.config = config;
            return;
        }
        if config == self.config && (self.engine.is_some() || self.starting.is_some()) {
            return;
        }
        self.config = config;
        if let Some(engine) = &self.engine {
            engine.reconfigure(self.config.clone());
        } else if self.starting.is_none() {
            self.spawn_start();
        }
    }

    /// Picks up the engine once the device has opened. Returns a message
    /// for the status line when it couldn't be opened.
    pub fn poll(&mut self) -> Option<String> {
        let (rx, opened_with) = self.starting.as_ref()?;
        let opened_with = opened_with.clone();
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err("the audio thread stopped".to_owned()),
        };
        self.starting = None;
        match result {
            Ok(engine) => {
                engine.set_muted(self.muted);
                // A choice made while the device was opening.
                if opened_with != self.config {
                    engine.reconfigure(self.config.clone());
                }
                self.engine = Some(engine);
                self.schedule_key = None;
                None
            }
            Err(e) => {
                eprintln!("FreeCut: no audio output: {e}");
                self.unavailable = Some(e);
                Some(NO_DEVICE.to_owned())
            }
        }
    }

    /// Why there is no sound right now, if there's a reason: no device, or
    /// a device that was lost and couldn't be reopened.
    pub fn problem(&self) -> Option<String> {
        match self.output_state() {
            OutputState::Opening => None,
            OutputState::NoDevice(cause) => Some(match cause {
                Some(cause) => format!("{NO_DEVICE} ({cause})"),
                None => NO_DEVICE.to_owned(),
            }),
            OutputState::Running { lost, .. } => lost,
        }
    }

    /// What the output is doing now.
    pub fn output_state(&self) -> OutputState {
        match &self.engine {
            Some(engine) => OutputState::Running {
                status: engine.status(),
                lost: engine.last_error(),
            },
            None if !self.started || self.starting.is_some() => OutputState::Opening,
            None => OutputState::NoDevice(self.unavailable.clone()),
        }
    }

    /// Gives the engine the project's current sound when the document, its
    /// media files or the solo set (AU-012) changed since the last call.
    /// `serial` tells projects apart. Cheap: the document is shared, not
    /// copied.
    pub fn sync(&mut self, serial: u64, project: &Project) {
        let Some(engine) = &self.engine else { return };
        let media_key = (serial, project.media_revision());
        if self.opener_key != Some(media_key) {
            self.opener = Arc::new(FileOpener::from_paths(
                &project.media_paths,
                &project.offline,
            ));
            self.opener_key = Some(media_key);
            self.schedule_key = None;
        }
        let solo = project.solo();
        let fresh = self.schedule_key.as_ref().is_some_and(|(s, r, set)| {
            *s == serial && *r == project.editor.revision() && set == solo
        });
        if !fresh {
            // Neither call restarts playback: queued audio plays out and the
            // change is heard after the read-ahead, with clip, track, solo
            // and master changes ramped (AU-010, AU-012).
            engine.set_master_gain(master_amplitude(project.doc()));
            let schedule = DocSchedule::root(project.doc().clone())
                .with_solo((!solo.is_empty()).then(|| solo.clone()));
            engine.set_source(Arc::new(schedule), Arc::clone(&self.opener));
            self.schedule_key = Some((serial, project.editor.revision(), solo.clone()));
        }
    }

    /// Stops and forgets the open project's sound (closing or switching
    /// projects).
    pub fn reset(&mut self) {
        if let Some(engine) = &self.engine {
            engine.pause();
            engine.set_source(
                Arc::new(freecut_audio::Silence),
                Arc::new(freecut_audio::Silence),
            );
        }
        self.opener = Arc::new(FileOpener::default());
        self.opener_key = None;
        self.schedule_key = None;
    }

    pub fn play(&self, from: Tick) {
        if let Some(engine) = &self.engine {
            engine.play(from);
        }
    }

    pub fn pause(&self) {
        if let Some(engine) = &self.engine {
            engine.pause();
        }
    }

    pub fn seek(&self, to: Tick) {
        if let Some(engine) = &self.engine {
            engine.seek(to);
        }
    }

    /// The time being heard, while the engine plays.
    pub fn heard(&self) -> Option<Tick> {
        self.engine
            .as_ref()
            .filter(|e| e.is_playing())
            .and_then(AudioEngine::position)
    }

    /// "Hear yourself" (AU-027): plays the recorder's feed on the output;
    /// `None` stops it. Returns whether it plays (no output, or another
    /// rate than the device's, plays nothing).
    pub fn set_monitor(&self, feed: Option<freecut_audio::record::MonitorFeed>) -> bool {
        match &self.engine {
            Some(engine) => engine.set_monitor(feed),
            None => false,
        }
    }

    pub fn muted(&self) -> bool {
        self.muted
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        if let Some(engine) = &self.engine {
            engine.set_muted(muted);
        }
    }
}

/// The master volume of `doc`'s root sequence as linear amplitude (AU-010).
pub fn master_amplitude(doc: &Document) -> f32 {
    doc.sequence(doc.root())
        .map_or(1.0, |s| gain_amplitude(s.master.gain))
}

#[cfg(test)]
impl Sound {
    /// Sound without an output device, for tests.
    pub fn without_device() -> Sound {
        Sound {
            unavailable: Some("test".to_owned()),
            started: true,
            ..Sound::new(|| {})
        }
    }
}

/// The banner for `state` (AU-010), if any: no device at all, a lost
/// device, or a chosen device that isn't the one playing.
pub fn banner(state: &OutputState) -> Option<Banner> {
    match state {
        OutputState::Opening => None,
        OutputState::NoDevice(cause) => Some(Banner {
            text: format!("{NO_DEVICE}. Connect a device, or choose another one."),
            detail: cause.clone(),
        }),
        OutputState::Running {
            lost: Some(error), ..
        } => Some(Banner {
            text: "The audio output device stopped working — playing without sound. \
                   Reconnect it, or choose another one."
                .to_owned(),
            detail: Some(error.clone()),
        }),
        OutputState::Running { status, lost: None } => {
            let reason = status.reason?;
            let mut text = capitalized(&reason.to_string());
            if status.using_fallback {
                if status.device_name.is_empty() {
                    text.push_str(" and no other device opened — playing without sound.");
                } else {
                    text.push_str(&format!(
                        "; playing on the system default (“{}”) instead.",
                        status.device_name
                    ));
                }
            } else {
                text.push_str("; trying it again…");
            }
            let advice = match reason {
                DeviceIssue::Busy => " Close the other app, or choose another device.",
                DeviceIssue::AccessDenied => {
                    " Allow FreeCut to use it in the system settings, or choose another device."
                }
                DeviceIssue::NotConnected => " Connect it, or choose another device.",
                DeviceIssue::NotResponding | DeviceIssue::Unsupported => " Choose another device.",
            };
            text.push_str(advice);
            Some(Banner {
                text,
                detail: status.requested.clone(),
            })
        }
    }
}

/// One line about the output for Preferences (UI-029).
pub fn output_line(state: &OutputState) -> String {
    match state {
        OutputState::Opening => "Opening the output device…".to_owned(),
        OutputState::NoDevice(_) => NO_DEVICE.to_owned(),
        OutputState::Running { lost: Some(_), .. } => {
            "The output device stopped working — playing without sound".to_owned()
        }
        OutputState::Running { status, .. } if status.device_name.is_empty() => {
            "Waiting for the output device…".to_owned()
        }
        OutputState::Running { status, .. } => match status.reason {
            Some(reason) if status.using_fallback => {
                format!("Playing on “{}”: {reason}", status.device_name)
            }
            _ => format!("Playing on “{}”", status.device_name),
        },
    }
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().collect::<String>() + chars.as_str()
    })
}

/// Playback in progress: where and when it started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Playback {
    pub from: Tick,
    pub started: Instant,
}

impl Playback {
    pub fn new(from: Tick) -> Playback {
        Playback {
            from,
            started: Instant::now(),
        }
    }

    /// The playhead at `now`: the time being heard when the audio engine
    /// plays (EX-002: audio is the master clock), else the start position
    /// plus the time elapsed on the system clock.
    pub fn time(&self, now: Instant, heard: Option<Tick>) -> Tick {
        if let Some(t) = heard {
            return t;
        }
        let elapsed = now.saturating_duration_since(self.started).as_nanos();
        let ticks = elapsed.saturating_mul(TICKS_PER_SECOND as u128) / 1_000_000_000;
        let ticks = i64::try_from(ticks).unwrap_or(i64::MAX);
        self.from
            .checked_add(Tick::from_raw(ticks))
            .unwrap_or(Tick::MAX)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::float_cmp)] // test code

    use std::time::Duration;

    use super::*;

    #[test]
    fn the_audio_clock_wins_while_it_plays() {
        let play = Playback::new(Tick::from_secs(2).unwrap());
        let later = play.started + Duration::from_millis(500);
        let heard = Tick::from_secs(7).unwrap();
        assert_eq!(play.time(later, Some(heard)), heard);
    }

    #[test]
    fn without_audio_the_system_clock_drives_playback() {
        let play = Playback::new(Tick::from_secs(2).unwrap());
        assert_eq!(play.time(play.started, None), Tick::from_secs(2).unwrap());
        let later = play.started + Duration::from_millis(1_500);
        assert_eq!(
            play.time(later, None),
            Tick::from_raw(Tick::from_secs(7).unwrap().raw() / 2)
        );
        // A clock reading from before the start never runs backwards.
        let earlier = play.started.checked_sub(Duration::from_millis(5));
        if let Some(earlier) = earlier {
            assert_eq!(play.time(earlier, None), Tick::from_secs(2).unwrap());
        }
    }

    #[test]
    fn the_banner_names_the_cause_and_what_plays_instead() {
        assert_eq!(banner(&OutputState::Opening), None);
        let none = banner(&OutputState::NoDevice(Some("no device".to_owned()))).unwrap();
        assert!(none.text.starts_with(NO_DEVICE), "{}", none.text);
        assert_eq!(none.detail.as_deref(), Some("no device"));

        let running = |status: EngineStatus, lost: Option<&str>| OutputState::Running {
            status,
            lost: lost.map(str::to_owned),
        };
        let fine = EngineStatus {
            device_name: "Speakers".to_owned(),
            ..EngineStatus::default()
        };
        assert_eq!(banner(&running(fine.clone(), None)), None);
        assert_eq!(
            output_line(&running(fine.clone(), None)),
            "Playing on “Speakers”"
        );

        let busy = EngineStatus {
            device_name: "Speakers".to_owned(),
            requested: Some("Studio".to_owned()),
            using_fallback: true,
            reason: Some(DeviceIssue::Busy),
        };
        let b = banner(&running(busy.clone(), None)).unwrap();
        assert_eq!(
            b.text,
            "The chosen output device is in use by another app; playing on the system default \
             (“Speakers”) instead. Close the other app, or choose another device."
        );
        assert_eq!(b.detail.as_deref(), Some("Studio"));
        assert_eq!(
            output_line(&running(busy, None)),
            "Playing on “Speakers”: the chosen output device is in use by another app"
        );

        let retry = EngineStatus {
            device_name: String::new(),
            requested: Some("Studio".to_owned()),
            using_fallback: false,
            reason: Some(DeviceIssue::NotResponding),
        };
        let b = banner(&running(retry, None)).unwrap();
        assert!(
            b.text.contains("didn't respond; trying it again…"),
            "{}",
            b.text
        );

        let lost = banner(&running(fine, Some("device gone"))).unwrap();
        assert!(lost.text.contains("stopped working"));
        assert_eq!(lost.detail.as_deref(), Some("device gone"));
    }

    #[test]
    fn preferences_wait_for_the_settings_and_retry_without_an_engine() {
        let mut sound = Sound::new(|| {});
        assert_eq!(sound.output_state(), OutputState::Opening);
        assert_eq!(sound.problem(), None);
        // Before start, a choice is only remembered.
        let config = EngineConfig {
            device: Some("Desk".to_owned()),
            buffer_frames: 512,
        };
        sound.configure(config.clone());
        assert_eq!(sound.config, config);
        assert_eq!(sound.output_state(), OutputState::Opening);

        let failed = Sound::without_device();
        assert_eq!(
            failed.output_state(),
            OutputState::NoDevice(Some("test".to_owned()))
        );
        assert!(failed.problem().unwrap().starts_with(NO_DEVICE));
        assert!(banner(&failed.output_state()).is_some());
    }

    #[test]
    fn the_engine_plays_the_saved_master_volume() {
        use freecut_timeline::edit::EditRequest;
        use freecut_timeline::history::Coalesce;
        use freecut_timeline::model::{GainDb, ProjectSettings};

        let mut project = Project::new(ProjectSettings::new_default().unwrap()).unwrap();
        assert_eq!(master_amplitude(project.doc()), 1.0);
        let seq = project.doc().root();
        let mut set = |hundredths| {
            let gain = GainDb::from_hundredths(hundredths);
            let req = EditRequest::SetMasterGain { seq, gain };
            project.apply(&req, Coalesce::Never).unwrap();
            master_amplitude(project.doc())
        };
        assert!((set(-602) - 0.5).abs() < 1e-3);
        assert!((set(1_200) - 3.981).abs() < 1e-3);
        assert_eq!(set(i32::MIN), 0.0);
    }
}
