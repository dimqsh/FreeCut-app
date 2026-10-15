//! The voiceover recorder panel (AU-027, UI-011): a floating panel above
//! the timeline toolbar's microphone button.
//!
//! It holds the input choice, the input level and a live meter (peak hold
//! and a clip light), channels, where the take goes, countdown, pre-roll,
//! "Hear yourself", muting the timeline, a large Record / Stop button,
//! *Redo last take* and the Takes list.
//!
//! [`RecorderPanel`] is the panel's state machine. The recorder itself runs
//! on a [`RecorderHost`]'s threads; the panel only sends commands and reads
//! the host's messages, and it tells the app what to do through
//! [`PanelEvent`]s (start the timeline, place a finished take, remove an
//! accidental one…) and [`PanelAction`]s (what the user clicked). Nothing
//! here blocks or touches files.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use eframe::egui::{self, Align2, Rect, RichText, Sense, Stroke, Vec2};
use freecut_audio::record::{Channels, MonitorFeed, RecordError, RecorderState, TakeInfo};
use freecut_audio::record::{EndReason, METER_FLOOR_DB};
use freecut_timeline::edit::{RecordTarget, first_free_audio_track, recorded_start};
use freecut_timeline::ids::{ClipId, MediaId};
use freecut_timeline::model::{
    ClipContent, Document, GainDb, GeneratedMediaKind, MediaOrigin, Sequence,
};
use freecut_timeline::time::{SampleRate, TICKS_PER_SECOND, Tick, TimeRange};

use super::timeline::LiveTake;
use crate::recorder::{
    Backend, HostEvent, InputChoice, Levels, OpenRequest, RecorderHost, TakeRequest, Wake,
    duration_ticks, frames_to_ticks, take_stamp,
};
use crate::settings::{COUNTDOWNS, MAX_PREROLL_SECS, VoiceoverSettings};
use crate::theme::{self, icons};
use crate::widgets::icon_button::IconButton;
use crate::widgets::segmented::{Segment, Segmented};
use crate::widgets::select::{Select, SelectOption};
use crate::widgets::sheet::{ButtonKind, button};
use crate::widgets::slider::SliderField;
use crate::widgets::toggle::toggle;

/// The panel's width.
const WIDTH: f32 = 340.0;
/// The meter's range (AU-027): −60 to 0 dBFS, green below −12, yellow to
/// −3, red above.
const METER_MIN_DB: f32 = -60.0;
const METER_YELLOW_DB: f32 = -12.0;
const METER_RED_DB: f32 = -3.0;
/// Live-waveform samples kept per take (about 20 minutes at 30 a second);
/// older ones are merged pairwise.
const MAX_PEAKS: usize = 36_000;

/// Edits are refused with this while a take counts down or records.
pub const EDIT_LOCK: &str = "Stop recording to edit";

/// What the panel shows at the top.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    Closed,
    /// The microphone is being opened.
    Opening,
    /// No usable input, and why.
    Waiting(Waiting),
    Ready,
    CountingDown,
    PreRoll,
    Recording,
    Stopping,
}

/// Why the panel waits (AU-027's edge cases).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Waiting {
    NoDevice,
    /// This build has no microphone support (the `record` feature).
    NotBuilt,
    /// The microphone went away during a take.
    Lost,
    /// The system doesn't allow FreeCut to use the microphone.
    Denied,
    /// Another app holds it, or it failed to open.
    Busy(String),
    Failed(String),
}

impl Waiting {
    /// The sentence the panel shows (FreeCut wording).
    pub fn text(&self) -> String {
        match self {
            Waiting::NoDevice => "No microphone found. Connect one and press Try again.".to_owned(),
            Waiting::NotBuilt => {
                "This build of FreeCut was made without microphone support".to_owned()
            }
            Waiting::Lost => "The microphone was disconnected".to_owned(),
            Waiting::Denied => "FreeCut isn't allowed to use the microphone. Allow it in your \
                 system's privacy settings, then press Try again. Nothing is recorded until then."
                .to_owned(),
            Waiting::Busy(detail) => format!(
                "The microphone is in use by another app ({detail}). Close that app or choose \
                 another input."
            ),
            Waiting::Failed(detail) => {
                format!("The microphone couldn't be opened: {detail}. Choose another input.")
            }
        }
    }
}

/// Why opening the microphone failed, as the panel says it.
pub fn waiting_for(e: &RecordError) -> Waiting {
    match e {
        RecordError::NoDevice if !Backend::microphones_built() => Waiting::NotBuilt,
        RecordError::NoDevice | RecordError::DeviceNotFound(_) => Waiting::NoDevice,
        RecordError::Denied(_) => Waiting::Denied,
        RecordError::Busy(detail) => Waiting::Busy(detail.clone()),
        RecordError::UnsupportedFormat(_) | RecordError::UnsupportedRate(_) => {
            Waiting::Failed("its sound format isn't supported".to_owned())
        }
        other => Waiting::Failed(other.to_string()),
    }
}

/// The take being recorded.
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveTake {
    /// The start position: the playhead when Record was pressed (fixed from
    /// then on).
    pub start: Tick,
    /// The pre-roll asked for (never more than the start position).
    pub preroll: Tick,
    pub target: RecordTarget,
    /// The take *Redo last take* replaces.
    pub replace: Option<ClipId>,
    /// The take's file, once the recorder created it.
    pub path: Option<PathBuf>,
    pub stopping: bool,
    pub cancelling: bool,
    /// Past the countdown: the timeline was asked to play.
    pub rolling: bool,
    /// When the timeline started playing and from where.
    pub playback: Option<(Instant, Tick)>,
}

/// A take that is on disk but not on the timeline: cancelled, or recovered
/// after a crash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LooseTake {
    pub path: PathBuf,
    /// Where it would have gone (a cancelled take); `None`: the playhead.
    pub at: Option<Tick>,
    /// The pre-roll handle before the take's start.
    pub handle_in: Tick,
    pub recovered: bool,
}

/// A finished take to place: probe `path` on a worker, then
/// `Project::insert_recorded`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub path: PathBuf,
    pub at: Tick,
    pub handle_in: Tick,
    pub target: RecordTarget,
    pub replace: Option<ClipId>,
    /// Where the take was meant to start (for *Redo last take*).
    pub start: Tick,
}

/// What the app should do after [`RecorderPanel::update`].
#[derive(Debug)]
pub enum PanelEvent {
    /// The countdown is over: play the timeline from here.
    StartPlayback(Tick),
    /// The take ended: stop the timeline.
    StopPlayback,
    /// Hand "Hear yourself"'s feed to the output (`None`: take it back).
    Monitor(Option<MonitorFeed>),
    Place(Placement),
    /// An accidental take (under 0.3 s): remove its file on a worker.
    Discard(PathBuf),
    /// A cancelled take was kept, not placed: offer *Place*.
    Kept(PathBuf),
    Notice(String),
}

/// What the user asked for in the panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelAction {
    Record,
    Stop,
    Cancel,
    Close,
    RedoLastTake,
    /// Place a take from the Takes list.
    Place(PathBuf),
    ShowOnTimeline(MediaId),
    Remove(MediaId),
    OpenPrivacySettings,
}

/// The recorder panel's state.
pub struct RecorderPanel {
    pub open: bool,
    backend: Backend,
    wake: Wake,
    host: Option<RecorderHost>,
    phase: Phase,
    devices: Option<Result<Vec<InputChoice>, String>>,
    rate: Option<SampleRate>,
    latency: Duration,
    levels: Option<Levels>,
    take: Option<ActiveTake>,
    /// The take's live waveform, oldest first.
    peaks: Vec<f32>,
    /// Takes on disk that aren't on the timeline.
    loose: Vec<LooseTake>,
    /// The last take placed from this panel and where it started.
    last_take: Option<(ClipId, Tick)>,
    /// Closing was asked while a take runs: *Keep*, *Discard*, *Go back*.
    confirm_close: bool,
    /// The panel closes once the running take is finished.
    close_after_take: bool,
    /// What the open microphone was opened with.
    opened_with: Option<OpenRequest>,
    /// A line under the meter (a low-disk warning).
    warning: Option<String>,
    events: Vec<PanelEvent>,
    /// The panel's size last frame (it hangs above the toolbar).
    size: Vec2,
}

impl std::fmt::Debug for RecorderPanel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecorderPanel")
            .field("open", &self.open)
            .field("phase", &self.phase)
            .field("take", &self.take)
            .finish_non_exhaustive()
    }
}

impl RecorderPanel {
    pub fn new(backend: Backend, wake: Wake) -> RecorderPanel {
        RecorderPanel {
            open: false,
            backend,
            wake,
            host: None,
            phase: Phase::Closed,
            devices: None,
            rate: None,
            latency: Duration::ZERO,
            levels: None,
            take: None,
            peaks: Vec::new(),
            loose: Vec::new(),
            last_take: None,
            confirm_close: false,
            close_after_take: false,
            opened_with: None,
            warning: None,
            events: Vec::new(),
            size: Vec2::new(WIDTH + 16.0, 520.0),
        }
    }

    /// Uses `backend` for the next microphone (tests).
    #[cfg(test)]
    pub fn set_backend(&mut self, backend: Backend) {
        self.backend = backend;
    }

    #[cfg(test)]
    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    #[cfg(test)]
    pub fn take(&self) -> Option<&ActiveTake> {
        self.take.as_ref()
    }

    #[cfg(test)]
    pub fn loose_takes(&self) -> &[LooseTake] {
        &self.loose
    }

    #[cfg(test)]
    pub fn levels(&self) -> Option<Levels> {
        self.levels
    }

    /// A take is counting down, recording or being finished.
    pub fn running(&self) -> bool {
        self.take.is_some()
    }

    /// Edits are refused (AU-027): a take counts down or records, and its
    /// start must not move under it.
    pub fn locks_edits(&self) -> bool {
        self.take
            .as_ref()
            .is_some_and(|t| !t.stopping && !t.cancelling)
    }

    /// Opens the panel and the microphone (on the host's thread).
    pub fn show_panel(&mut self, prefs: &VoiceoverSettings, project_rate: u32) {
        self.open = true;
        self.close_after_take = false;
        if self.host.is_none() {
            match RecorderHost::spawn(self.backend.clone(), self.wake.clone()) {
                Ok(host) => self.host = Some(host),
                Err(e) => {
                    self.phase = Phase::Waiting(Waiting::Failed(e.to_string()));
                    return;
                }
            }
        }
        if let Some(host) = &self.host {
            host.list();
        }
        if matches!(self.phase, Phase::Closed | Phase::Waiting(_)) {
            self.reopen(prefs, project_rate);
        }
    }

    /// Opens the microphone again with `prefs` (another input, channels,
    /// or *Try again*). Only while no take runs.
    pub fn reopen(&mut self, prefs: &VoiceoverSettings, project_rate: u32) {
        if self.take.is_some() {
            return;
        }
        let Some(host) = &self.host else { return };
        let request = open_request(prefs, project_rate);
        host.open(request.clone());
        self.opened_with = Some(request);
        self.phase = Phase::Opening;
        self.levels = None;
    }

    /// Ends the recorder's thread and waits for a running take's file to
    /// be finished (quitting).
    pub fn shutdown_and_wait(&mut self, timeout: Duration) {
        if let Some(host) = self.host.take() {
            host.shutdown_and_wait(timeout);
        }
    }

    /// Lists the inputs again.
    pub fn refresh_devices(&mut self) {
        if let Some(host) = &self.host {
            self.devices = None;
            host.list();
        }
    }

    /// Asks to close: right away when idle; with a take running, the panel
    /// asks *Keep*, *Discard* or *Go back* first.
    pub fn request_close(&mut self) {
        if self.take.is_some() {
            self.confirm_close = true;
        } else {
            self.close();
        }
    }

    /// Closes the panel and the microphone.
    fn close(&mut self) {
        self.open = false;
        self.confirm_close = false;
        self.close_after_take = false;
        if let Some(host) = self.host.take() {
            host.close();
        }
        self.phase = Phase::Closed;
        self.levels = None;
        self.opened_with = None;
        self.warning = None;
        self.events.push(PanelEvent::Monitor(None));
    }

    /// The project closed: a running take is cancelled (and kept on disk),
    /// and the Takes list starts over.
    pub fn project_changed(&mut self) {
        if let (Some(host), Some(_)) = (&self.host, &self.take) {
            host.cancel();
        }
        self.take = None;
        self.peaks.clear();
        self.loose.clear();
        self.last_take = None;
        self.close();
        self.events.clear();
        self.events.push(PanelEvent::Monitor(None));
    }

    /// Starts a take at `start` (the playhead): after the countdown, with
    /// up to the chosen pre-roll before it. `folders` are where the file
    /// may go, in order. `redo` is the take *Redo last take* replaces.
    pub fn begin_take(
        &mut self,
        start: Tick,
        prefs: &VoiceoverSettings,
        folders: Vec<PathBuf>,
        redo: Option<ClipId>,
        now: SystemTime,
    ) -> Result<(), String> {
        if self.take.is_some() {
            return Err("A take is already running".to_owned());
        }
        if self.phase != Phase::Ready {
            return Err(match &self.phase {
                Phase::Waiting(why) => why.text(),
                _ => "The microphone isn't ready yet".to_owned(),
            });
        }
        let Some(host) = &self.host else {
            return Err("The microphone isn't open".to_owned());
        };
        let wanted = Duration::from_secs(u64::from(prefs.preroll_secs.min(MAX_PREROLL_SECS)));
        // Playback can't start before 0: the pre-roll stops there.
        let preroll = duration_ticks(wanted).min(start.max(Tick::ZERO));
        let preroll_dur = Duration::from_nanos(
            u64::try_from(i128::from(preroll.raw()) * 1_000_000_000 / i128::from(TICKS_PER_SECOND))
                .unwrap_or(0),
        );
        host.take(TakeRequest {
            folders,
            stamp: take_stamp(now),
            channels: channels(prefs),
            gain: GainDb::from_hundredths(prefs.input_level),
            countdown: Duration::from_secs(u64::from(prefs.countdown_secs)),
            preroll: preroll_dur,
            monitor: prefs.monitor,
        });
        self.take = Some(ActiveTake {
            start,
            preroll,
            target: if prefs.new_track {
                RecordTarget::NewTrack
            } else {
                RecordTarget::FirstFree
            },
            replace: redo,
            path: None,
            stopping: false,
            cancelling: false,
            rolling: false,
            playback: None,
        });
        self.peaks.clear();
        self.warning = None;
        Ok(())
    }

    /// Stop: the take ends and is placed. A countdown ends with nothing
    /// recorded.
    pub fn stop(&mut self) {
        if let (Some(host), Some(take)) = (&self.host, &mut self.take) {
            host.stop();
            take.stopping = true;
        }
    }

    /// `Esc` and *Cancel*: a countdown or pre-roll ends with nothing kept;
    /// a take that is recording is finished, kept in the Takes list and not
    /// placed. With no take, the panel closes.
    pub fn cancel(&mut self) {
        match (&self.host, &mut self.take) {
            (Some(host), Some(take)) => {
                host.cancel();
                take.cancelling = true;
            }
            _ => self.request_close(),
        }
    }

    /// The timeline started playing for the take.
    pub fn playback_started(&mut self, at: Instant, from: Tick) {
        if let Some(take) = &mut self.take {
            take.playback = Some((at, from));
        }
    }

    /// The take *Redo last take* replaces, if it is still on the timeline.
    pub fn redo_target(&self, doc: &Document) -> Option<(ClipId, Tick)> {
        self.last_take.filter(|(clip, _)| doc.clip(*clip).is_some())
    }

    /// A take was placed (as `clip`, meant to start at `start`).
    pub fn placed(&mut self, path: &Path, clip: ClipId, start: Tick) {
        self.last_take = Some((clip, start));
        self.loose.retain(|t| t.path != path);
    }

    /// A take couldn't be placed: keep it in the Takes list.
    pub fn not_placed(&mut self, placement: &Placement) {
        if !self.loose.iter().any(|t| t.path == placement.path) {
            self.loose.push(LooseTake {
                path: placement.path.clone(),
                at: Some(placement.at),
                handle_in: placement.handle_in,
                recovered: false,
            });
        }
    }

    /// Takes found after a crash (PM-043), offered in the Takes list.
    pub fn recovered(&mut self, paths: Vec<PathBuf>) {
        for path in paths {
            if !self.loose.iter().any(|t| t.path == path) {
                self.loose.push(LooseTake {
                    path,
                    at: None,
                    handle_in: Tick::ZERO,
                    recovered: true,
                });
            }
        }
    }

    /// The take in the Takes list at `path`.
    pub fn loose_take(&self, path: &Path) -> Option<&LooseTake> {
        self.loose.iter().find(|t| t.path == path)
    }

    /// Reads what the recorder reported. `offset_ms` is the latency offset
    /// preference.
    pub fn update(&mut self, offset_ms: i32) -> Vec<PanelEvent> {
        let incoming = self
            .host
            .as_ref()
            .map(RecorderHost::poll)
            .unwrap_or_default();
        for event in incoming {
            self.host_event(event, offset_ms);
        }
        std::mem::take(&mut self.events)
    }

    fn host_event(&mut self, event: HostEvent, offset_ms: i32) {
        match event {
            HostEvent::Devices(list) => {
                self.devices = Some(list.map_err(|e| e.to_string()));
            }
            HostEvent::Opened {
                rate,
                latency,
                monitor,
            } => {
                self.rate = Some(rate);
                self.latency = latency;
                self.phase = Phase::Ready;
                if monitor.is_some() {
                    self.events.push(PanelEvent::Monitor(monitor));
                }
            }
            HostEvent::OpenFailed(e) => self.phase = Phase::Waiting(waiting_for(&e)),
            HostEvent::Armed(path) => {
                if let Some(take) = &mut self.take {
                    take.path = Some(path);
                }
            }
            HostEvent::TakeRefused(why) => {
                self.take = None;
                self.events.push(PanelEvent::Notice(why));
            }
            HostEvent::State(state) => self.state(state),
            HostEvent::Levels(levels) => {
                let before = self.levels.map(|l| l.state);
                self.levels = Some(levels);
                if before != Some(levels.state) {
                    self.state(levels.state);
                }
                if levels.state == RecorderState::Recording {
                    self.push_peak(levels.meter.peak_db);
                }
                // A countdown stopped or cancelled ends without a take.
                if levels.state == RecorderState::Ready
                    && let Some(take) = &self.take
                    && take.path.is_some()
                    && !take.rolling
                    && (take.stopping || take.cancelling)
                {
                    self.take = None;
                    self.after_take();
                }
            }
            HostEvent::Finished(info) => self.finished(&info, offset_ms),
            HostEvent::Failed(e) => {
                if self.take.take().is_some() {
                    self.events.push(PanelEvent::StopPlayback);
                }
                self.events.push(PanelEvent::Notice(format!(
                    "The take couldn't be written ({e}). What was recorded is kept and offered \
                     the next time the project opens"
                )));
                self.after_take();
            }
            HostEvent::LowDisk(free) => {
                let mb = free / 1_000_000;
                self.warning = Some(format!(
                    "The disk is almost full ({mb} MB free). Recording stops at 50 MB"
                ));
            }
        }
    }

    fn state(&mut self, state: RecorderState) {
        self.phase = match state {
            RecorderState::Waiting => Phase::Waiting(Waiting::Lost),
            RecorderState::Ready => Phase::Ready,
            RecorderState::CountingDown => Phase::CountingDown,
            RecorderState::PreRoll => Phase::PreRoll,
            RecorderState::Recording => Phase::Recording,
            RecorderState::Stopping => Phase::Stopping,
        };
        if matches!(state, RecorderState::PreRoll | RecorderState::Recording)
            && let Some(take) = &mut self.take
            && !take.rolling
        {
            take.rolling = true;
            let from = take.start.checked_sub(take.preroll).unwrap_or(Tick::ZERO);
            self.events
                .push(PanelEvent::StartPlayback(from.max(Tick::ZERO)));
        }
    }

    fn push_peak(&mut self, peak_db: f32) {
        let amp = if peak_db <= METER_FLOOR_DB {
            0.0
        } else {
            10f32.powf(peak_db / 20.0).clamp(0.0, 1.0)
        };
        self.peaks.push(amp);
        if self.peaks.len() > MAX_PEAKS {
            self.peaks = self
                .peaks
                .chunks(2)
                .map(|c| c.iter().copied().fold(0.0, f32::max))
                .collect();
        }
    }

    /// A take ended: place it, keep it, or remove it.
    fn finished(&mut self, info: &TakeInfo, offset_ms: i32) {
        let take = self.take.take();
        self.events.push(PanelEvent::StopPlayback);
        let Some(take) = take else {
            // A take this panel didn't start (it can't happen): keep it.
            self.loose.push(LooseTake {
                path: info.path.clone(),
                at: None,
                handle_in: frames_to_ticks(info.preroll_frames, info.rate),
                recovered: false,
            });
            self.after_take();
            return;
        };
        let handle_in = frames_to_ticks(info.preroll_frames, info.rate);
        let at = take_position(
            take.start,
            handle_in,
            info.started_at.instant(),
            take.playback,
            offset_ms,
            info.rate,
        );
        if info.ended_by == EndReason::Cancelled || take.cancelling {
            self.loose.push(LooseTake {
                path: info.path.clone(),
                at: Some(at),
                handle_in,
                recovered: false,
            });
            self.events.push(PanelEvent::Kept(info.path.clone()));
        } else if info.is_too_short() {
            self.events.push(PanelEvent::Discard(info.path.clone()));
            self.events
                .push(PanelEvent::Notice("Take too short".to_owned()));
        } else {
            if let Some(why) = end_notice(info.ended_by) {
                self.events.push(PanelEvent::Notice(why.to_owned()));
            }
            self.events.push(PanelEvent::Place(Placement {
                path: info.path.clone(),
                at,
                handle_in,
                target: take.target,
                replace: take.replace,
                start: take.start,
            }));
        }
        self.after_take();
    }

    fn after_take(&mut self) {
        self.peaks.clear();
        if self.close_after_take {
            self.close();
        }
    }

    /// The time recorded after the start so far.
    pub fn elapsed(&self) -> Option<Tick> {
        let take = self.take.as_ref()?;
        let levels = self.levels?;
        let rate = self.rate?;
        let preroll = crate::recorder::frames_in(
            Duration::from_nanos(
                u64::try_from(
                    i128::from(take.preroll.raw()) * 1_000_000_000 / i128::from(TICKS_PER_SECOND),
                )
                .unwrap_or(0),
            ),
            rate,
        );
        Some(frames_to_ticks(
            levels.captured.saturating_sub(preroll),
            rate,
        ))
    }

    /// What the toolbar's Record button shows beside it while a take runs:
    /// the countdown, then the elapsed time.
    pub fn button_text(&self) -> Option<String> {
        let take = self.take.as_ref()?;
        if !take.rolling {
            let left = self.levels.and_then(|l| l.countdown_left)?;
            return Some(countdown_number(left).to_string());
        }
        Some(clock(self.elapsed().unwrap_or(Tick::ZERO)))
    }

    /// The take as the timeline draws it: from its start, as long as what
    /// was recorded after it, on the track it would land on.
    pub fn live_take(&self, seq: &Sequence) -> Option<LiveTake> {
        let take = self.take.as_ref()?;
        if !take.rolling || self.phase == Phase::PreRoll {
            return None;
        }
        let length = self.elapsed()?;
        let track = match take.target {
            RecordTarget::NewTrack => None,
            RecordTarget::FirstFree => {
                let span = length.max(Tick::from_raw(1));
                TimeRange::new(take.start, span)
                    .ok()
                    .and_then(|range| first_free_audio_track(seq, range))
            }
        };
        Some(LiveTake {
            start: take.start,
            length,
            track,
            peaks: self.peaks.clone(),
        })
    }

    /// The meter, the controls and the Takes list. Settings the user
    /// changes are written to `prefs`; the clicks come back as actions.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        anchor: Rect,
        prefs: &mut VoiceoverSettings,
        doc: &Document,
        playhead: Tick,
    ) -> Vec<PanelAction> {
        let mut actions = Vec::new();
        if !self.open {
            return actions;
        }
        let project_rate = doc.settings().sample_rate.hz();
        let before = prefs.clone();
        let pos = if anchor.is_finite() && anchor.width() > 0.0 {
            anchor.right_top() - Vec2::new(0.0, 6.0)
        } else {
            ctx.content_rect().right_bottom() - Vec2::new(16.0, 260.0)
        };
        let room = (pos.y - ctx.content_rect().top() - 40.0).max(220.0);
        // Laid out from its size last frame, so its content is never
        // squeezed by the room below a guessed top edge.
        let size = self.size;
        let top_left = egui::pos2(
            pos.x - size.x,
            (pos.y - size.y).max(ctx.content_rect().top()),
        );
        let area = egui::Area::new(egui::Id::new("freecut-recorder"))
            .order(egui::Order::Foreground)
            .fixed_pos(top_left)
            .constrain(true)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(WIDTH);
                    // It hangs above the toolbar: what doesn't fit above
                    // scrolls, so the Record button stays uncovered.
                    egui::ScrollArea::vertical()
                        .id_salt("recorder-body")
                        .max_height(room)
                        .auto_shrink([false, true])
                        .show(ui, |ui| self.body(ui, prefs, doc, playhead, &mut actions));
                });
            });
        if area.response.rect.size() != self.size {
            self.size = area.response.rect.size();
            ctx.request_repaint();
        }
        self.after_settings(&before, prefs, project_rate);
        actions
    }

    /// Sends what changed in the settings to the recorder.
    fn after_settings(
        &mut self,
        before: &VoiceoverSettings,
        prefs: &VoiceoverSettings,
        project_rate: u32,
    ) {
        if prefs.input_level != before.input_level
            && let Some(host) = &self.host
        {
            host.set_gain(GainDb::from_hundredths(prefs.input_level));
        }
        if prefs.monitor != before.monitor
            && let Some(host) = &self.host
        {
            host.set_monitor(prefs.monitor);
        }
        let device_changed =
            prefs.input_device != before.input_device || prefs.stereo != before.stereo;
        if device_changed && self.take.is_none() {
            self.reopen(prefs, project_rate);
        }
    }

    fn body(
        &mut self,
        ui: &mut egui::Ui,
        prefs: &mut VoiceoverSettings,
        doc: &Document,
        playhead: Tick,
        actions: &mut Vec<PanelAction>,
    ) {
        let look = theme::look(ui.ctx());
        let p = look.palette;
        let idle = self.take.is_none();

        // Title and close.
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(icons::MICROPHONE)
                    .font(theme::fonts::icon_font(ui.ctx(), 16.0, false))
                    .color(p.text_secondary),
            );
            ui.label(RichText::new("Record voiceover").strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if IconButton::new(icons::CLOSE, "Close the recorder (Esc)")
                    .show(ui)
                    .clicked()
                {
                    actions.push(PanelAction::Close);
                }
            });
        });
        self.state_line(ui);
        if self.confirm_close {
            ui.add_space(4.0);
            ui.label("Stop and keep this take?");
            ui.horizontal(|ui| {
                if button(ui, ButtonKind::Primary, None, "Keep", 0.0, None, true).clicked() {
                    self.confirm_close = false;
                    self.close_after_take = true;
                    self.open = false;
                    actions.push(PanelAction::Stop);
                }
                if button(ui, ButtonKind::Danger, None, "Discard", 0.0, None, true).clicked() {
                    self.confirm_close = false;
                    self.close_after_take = true;
                    self.open = false;
                    actions.push(PanelAction::Cancel);
                }
                if button(ui, ButtonKind::Ghost, None, "Go back", 0.0, None, true).clicked() {
                    self.confirm_close = false;
                }
            });
        }
        if let Phase::Waiting(why) = &self.phase {
            let why = why.clone();
            ui.add_space(4.0);
            ui.label(RichText::new(why.text()).color(p.warning));
            ui.horizontal(|ui| {
                if why == Waiting::Denied
                    && crate::jobs::platform::microphone_settings_command(
                        crate::jobs::platform::Os::current(),
                    )
                    .is_some()
                    && button(
                        ui,
                        ButtonKind::Secondary,
                        None,
                        "Open privacy settings",
                        0.0,
                        None,
                        true,
                    )
                    .clicked()
                {
                    actions.push(PanelAction::OpenPrivacySettings);
                }
                if why != Waiting::NotBuilt
                    && button(
                        ui,
                        ButtonKind::Secondary,
                        None,
                        "Try again",
                        0.0,
                        None,
                        true,
                    )
                    .clicked()
                {
                    self.refresh_devices();
                    self.reopen(prefs, doc.settings().sample_rate.hz());
                }
            });
        }
        ui.add_space(6.0);

        ui.add_enabled_ui(idle, |ui| self.device_row(ui, prefs));
        form_row(ui, "Input level", |ui| {
            let level = SliderField::new(
                egui::Id::new("recorder-level"),
                "Input level",
                f64::from(prefs.input_level) / 100.0,
                -20.0..=20.0,
            )
            .unit("dB")
            .show(ui);
            if level.changed {
                // Within ±2000 hundredths: the slider's range.
                prefs.input_level = (level.value * 100.0).round() as i32;
            }
        });
        if self.meter(ui)
            && let Some(host) = &self.host
        {
            host.reset_clip();
        }
        if let Some(warning) = &self.warning {
            ui.label(RichText::new(warning).small().color(p.warning));
        }
        ui.add_space(4.0);
        self.time_line(ui, doc, playhead);
        ui.add_space(6.0);
        self.buttons(ui, doc, actions);
        ui.add_space(6.0);
        ui.separator();
        ui.add_enabled_ui(idle, |ui| choice_rows(ui, prefs));
        ui.add_space(4.0);
        ui.separator();
        takes_list(ui, doc, &self.loose, actions);
    }

    fn state_line(&self, ui: &mut egui::Ui) {
        let p = theme::look(ui.ctx()).palette;
        let (text, color) = match &self.phase {
            Phase::Closed | Phase::Opening => {
                ("Opening the microphone…".to_owned(), p.text_secondary)
            }
            Phase::Waiting(_) => ("Waiting".to_owned(), p.warning),
            Phase::Ready => ("Ready".to_owned(), p.success),
            Phase::CountingDown => {
                let left = self
                    .levels
                    .and_then(|l| l.countdown_left)
                    .map_or(String::new(), |d| format!(" {}", countdown_number(d)));
                (format!("Counting down{left}"), p.warning)
            }
            Phase::PreRoll => ("Pre-roll".to_owned(), p.warning),
            Phase::Recording => (
                format!(
                    "● Recording {}",
                    clock(self.elapsed().unwrap_or(Tick::ZERO))
                ),
                p.danger,
            ),
            Phase::Stopping => ("Stopping…".to_owned(), p.text_secondary),
        };
        ui.label(RichText::new(text).color(color).strong());
    }

    fn device_row(&mut self, ui: &mut egui::Ui, prefs: &mut VoiceoverSettings) {
        let devices: Vec<InputChoice> = match &self.devices {
            Some(Ok(list)) => list.clone(),
            _ => Vec::new(),
        };
        let mut labels = vec!["System default".to_owned()];
        labels.extend(devices.iter().map(|d| d.name.clone()));
        // A chosen input that isn't connected stays in the list, marked.
        let mut current = match &prefs.input_device {
            None => Some(0),
            Some(id) => devices.iter().position(|d| &d.id == id).map(|i| i + 1),
        };
        if current.is_none() {
            labels.push("Chosen input (not connected)".to_owned());
            current = Some(labels.len() - 1);
        }
        let options: Vec<SelectOption<'_>> = labels.iter().map(|l| SelectOption::new(l)).collect();
        let mut picked = None;
        form_row(ui, "Microphone", |ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if IconButton::new(icons::REFRESH, "List the inputs again")
                    .show(ui)
                    .clicked()
                {
                    self.refresh_devices();
                }
                picked = Select::new(egui::Id::new("recorder-device"), &options, current)
                    .fill_width()
                    .show(ui)
                    .picked;
            });
        });
        match picked {
            Some(0) => prefs.input_device = None,
            Some(i) => {
                if let Some(d) = devices.get(i - 1) {
                    prefs.input_device = Some(d.id.clone());
                }
            }
            None => {}
        }
    }

    /// The level meter and its clip light; returns whether the light was
    /// clicked (to clear it).
    fn meter(&self, ui: &mut egui::Ui) -> bool {
        let p = theme::look(ui.ctx()).palette;
        let reading = self.levels.map(|l| l.meter);
        let mut cleared = false;
        ui.horizontal(|ui| {
            let w = ui.available_width() - 22.0;
            let (rect, _) = ui.allocate_exact_size(Vec2::new(w.max(40.0), 10.0), Sense::hover());
            let painter = ui.painter();
            painter.rect_filled(rect, theme::R_XS, p.bg_field);
            let x = |db: f32| {
                let f = ((db - METER_MIN_DB) / -METER_MIN_DB).clamp(0.0, 1.0);
                rect.left() + f * rect.width()
            };
            if let Some(m) = reading {
                // Coloured by range: green, yellow from −12, red from −3.
                for (lo, hi, color) in [
                    (METER_MIN_DB, METER_YELLOW_DB, p.success),
                    (METER_YELLOW_DB, METER_RED_DB, p.warning),
                    (METER_RED_DB, 0.0, p.danger),
                ] {
                    let top = m.peak_db.min(hi);
                    if top > lo {
                        let r = Rect::from_x_y_ranges(x(lo)..=x(top), rect.y_range());
                        painter.rect_filled(r, 0.0, color);
                    }
                }
                if m.held_peak_db > METER_MIN_DB {
                    let hx = x(m.held_peak_db);
                    painter.vline(hx, rect.y_range(), Stroke::new(2.0, p.text_primary));
                }
            }
            let clipped = reading.is_some_and(|m| m.clipped);
            let (light, response) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::click());
            let fill = if clipped { p.danger } else { p.bg_field };
            ui.painter().circle(
                light.center(),
                5.0,
                fill,
                Stroke::new(1.0, p.border_control),
            );
            let tip = if clipped {
                "The input clipped. Click to clear"
            } else {
                "Lights red when the input clips"
            };
            cleared = response.on_hover_text(tip).clicked() && clipped;
        });
        if reading.is_some_and(|m| m.clipped) && self.phase == Phase::Recording {
            ui.label(
                RichText::new("Input clipped. Lower the level for the next take.")
                    .small()
                    .color(p.danger),
            );
        }
        cleared
    }

    /// The elapsed time and the time left to the timeline's end.
    fn time_line(&self, ui: &mut egui::Ui, doc: &Document, playhead: Tick) {
        let p = theme::look(ui.ctx()).palette;
        let end = doc
            .sequence(doc.root())
            .map_or(Tick::ZERO, Sequence::duration);
        let elapsed = self.elapsed().unwrap_or(Tick::ZERO);
        let at = self.take.as_ref().map_or(playhead, |t| {
            t.start.checked_add(elapsed).unwrap_or(t.start)
        });
        let mut text = format!("Take {}", clock(elapsed));
        if at < end {
            let left = end.checked_sub(at).unwrap_or(Tick::ZERO);
            text.push_str(&format!(" · {} left in the timeline", clock(left)));
        }
        ui.label(RichText::new(text).color(p.text_secondary));
    }

    fn buttons(&mut self, ui: &mut egui::Ui, doc: &Document, actions: &mut Vec<PanelAction>) {
        let ready = self.phase == Phase::Ready;
        ui.horizontal(|ui| {
            if self.take.is_some() {
                let stopping = self
                    .take
                    .as_ref()
                    .is_some_and(|t| t.stopping || t.cancelling);
                if button(
                    ui,
                    ButtonKind::Danger,
                    None,
                    "Stop",
                    120.0,
                    Some(36.0),
                    !stopping,
                )
                .on_hover_text("Stop and place the take (Enter)")
                .clicked()
                {
                    actions.push(PanelAction::Stop);
                }
                if button(
                    ui,
                    ButtonKind::Ghost,
                    None,
                    "Cancel",
                    0.0,
                    Some(36.0),
                    !stopping,
                )
                .on_hover_text("Stop without placing the take (Esc)")
                .clicked()
                {
                    actions.push(PanelAction::Cancel);
                }
            } else {
                let r = button(
                    ui,
                    ButtonKind::Primary,
                    None,
                    "● Record",
                    120.0,
                    Some(36.0),
                    ready,
                );
                let r = if ready {
                    r.on_hover_text("Start a take at the playhead (Enter)")
                } else {
                    r.on_disabled_hover_text("No microphone is ready")
                };
                if r.clicked() {
                    actions.push(PanelAction::Record);
                }
                let redo = ready && self.redo_target(doc).is_some();
                if button(
                    ui,
                    ButtonKind::Secondary,
                    None,
                    "Redo last take",
                    0.0,
                    Some(36.0),
                    redo,
                )
                .on_hover_text("Remove the last take and record it again from the same place")
                .clicked()
                {
                    actions.push(PanelAction::RedoLastTake);
                }
            }
        });
    }
}

/// What the microphone is opened with.
fn open_request(prefs: &VoiceoverSettings, project_rate: u32) -> OpenRequest {
    OpenRequest {
        device: prefs.input_device.clone(),
        rate: project_rate,
        channels: channels(prefs),
        gain: GainDb::from_hundredths(prefs.input_level),
        monitor: prefs.monitor,
    }
}

fn channels(prefs: &VoiceoverSettings) -> Channels {
    if prefs.stereo {
        Channels::Stereo
    } else {
        Channels::Mono
    }
}

/// What a take that ended by itself says (AU-027).
fn end_notice(reason: EndReason) -> Option<&'static str> {
    match reason {
        EndReason::User | EndReason::Cancelled | EndReason::RangeEnd => None,
        EndReason::DeviceLost => {
            Some("The microphone was disconnected. The take so far was kept and placed")
        }
        EndReason::DiskLow => Some(
            "The disk is nearly full, so the take stopped. It was kept and placed; free some \
             space before recording again",
        ),
        EndReason::Overrun => {
            Some("The computer fell behind, so the take stopped. What was recorded was placed")
        }
        EndReason::RateChanged => Some(
            "The microphone changed its sample rate, so the take stopped. The take so far was \
             kept and placed",
        ),
    }
}

/// Where a finished take's clip starts: the timeline time at which its
/// first sample (`started_at`) was captured, plus its pre-roll handle. While
/// the timeline played from `from` since `playback`'s instant, that is
/// `from + (started_at − playback) + handle`; without playback the take
/// starts at `start`. `started_at` already allows for the input latency, so
/// only the user's offset preference is added, on the take's sample grid.
pub fn take_position(
    start: Tick,
    handle_in: Tick,
    started_at: Instant,
    playback: Option<(Instant, Tick)>,
    offset_ms: i32,
    rate: SampleRate,
) -> Tick {
    let base = match playback {
        Some((played_at, from)) => {
            let (later, d) = match started_at.checked_duration_since(played_at) {
                Some(d) => (true, d),
                None => (false, played_at.duration_since(started_at)),
            };
            let d = duration_ticks(d);
            let first = if later {
                from.checked_add(d)
            } else {
                from.checked_sub(d)
            }
            .unwrap_or(from);
            first.checked_add(handle_in).unwrap_or(first)
        }
        None => start,
    };
    recorded_start(base, Duration::ZERO, offset_ms, rate)
}

/// The countdown's number: 3, 2, 1 (whole seconds left, rounded up).
fn countdown_number(left: Duration) -> u128 {
    left.as_nanos().div_ceil(1_000_000_000)
}

/// `m:ss.d` (UI edge only).
pub fn clock(t: Tick) -> String {
    let tenths = (t.as_secs_f64().max(0.0) * 10.0).floor() as u64;
    format!("{}:{:02}.{}", tenths / 600, tenths / 10 % 60, tenths % 10)
}

/// A label column and a control.
fn form_row(ui: &mut egui::Ui, label: &str, control: impl FnOnce(&mut egui::Ui)) {
    let p = theme::look(ui.ctx()).palette;
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(104.0, 24.0), Sense::hover());
        ui.painter().text(
            rect.left_center(),
            Align2::LEFT_CENTER,
            label,
            egui::TextStyle::Body.resolve(ui.style()),
            p.text_secondary,
        );
        control(ui);
    });
}

/// Channels, where the take goes, countdown, pre-roll and the toggles.
fn choice_rows(ui: &mut egui::Ui, prefs: &mut VoiceoverSettings) {
    form_row(ui, "Channels", |ui| {
        let items = [Segment::new(None, "Mono"), Segment::new(None, "Stereo")];
        let out = Segmented::new(
            egui::Id::new("recorder-channels"),
            &items,
            usize::from(prefs.stereo),
        )
        .fill_width()
        .show(ui);
        if let Some(i) = out.picked {
            prefs.stereo = i == 1;
        }
    });
    form_row(ui, "Place take on", |ui| {
        let options = [
            SelectOption::new("First free audio track"),
            SelectOption::new("A new audio track"),
        ];
        let out = Select::new(
            egui::Id::new("recorder-place"),
            &options,
            Some(usize::from(prefs.new_track)),
        )
        .fill_width()
        .show(ui);
        if let Some(i) = out.picked {
            prefs.new_track = i == 1;
        }
    });
    form_row(ui, "Countdown", |ui| {
        let items = [
            Segment::new(None, "Off"),
            Segment::new(None, "3 s"),
            Segment::new(None, "5 s"),
        ];
        let current = COUNTDOWNS
            .iter()
            .position(|c| *c == prefs.countdown_secs)
            .unwrap_or(1);
        let out = Segmented::new(egui::Id::new("recorder-countdown"), &items, current)
            .fill_width()
            .show(ui);
        if let Some(c) = out.picked.and_then(|i| COUNTDOWNS.get(i)) {
            prefs.countdown_secs = *c;
        }
    });
    form_row(ui, "Pre-roll", |ui| {
        let labels: Vec<String> = (0..=MAX_PREROLL_SECS)
            .map(|s| {
                if s == 0 {
                    "None".to_owned()
                } else {
                    format!("{s} s")
                }
            })
            .collect();
        let options: Vec<SelectOption<'_>> = labels.iter().map(|l| SelectOption::new(l)).collect();
        let out = Select::new(
            egui::Id::new("recorder-preroll"),
            &options,
            Some(usize::from(prefs.preroll_secs.min(MAX_PREROLL_SECS))),
        )
        .fill_width()
        .show(ui);
        if let Some(i) = out.picked {
            prefs.preroll_secs = u8::try_from(i).unwrap_or(0);
        }
    });
    form_row(ui, "Hear yourself", |ui| {
        toggle(
            ui,
            egui::Id::new("recorder-monitor"),
            &mut prefs.monitor,
            "Hear yourself",
        )
        .on_hover_text("Play the microphone through your speakers or headphones. Use headphones, or the timeline's sound and yours will feed back");
    });
    form_row(ui, "Mute timeline", |ui| {
        toggle(
            ui,
            egui::Id::new("recorder-mute"),
            &mut prefs.mute_timeline,
            "Mute timeline while recording",
        )
        .on_hover_text("Mute the timeline while recording, so it doesn't bleed into the take");
    });
}

/// The Takes list (newest first): this project's recordings with *Show on
/// timeline* and *Remove from project*, then takes on disk that aren't
/// placed, with *Place*.
fn takes_list(
    ui: &mut egui::Ui,
    doc: &Document,
    loose: &[LooseTake],
    actions: &mut Vec<PanelAction>,
) {
    let p = theme::look(ui.ctx()).palette;
    ui.label(RichText::new("Takes").strong());
    let mut recorded: Vec<_> = doc
        .library()
        .items()
        .filter(|m| {
            matches!(
                m.origin,
                MediaOrigin::Generated {
                    kind: GeneratedMediaKind::Voiceover
                }
            )
        })
        .collect();
    recorded.sort_by_key(|m| std::cmp::Reverse(m.id));
    if recorded.is_empty() && loose.is_empty() {
        ui.label(
            RichText::new(
                "No takes yet. Put the playhead where the voice should start and press Record.",
            )
            .color(p.text_tertiary),
        );
        return;
    }
    let used = |media: MediaId| {
        doc.sequence(doc.root()).is_some_and(|seq| {
            seq.tracks().any(|(_, t)| {
                t.clips().iter().any(
                    |c| matches!(c.content, ClipContent::Media { asset, .. } if asset == media),
                )
            })
        })
    };
    egui::ScrollArea::vertical()
        .id_salt("recorder-takes")
        .max_height(150.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for item in &loose.iter().rev().collect::<Vec<_>>() {
                ui.horizontal(|ui| {
                    ui.label(file_name(&item.path));
                    let tag = if item.recovered {
                        "Recovered"
                    } else {
                        "Not placed"
                    };
                    ui.label(RichText::new(tag).small().color(p.warning));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Place").clicked() {
                            actions.push(PanelAction::Place(item.path.clone()));
                        }
                    });
                });
            }
            for item in recorded {
                ui.horizontal(|ui| {
                    let name = crate::ui::timeline::media_name(item);
                    ui.label(name);
                    ui.label(
                        RichText::new(clock(item.info.duration))
                            .small()
                            .color(p.text_tertiary),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .small_button("Remove from project")
                            .on_hover_text("The file stays on disk")
                            .clicked()
                        {
                            actions.push(PanelAction::Remove(item.id));
                        }
                        if ui
                            .add_enabled(
                                used(item.id),
                                egui::Button::new("Show on timeline").small(),
                            )
                            .clicked()
                        {
                            actions.push(PanelAction::ShowOnTimeline(item.id));
                        }
                    });
                });
            }
        });
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)] // test code

    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use freecut_audio::record::SpaceProbe;

    use super::*;
    use crate::recorder::tests::tone;

    fn s(n: i64) -> Tick {
        Tick::from_secs(n).unwrap()
    }

    fn prefs() -> VoiceoverSettings {
        VoiceoverSettings {
            countdown_secs: 0,
            ..VoiceoverSettings::default()
        }
    }

    fn panel_with(backend: Backend) -> RecorderPanel {
        RecorderPanel::new(backend, Arc::new(|| {}))
    }

    fn panel() -> RecorderPanel {
        panel_with(Backend::synthetic(tone(), 48_000))
    }

    /// Reads the recorder until `done` holds; returns every event.
    fn pump(
        panel: &mut RecorderPanel,
        mut done: impl FnMut(&RecorderPanel, &[PanelEvent]) -> bool,
    ) -> Vec<PanelEvent> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut all = Vec::new();
        loop {
            all.extend(panel.update(0));
            if done(panel, &all) {
                return all;
            }
            assert!(Instant::now() < deadline, "timed out; got {all:?}");
            std::thread::sleep(Duration::from_millis(3));
        }
    }

    fn ready(panel: &mut RecorderPanel, prefs: &VoiceoverSettings) {
        panel.show_panel(prefs, 48_000);
        pump(panel, |p, _| p.phase() == &Phase::Ready);
        assert!(panel.levels().is_some(), "the meter is live when Ready");
    }

    fn placement(events: &[PanelEvent]) -> Option<&Placement> {
        events.iter().find_map(|e| match e {
            PanelEvent::Place(p) => Some(p),
            _ => None,
        })
    }

    fn recorded_for(panel: &mut RecorderPanel, at_least: Tick) {
        pump(panel, |p, _| p.elapsed().is_some_and(|e| e >= at_least));
    }

    #[test]
    fn a_take_locks_edits_and_is_placed_at_its_start() {
        let dir = tempfile::tempdir().unwrap();
        let mut panel = panel();
        let prefs = prefs();
        ready(&mut panel, &prefs);
        panel
            .begin_take(
                s(2),
                &prefs,
                vec![dir.path().into()],
                None,
                SystemTime::now(),
            )
            .unwrap();
        assert!(panel.locks_edits(), "refused from Record on");
        assert!(panel.running());
        let events = pump(&mut panel, |_, e| {
            e.iter().any(|e| matches!(e, PanelEvent::StartPlayback(_)))
        });
        assert!(
            events
                .iter()
                .any(|e| matches!(e, PanelEvent::StartPlayback(t) if *t == s(2))),
            "the timeline plays from the start position"
        );
        recorded_for(&mut panel, Tick::from_raw(s(1).raw() / 2));
        assert_eq!(panel.phase(), &Phase::Recording);
        let live = {
            let project = crate::session::Project::new(
                freecut_timeline::model::ProjectSettings::new_default().unwrap(),
            )
            .unwrap();
            let doc = project.doc();
            panel.live_take(doc.sequence(doc.root()).unwrap())
        };
        let live = live.expect("the red clip grows");
        assert_eq!(live.start, s(2));
        assert!(!live.peaks.is_empty(), "with a live waveform");
        assert!(panel.button_text().is_some_and(|t| t.starts_with("0:00.")));

        panel.stop();
        assert!(!panel.locks_edits(), "edits come back once Stop is pressed");
        let events = pump(&mut panel, |_, e| placement(e).is_some());
        let place = placement(&events).unwrap();
        assert_eq!(
            place.at,
            s(2),
            "no playback ran: the take starts at the start"
        );
        assert_eq!(place.handle_in, Tick::ZERO);
        assert_eq!(place.target, RecordTarget::FirstFree);
        assert_eq!(place.path.parent(), Some(dir.path()));
        assert!(events.iter().any(|e| matches!(e, PanelEvent::StopPlayback)));
        assert!(panel.take().is_none());
    }

    #[test]
    fn the_pre_roll_plays_before_the_start_and_stays_as_a_handle() {
        let dir = tempfile::tempdir().unwrap();
        let mut panel = panel();
        let prefs = VoiceoverSettings {
            preroll_secs: 2,
            new_track: true,
            ..prefs()
        };
        ready(&mut panel, &prefs);
        // Only 1 s before the start: the pre-roll stops at 0.
        panel
            .begin_take(
                s(1),
                &prefs,
                vec![dir.path().into()],
                None,
                SystemTime::now(),
            )
            .unwrap();
        let events = pump(&mut panel, |_, e| {
            e.iter().any(|e| matches!(e, PanelEvent::StartPlayback(_)))
        });
        assert!(
            events
                .iter()
                .any(|e| matches!(e, PanelEvent::StartPlayback(t) if *t == Tick::ZERO))
        );
        pump(&mut panel, |p, _| p.phase() == &Phase::Recording);
        recorded_for(&mut panel, Tick::from_raw(s(1).raw() / 2));
        panel.stop();
        let events = pump(&mut panel, |_, e| placement(e).is_some());
        let place = placement(&events).unwrap();
        assert_eq!(place.handle_in, s(1), "a 1 s handle");
        assert_eq!(place.target, RecordTarget::NewTrack);
    }

    #[test]
    fn a_cancelled_take_is_kept_in_the_takes_list_and_not_placed() {
        let dir = tempfile::tempdir().unwrap();
        let mut panel = panel();
        let prefs = prefs();
        ready(&mut panel, &prefs);
        panel
            .begin_take(
                s(3),
                &prefs,
                vec![dir.path().into()],
                None,
                SystemTime::now(),
            )
            .unwrap();
        pump(&mut panel, |p, _| p.phase() == &Phase::Recording);
        recorded_for(&mut panel, Tick::from_raw(s(1).raw() / 2));
        panel.cancel();
        assert!(!panel.locks_edits());
        let events = pump(&mut panel, |_, e| {
            e.iter().any(|e| matches!(e, PanelEvent::Kept(_)))
        });
        assert!(placement(&events).is_none(), "not placed");
        let kept = &panel.loose_takes()[0];
        assert_eq!(
            kept.at,
            Some(s(3)),
            "Place puts it where it was meant to go"
        );
        assert!(kept.path.exists());
        assert!(!kept.recovered);
    }

    #[test]
    fn a_countdown_cancelled_records_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut panel = panel();
        let prefs = VoiceoverSettings {
            countdown_secs: 3,
            ..prefs()
        };
        ready(&mut panel, &prefs);
        panel
            .begin_take(
                s(0),
                &prefs,
                vec![dir.path().into()],
                None,
                SystemTime::now(),
            )
            .unwrap();
        pump(&mut panel, |p, _| p.phase() == &Phase::CountingDown);
        assert!(panel.locks_edits(), "edits wait during the countdown too");
        assert_eq!(panel.button_text().as_deref(), Some("3"));
        panel.cancel();
        // Take completion and the writer's state notification can arrive in
        // separate polls. Wait for both parts of the cancellation to settle.
        let events = pump(&mut panel, |p, _| {
            p.take().is_none() && p.phase() == &Phase::Ready
        });
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, PanelEvent::Place(_) | PanelEvent::Kept(_)))
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        assert_eq!(panel.phase(), &Phase::Ready);
    }

    #[test]
    fn an_accidental_click_is_removed_as_too_short() {
        let dir = tempfile::tempdir().unwrap();
        let mut panel = panel();
        let prefs = prefs();
        ready(&mut panel, &prefs);
        panel
            .begin_take(
                s(0),
                &prefs,
                vec![dir.path().into()],
                None,
                SystemTime::now(),
            )
            .unwrap();
        pump(&mut panel, |p, _| p.phase() == &Phase::Recording);
        panel.stop();
        let events = pump(&mut panel, |_, e| {
            e.iter().any(|e| matches!(e, PanelEvent::Discard(_)))
        });
        assert!(placement(&events).is_none());
        assert!(
            events
                .iter()
                .any(|e| matches!(e, PanelEvent::Notice(t) if t == "Take too short"))
        );
    }

    #[test]
    fn a_nearly_full_disk_stops_the_take_and_places_it_with_a_notice() {
        let dir = tempfile::tempdir().unwrap();
        let checks = Arc::new(AtomicU32::new(0));
        let seen = Arc::clone(&checks);
        // Plenty of room for the first checks (4 096 frames each), then 10 MB.
        let probe = SpaceProbe::new(move |_| {
            let n = seen.fetch_add(1, Ordering::SeqCst);
            Some(if n < 8 { 1 << 40 } else { 10_000_000 })
        });
        let mut panel = panel_with(Backend::synthetic(tone(), 48_000).with_space(probe));
        let prefs = prefs();
        ready(&mut panel, &prefs);
        panel
            .begin_take(
                s(0),
                &prefs,
                vec![dir.path().into()],
                None,
                SystemTime::now(),
            )
            .unwrap();
        let events = pump(&mut panel, |_, e| placement(e).is_some());
        assert!(checks.load(Ordering::SeqCst) > 8);
        assert!(events.iter().any(
            |e| matches!(e, PanelEvent::Notice(t) if t.starts_with("The disk is nearly full"))
        ));
    }

    #[test]
    fn a_refused_microphone_explains_itself() {
        let mut panel = panel_with(Backend::new(
            |_, _, _| Err(RecordError::Denied("privacy".to_owned())),
            || Ok(Vec::new()),
        ));
        let prefs = prefs();
        panel.show_panel(&prefs, 48_000);
        pump(&mut panel, |p, _| matches!(p.phase(), Phase::Waiting(_)));
        assert_eq!(panel.phase(), &Phase::Waiting(Waiting::Denied));
        let err = panel
            .begin_take(s(0), &prefs, Vec::new(), None, SystemTime::now())
            .unwrap_err();
        assert!(err.contains("isn't allowed"), "{err}");
        assert_eq!(
            waiting_for(&RecordError::Busy("Zoom".into())),
            Waiting::Busy("Zoom".into())
        );
        assert!(Waiting::NoDevice.text().starts_with("No microphone found"));
    }

    #[test]
    fn closing_during_a_take_asks_first() {
        let dir = tempfile::tempdir().unwrap();
        let mut panel = panel();
        let prefs = prefs();
        ready(&mut panel, &prefs);
        panel
            .begin_take(
                s(0),
                &prefs,
                vec![dir.path().into()],
                None,
                SystemTime::now(),
            )
            .unwrap();
        panel.request_close();
        assert!(panel.open, "Keep, Discard or Go back first");
        assert!(panel.confirm_close);
        panel.cancel();
        pump(&mut panel, |p, _| p.take().is_none());
        panel.request_close();
        assert!(!panel.open);
        assert_eq!(panel.phase(), &Phase::Closed);
    }

    #[test]
    fn the_take_lines_up_with_the_picture_that_was_playing() {
        let rate = SampleRate::new(48_000).unwrap();
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        // Playback from 1 s started 10 ms before the first sample came in:
        // that sample is 1.01 s of the timeline; after a 0.5 s handle the
        // clip starts at 1.51 s.
        let handle = Tick::from_raw(s(1).raw() / 2);
        let at = take_position(s(9), handle, t0 + ms(10), Some((t0, s(1))), 0, rate);
        let expected = recorded_start(
            Tick::from_raw(s(1).raw() + s(1).raw() / 100 + handle.raw()),
            Duration::ZERO,
            0,
            rate,
        );
        assert_eq!(at, expected);
        // Capture before playback: earlier.
        let at = take_position(s(9), Tick::ZERO, t0, Some((t0 + ms(20), s(1))), 0, rate);
        assert_eq!(at, Tick::from_raw(s(1).raw() - s(1).raw() / 50));
        // No playback: the start position, moved by the user's offset.
        let at = take_position(s(4), Tick::ZERO, t0, None, 250, rate);
        assert_eq!(at, Tick::from_raw(s(4).raw() + s(1).raw() / 4));
    }

    #[test]
    fn the_clock_reads_minutes_seconds_and_tenths() {
        assert_eq!(clock(Tick::ZERO), "0:00.0");
        assert_eq!(
            clock(Tick::from_raw(s(75).raw() + s(1).raw() / 2)),
            "1:15.5"
        );
    }
}
