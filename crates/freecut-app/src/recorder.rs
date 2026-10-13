//! The voiceover recorder's worker thread (AU-027).
//!
//! `freecut_audio::record::Recorder` runs capture and writing on its own
//! threads, but opening a microphone, listing inputs, creating the
//! Recordings folder and cancelling a take all block. A [`RecorderHost`]
//! owns the recorder on one more thread and talks to the UI only through
//! messages: the UI sends commands and reads [`HostEvent`]s (state changes,
//! finished takes and, about 30 times a second, the meter), so the UI thread
//! never blocks on the microphone or the disk.
//!
//! Where the frames come from is a [`Backend`]: the system's microphones
//! (the `record` feature), or a synthetic source in tests, so nothing here
//! needs a real device to be tested.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossbeam_channel::{Receiver, Sender, never, select, unbounded};
use freecut_audio::record::{
    CaptureSource, Channels, MeterReading, MonitorFeed, RecordConfig, RecordError, Recorder,
    RecorderEvent, RecorderState, SpaceProbe, TakeInfo,
};
use freecut_project::recordings::{TakeStamp, new_take_path};
use freecut_timeline::model::GainDb;
use freecut_timeline::time::{SampleRate, TICKS_PER_SECOND, Tick};

/// How often the meter goes to the UI.
const LEVELS_EVERY: Duration = Duration::from_millis(33);

/// An input the panel can offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputChoice {
    /// What to open it with.
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

/// Opens a capture source: `(input id or the default, preferred rate,
/// channels)`.
type OpenFn = dyn Fn(Option<&str>, u32, Channels) -> Result<Box<dyn CaptureSource>, RecordError>
    + Send
    + Sync;
/// Lists the inputs.
type ListFn = dyn Fn() -> Result<Vec<InputChoice>, RecordError> + Send + Sync;

/// Where the recorder's frames come from.
#[derive(Clone)]
pub struct Backend {
    open: Arc<OpenFn>,
    list: Arc<ListFn>,
    /// The free-space probe takes use (tests script a disk).
    space: Option<SpaceProbe>,
}

impl std::fmt::Debug for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Backend")
    }
}

impl Backend {
    pub fn new(
        open: impl Fn(Option<&str>, u32, Channels) -> Result<Box<dyn CaptureSource>, RecordError>
        + Send
        + Sync
        + 'static,
        list: impl Fn() -> Result<Vec<InputChoice>, RecordError> + Send + Sync + 'static,
    ) -> Backend {
        Backend {
            open: Arc::new(open),
            list: Arc::new(list),
            space: None,
        }
    }

    /// The system's microphones.
    #[cfg(feature = "record")]
    pub fn system() -> Backend {
        use freecut_audio::record::{CpalInput, list_inputs};
        Backend::new(
            |id, rate, channels| {
                CpalInput::open(id, rate, channels).map(|c| Box::new(c) as Box<dyn CaptureSource>)
            },
            || {
                list_inputs().map(|list| {
                    list.into_iter()
                        .map(|d| InputChoice {
                            id: d.id,
                            name: d.name,
                            is_default: d.is_default,
                        })
                        .collect()
                })
            },
        )
    }

    /// A build without the `record` feature has no microphones.
    #[cfg(not(feature = "record"))]
    pub fn system() -> Backend {
        Backend::new(|_, _, _| Err(RecordError::NoDevice), || Ok(Vec::new()))
    }

    /// Whether this build can record from a microphone at all.
    pub fn microphones_built() -> bool {
        cfg!(feature = "record")
    }

    /// A synthetic microphone playing `signal` in real time at `rate`
    /// (tests and the headless window).
    #[cfg(test)]
    pub fn synthetic(signal: freecut_audio::record::Signal, rate: u32) -> Backend {
        use freecut_audio::record::SyntheticInput;
        Backend::new(
            move |_, _, channels| {
                Ok(
                    Box::new(SyntheticInput::new(signal, rate, channels.count()))
                        as Box<dyn CaptureSource>,
                )
            },
            || {
                Ok(vec![InputChoice {
                    id: "synthetic".to_owned(),
                    name: "Test tone".to_owned(),
                    is_default: true,
                }])
            },
        )
    }

    /// Takes check free space with `probe`.
    #[cfg(test)]
    pub fn with_space(mut self, probe: SpaceProbe) -> Backend {
        self.space = Some(probe);
        self
    }
}

/// Opening the microphone.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenRequest {
    /// The input's id; `None`: the system default.
    pub device: Option<String>,
    /// The project's rate, used when the device offers it.
    pub rate: u32,
    pub channels: Channels,
    pub gain: GainDb,
    pub monitor: bool,
}

/// Starting a take.
#[derive(Debug, Clone, PartialEq)]
pub struct TakeRequest {
    /// Where the take goes, tried in order: the project's Recordings
    /// folder, then the app-data fallback (PM-041).
    pub folders: Vec<PathBuf>,
    pub stamp: TakeStamp,
    pub channels: Channels,
    pub gain: GainDb,
    pub countdown: Duration,
    pub preroll: Duration,
    pub monitor: bool,
}

#[derive(Debug)]
enum Command {
    List,
    Open(OpenRequest),
    Close,
    Take(TakeRequest),
    Stop,
    Cancel,
    Gain(GainDb),
    Monitor(bool),
    ResetClip,
}

/// What the meter shows, sent while a microphone is open.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Levels {
    pub meter: MeterReading,
    pub state: RecorderState,
    /// Frames of the current take so far, pre-roll included.
    pub captured: u64,
    pub countdown_left: Option<Duration>,
}

/// What the host reports.
#[derive(Debug)]
pub enum HostEvent {
    Devices(Result<Vec<InputChoice>, RecordError>),
    /// The microphone is open and the meter live (Ready).
    Opened {
        rate: SampleRate,
        latency: Duration,
        /// "Hear yourself"'s feed for the output engine.
        monitor: Option<MonitorFeed>,
    },
    OpenFailed(RecordError),
    /// A take was started; it is written to this file.
    Armed(PathBuf),
    /// A take couldn't start.
    TakeRefused(String),
    State(RecorderState),
    Finished(TakeInfo),
    Failed(RecordError),
    LowDisk(u64),
    Levels(Levels),
}

/// Wakes the UI when something arrives.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// Owns a recorder on its own thread. Dropping the host lets the thread
/// finish by itself (a take still running is cancelled and kept);
/// [`RecorderHost::shutdown_and_wait`] waits for that.
pub struct RecorderHost {
    commands: Option<Sender<Command>>,
    events: Receiver<HostEvent>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for RecorderHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecorderHost").finish_non_exhaustive()
    }
}

impl RecorderHost {
    /// Starts the worker. Nothing is opened until [`RecorderHost::open`].
    pub fn spawn(backend: Backend, wake: Wake) -> std::io::Result<RecorderHost> {
        let (commands, command_rx) = unbounded();
        let (event_tx, events) = unbounded();
        let thread = std::thread::Builder::new()
            .name("freecut-recorder".to_owned())
            .spawn(move || run(&backend, &command_rx, &event_tx, &wake))?;
        Ok(RecorderHost {
            commands: Some(commands),
            events,
            thread: Some(thread),
        })
    }

    fn send(&self, command: Command) {
        if let Some(tx) = &self.commands {
            let _ = tx.send(command);
        }
    }

    /// Lists the inputs ([`HostEvent::Devices`]).
    pub fn list(&self) {
        self.send(Command::List);
    }

    /// Opens (or reopens) the microphone ([`HostEvent::Opened`] or
    /// [`HostEvent::OpenFailed`]).
    pub fn open(&self, request: OpenRequest) {
        self.send(Command::Open(request));
    }

    /// Closes the microphone.
    pub fn close(&self) {
        self.send(Command::Close);
    }

    /// Starts a take ([`HostEvent::Armed`] or [`HostEvent::TakeRefused`]).
    pub fn take(&self, request: TakeRequest) {
        self.send(Command::Take(request));
    }

    /// Stops the take; it arrives as [`HostEvent::Finished`]. A countdown
    /// ends with nothing recorded.
    pub fn stop(&self) {
        self.send(Command::Stop);
    }

    /// Cancels: a countdown or pre-roll leaves nothing; a take that is
    /// recording is finished and kept, marked cancelled.
    pub fn cancel(&self) {
        self.send(Command::Cancel);
    }

    pub fn set_gain(&self, gain: GainDb) {
        self.send(Command::Gain(gain));
    }

    pub fn set_monitor(&self, on: bool) {
        self.send(Command::Monitor(on));
    }

    pub fn reset_clip(&self) {
        self.send(Command::ResetClip);
    }

    /// Everything that arrived, oldest first.
    pub fn poll(&self) -> Vec<HostEvent> {
        self.events.try_iter().collect()
    }

    /// Ends the worker and waits up to `timeout` for it (a running take is
    /// cancelled and its file finished first). For quitting.
    pub fn shutdown_and_wait(mut self, timeout: Duration) {
        self.commands = None;
        let Some(thread) = self.thread.take() else {
            return;
        };
        let deadline = Instant::now() + timeout;
        while !thread.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        if thread.is_finished() {
            let _ = thread.join();
        }
    }
}

impl Drop for RecorderHost {
    fn drop(&mut self) {
        // Closing the channel ends the worker's loop; it is not joined
        // here, so the UI never waits for a file to be finished.
        self.commands = None;
    }
}

/// The worker's loop.
fn run(backend: &Backend, commands: &Receiver<Command>, events: &Sender<HostEvent>, wake: &Wake) {
    let send = |e: HostEvent| {
        let _ = events.send(e);
        wake();
    };
    let mut recorder: Option<Recorder> = None;
    let mut rec_events: Receiver<RecorderEvent> = never();
    let mut last: Option<Levels> = None;
    loop {
        select! {
            recv(commands) -> command => match command {
                Ok(command) => command_in(
                    backend,
                    command,
                    &mut recorder,
                    &mut rec_events,
                    &send,
                ),
                Err(_) => break,
            },
            recv(rec_events) -> event => match event {
                Ok(event) => send(match event {
                    RecorderEvent::State(s) => HostEvent::State(s),
                    RecorderEvent::Finished(info) => HostEvent::Finished(info),
                    RecorderEvent::Failed(e) => HostEvent::Failed(e),
                    RecorderEvent::LowDisk { free_bytes } => HostEvent::LowDisk(free_bytes),
                }),
                Err(_) => rec_events = never(),
            },
            default(LEVELS_EVERY) => {}
        }
        if let Some(r) = &recorder {
            let levels = Levels {
                meter: r.meter(),
                state: r.state(),
                captured: r.captured_frames(),
                countdown_left: r.countdown_left(),
            };
            if last != Some(levels) {
                last = Some(levels);
                send(HostEvent::Levels(levels));
            }
        } else {
            last = None;
        }
    }
    // The recorder cancels a running take (kept past its pre-roll) and
    // finishes the file as it drops.
    drop(recorder);
}

fn command_in(
    backend: &Backend,
    command: Command,
    recorder: &mut Option<Recorder>,
    rec_events: &mut Receiver<RecorderEvent>,
    send: &impl Fn(HostEvent),
) {
    match command {
        Command::List => send(HostEvent::Devices((backend.list)())),
        Command::Open(request) => {
            // The old recorder (and its device) goes first.
            *recorder = None;
            *rec_events = never();
            match open(backend, &request) {
                Ok(mut r) => {
                    *rec_events = r.events();
                    send(HostEvent::Opened {
                        rate: r.rate(),
                        latency: r.input_latency(),
                        monitor: r.monitor_feed(),
                    });
                    *recorder = Some(r);
                }
                Err(e) => send(HostEvent::OpenFailed(e)),
            }
        }
        Command::Close => {
            *recorder = None;
            *rec_events = never();
        }
        Command::Take(request) => match recorder.as_mut() {
            Some(r) => match start_take(r, &request) {
                Ok(path) => send(HostEvent::Armed(path)),
                Err(why) => send(HostEvent::TakeRefused(why)),
            },
            None => send(HostEvent::TakeRefused("No microphone is open".to_owned())),
        },
        Command::Stop => {
            if let Some(r) = recorder.as_ref() {
                r.request_stop();
            }
        }
        Command::Cancel => {
            if let Some(r) = recorder.as_mut()
                && let Err(e) = r.cancel()
            {
                send(HostEvent::Failed(e));
            }
        }
        Command::Gain(gain) => {
            if let Some(r) = recorder.as_mut() {
                r.set_gain(gain);
            }
        }
        Command::Monitor(on) => {
            if let Some(r) = recorder.as_mut() {
                r.set_monitor(on);
            }
        }
        Command::ResetClip => {
            if let Some(r) = recorder.as_ref() {
                r.reset_clip();
            }
        }
    }
}

fn open(backend: &Backend, request: &OpenRequest) -> Result<Recorder, RecordError> {
    let source = (backend.open)(request.device.as_deref(), request.rate, request.channels)?;
    // The real path is set for each take.
    let mut config = RecordConfig::new(PathBuf::new());
    config.channels = request.channels;
    config.gain = request.gain;
    config.monitor = request.monitor;
    if let Some(space) = &backend.space {
        config.free_space = space.clone();
    }
    Recorder::open(config, source)
}

/// Sets the next take up and starts it (after its countdown). Returns the
/// take's file, or why it can't start.
fn start_take(r: &mut Recorder, request: &TakeRequest) -> Result<PathBuf, String> {
    if r.state() != RecorderState::Ready {
        return Err("The microphone isn't ready".to_owned());
    }
    let folder = usable_folder(&request.folders)?;
    let path = new_take_path(&folder, request.stamp, Path::exists)
        .ok_or("Too many takes in one second; try again")?;
    let mut config = r.config().clone();
    config.path.clone_from(&path);
    config.channels = request.channels;
    config.gain = request.gain;
    config.countdown = request.countdown;
    config.preroll_frames = frames_in(request.preroll, r.rate());
    config.stop_after_frames = None;
    config.monitor = request.monitor;
    r.update(config)
        .map_err(|e| format!("The take couldn't start: {e}"))?;
    let started = if request.countdown.is_zero() {
        r.start()
    } else {
        r.arm()
    };
    started.map_err(|e| format!("The take couldn't start: {e}"))?;
    Ok(path)
}

/// The first folder that exists or can be created.
fn usable_folder(folders: &[PathBuf]) -> Result<PathBuf, String> {
    let mut last = None;
    for folder in folders {
        match std::fs::create_dir_all(folder) {
            Ok(()) => return Ok(folder.clone()),
            Err(e) => last = Some(e),
        }
    }
    Err(match last {
        Some(e) => format!("FreeCut can't create the Recordings folder: {e}"),
        None => "There is nowhere to save recordings: save the project first".to_owned(),
    })
}

/// Frames in `d` at `rate`, rounded down.
pub fn frames_in(d: Duration, rate: SampleRate) -> u64 {
    let frames = d.as_nanos() * u128::from(rate.hz()) / 1_000_000_000;
    u64::try_from(frames).unwrap_or(u64::MAX)
}

/// `frames` samples at `rate`, as timeline time.
pub fn frames_to_ticks(frames: u64, rate: SampleRate) -> Tick {
    let ticks = i128::from(frames) * i128::from(rate.sample_ticks().raw());
    Tick::from_raw(i64::try_from(ticks).unwrap_or(i64::MAX))
}

/// A duration as timeline time, rounded down.
pub fn duration_ticks(d: Duration) -> Tick {
    let ticks = d.as_nanos() * TICKS_PER_SECOND.unsigned_abs() as u128 / 1_000_000_000;
    Tick::from_raw(i64::try_from(ticks).unwrap_or(i64::MAX))
}

/// The date and time of `at` for a take's name. The workspace has no time
/// zone database, so this is UTC.
pub fn take_stamp(at: SystemTime) -> TakeStamp {
    let secs = at.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rest = secs % 86_400;
    let (y, m, d) = crate::settings::civil_from_days(days);
    // Every field is in range by construction (hours < 24, minutes < 60).
    TakeStamp {
        year: u16::try_from(y).unwrap_or(1970),
        month: u8::try_from(m).unwrap_or(1),
        day: u8::try_from(d).unwrap_or(1),
        hour: u8::try_from(rest / 3_600).unwrap_or(0),
        minute: u8::try_from(rest / 60 % 60).unwrap_or(0),
        second: u8::try_from(rest % 60).unwrap_or(0),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)] // test code

    use freecut_audio::record::{EndReason, Signal};

    use super::*;

    pub(crate) fn tone() -> Signal {
        Signal::Sine {
            hz: 440.0,
            amp: 0.5,
        }
    }

    fn host(backend: Backend) -> RecorderHost {
        RecorderHost::spawn(backend, Arc::new(|| {})).unwrap()
    }

    /// Waits up to 10 s for an event `pick` accepts.
    pub(crate) fn wait_for<T>(
        host: &RecorderHost,
        mut pick: impl FnMut(HostEvent) -> Option<T>,
    ) -> T {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            assert!(Instant::now() < deadline, "the event never came");
            for e in host.poll() {
                if let Some(t) = pick(e) {
                    return t;
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn open_request() -> OpenRequest {
        OpenRequest {
            device: None,
            rate: 48_000,
            channels: Channels::Mono,
            gain: GainDb::ZERO,
            monitor: false,
        }
    }

    fn take_request(folder: &Path) -> TakeRequest {
        TakeRequest {
            folders: vec![folder.to_owned()],
            stamp: take_stamp(UNIX_EPOCH + Duration::from_secs(1_791_815_525)),
            channels: Channels::Mono,
            gain: GainDb::ZERO,
            countdown: Duration::ZERO,
            preroll: Duration::ZERO,
            monitor: false,
        }
    }

    #[test]
    fn the_system_backend_is_built_without_opening_anything() {
        // Building it only stores how to open inputs.
        let _ = Backend::system();
        assert_eq!(Backend::microphones_built(), cfg!(feature = "record"));
    }

    #[test]
    fn take_names_carry_the_date_and_time() {
        let stamp = take_stamp(UNIX_EPOCH + Duration::from_secs(1_791_815_525));
        assert_eq!(
            freecut_project::recordings::take_file_name(stamp, 1),
            "Voiceover 2026-10-12 14.32.05.wav"
        );
    }

    #[test]
    fn the_marker_names_agree_with_the_writer() {
        assert_eq!(
            freecut_project::recordings::MARKER_SUFFIX,
            freecut_audio::record::wav::MARKER_SUFFIX
        );
    }

    #[test]
    fn opens_meters_and_records_a_take_off_the_ui_thread() {
        let dir = tempfile::tempdir().unwrap();
        let host = host(Backend::synthetic(tone(), 48_000));
        host.list();
        let devices = wait_for(&host, |e| match e {
            HostEvent::Devices(d) => Some(d),
            _ => None,
        });
        assert_eq!(devices.unwrap()[0].name, "Test tone");
        host.open(open_request());
        let rate = wait_for(&host, |e| match e {
            HostEvent::Opened { rate, .. } => Some(rate),
            _ => None,
        });
        assert_eq!(rate.hz(), 48_000);
        // The meter is live before recording.
        let level = wait_for(&host, |e| match e {
            HostEvent::Levels(l) if l.meter.peak_db > -20.0 => Some(l),
            _ => None,
        });
        assert_eq!(level.state, RecorderState::Ready);

        host.take(take_request(dir.path()));
        let path = wait_for(&host, |e| match e {
            HostEvent::Armed(p) => Some(p),
            HostEvent::TakeRefused(why) => panic!("{why}"),
            _ => None,
        });
        assert_eq!(path.parent(), Some(dir.path()));
        wait_for(&host, |e| match e {
            HostEvent::Levels(l) if l.captured > 24_000 => Some(()),
            _ => None,
        });
        host.stop();
        let info = wait_for(&host, |e| match e {
            HostEvent::Finished(info) => Some(info),
            _ => None,
        });
        assert_eq!(info.path, path);
        assert_eq!(info.ended_by, EndReason::User);
        assert!(info.frames >= 24_000);
        let wav = freecut_audio::record::wav::read_info(&path).unwrap();
        assert_eq!(wav.rate, 48_000);
        assert!(!freecut_project::recordings::marker_path(&path).exists());
    }

    #[test]
    fn a_cancelled_take_is_finished_and_kept() {
        let dir = tempfile::tempdir().unwrap();
        let host = host(Backend::synthetic(tone(), 48_000));
        host.open(open_request());
        wait_for(&host, |e| {
            matches!(e, HostEvent::Opened { .. }).then_some(())
        });
        host.take(take_request(dir.path()));
        let path = wait_for(&host, |e| match e {
            HostEvent::Armed(p) => Some(p),
            _ => None,
        });
        wait_for(&host, |e| match e {
            HostEvent::Levels(l) if l.captured > 4_800 => Some(()),
            _ => None,
        });
        host.cancel();
        let info = wait_for(&host, |e| match e {
            HostEvent::Finished(info) => Some(info),
            _ => None,
        });
        assert_eq!(info.ended_by, EndReason::Cancelled);
        assert!(path.exists(), "FreeCut never deletes a take without asking");
    }

    #[test]
    fn a_countdown_cancelled_records_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let host = host(Backend::synthetic(tone(), 48_000));
        host.open(open_request());
        wait_for(&host, |e| {
            matches!(e, HostEvent::Opened { .. }).then_some(())
        });
        let mut request = take_request(dir.path());
        request.countdown = Duration::from_secs(3);
        host.take(request);
        wait_for(&host, |e| match e {
            HostEvent::State(RecorderState::CountingDown) => Some(()),
            HostEvent::Levels(l) if l.state == RecorderState::CountingDown => Some(()),
            _ => None,
        });
        host.cancel();
        wait_for(&host, |e| match e {
            HostEvent::Levels(l) if l.state == RecorderState::Ready => Some(()),
            HostEvent::Finished(_) => panic!("nothing was recorded"),
            _ => None,
        });
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn takes_fall_back_to_the_next_folder_and_say_when_none_works() {
        let dir = tempfile::tempdir().unwrap();
        // A file where the Recordings folder should be.
        let blocked = dir.path().join("blocked");
        std::fs::write(&blocked, b"x").unwrap();
        let fallback = dir.path().join("fallback");
        assert_eq!(
            usable_folder(&[blocked.join("Recordings"), fallback.clone()]).unwrap(),
            fallback
        );
        let err = usable_folder(&[blocked.join("Recordings")]).unwrap_err();
        assert!(err.contains("Recordings folder"), "{err}");
        assert!(usable_folder(&[]).unwrap_err().contains("save the project"));
    }

    #[test]
    fn a_microphone_that_fails_to_open_is_reported() {
        let host = host(Backend::new(
            |_, _, _| {
                Err(RecordError::Denied(
                    "blocked in privacy settings".to_owned(),
                ))
            },
            || Ok(Vec::new()),
        ));
        host.open(open_request());
        let e = wait_for(&host, |e| match e {
            HostEvent::OpenFailed(e) => Some(e),
            _ => None,
        });
        assert!(matches!(e, RecordError::Denied(_)));
        host.take(take_request(Path::new("/nowhere")));
        let why = wait_for(&host, |e| match e {
            HostEvent::TakeRefused(why) => Some(why),
            _ => None,
        });
        assert_eq!(why, "No microphone is open");
    }

    #[test]
    fn time_conversions_are_exact() {
        let rate = SampleRate::new(48_000).unwrap();
        assert_eq!(frames_in(Duration::from_millis(500), rate), 24_000);
        assert_eq!(frames_to_ticks(48_000, rate), Tick::from_secs(1).unwrap());
        assert_eq!(
            duration_ticks(Duration::from_secs(2)),
            Tick::from_secs(2).unwrap()
        );
    }
}
