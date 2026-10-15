//! The export job (EX-022): render, mix, encode, verify, place the file.
//!
//! [`ExportJob::start`] takes an [`ExportRequest`] — an immutable `Arc`
//! snapshot of the document, the resolved media paths and the settings —
//! and runs it on its own threads. The job never reads the live document,
//! so editing continues while it runs.
//!
//! ```text
//!  job thread ─ prepare: plan, source stamps, claim "<final>.part" (create_new),
//!  │            open the EncodeSession
//!  ├─ render thread: ExportRenderer (CPU compositor, C44) ──frames (bounded 4)──┐
//!  ├─ audio thread:  OfflineMix (freecut-audio) ──────────blocks (bounded 16)──┤
//!  │                                                                           ▼
//!  └─ mux loop (job thread, owns the EncodeSession): pulls from whichever stream
//!     is behind in time (`video_due_before_audio`), then finish → verify_output
//!     → fsync → rename onto the final name
//! ```
//!
//! - **Progress** goes through a lock-free [`ProgressCell`] the UI polls.
//! - **Cancel** sets a [`CancelToken`]; the mux loop checks it at least every
//!   100 ms, the producers once per frame or block. The `.part` file is
//!   deleted and no final file appears.
//! - **Failures** map to EX-022's catalogue ([`FailureCode`]); every failure
//!   deletes the `.part` file. A panic anywhere in the job becomes the
//!   `internal` failure instead of a crash.
//! - **Replace** writes to the `.part` file and renames it over the old file
//!   at the end, so the old file survives until the new one is complete.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::OpenOptions;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, bounded, unbounded};
use freecut_audio::{DocSchedule, FileOpener, OfflineMix};
use freecut_media::CancelToken;
use freecut_media::encode::{
    AudioSpec, EncodeError, EncodeSession, EncodeSpec, VideoQuality, verify_output_with_cancel,
    video_due_before_audio,
};
use freecut_project::export::{
    ConflictChoice, Container, ExportSettings, resolve_conflict, sanitize_file_stem, target_path,
};
use freecut_render::Compositor;
use freecut_render::FontRegistry;
use freecut_render::export::{ExportRenderError, ExportRenderer};
use freecut_timeline::export::ExportClock;
use freecut_timeline::ids::{ClipId, MediaId, SeqId, TrackId};
use freecut_timeline::model::{Document, Sequence};
use freecut_timeline::motion::{PresetCatalog, PresetTable};
use freecut_timeline::time::{FrameRate, SampleRate, Tick, TimeRange, Timecode};

use super::progress::{Phase, Progress, ProgressCell};

/// Rendered frames waiting for the encoder. Small: a 4K frame is 33 MB.
const VIDEO_QUEUE: usize = 4;
/// Mixed audio blocks waiting for the encoder.
const AUDIO_QUEUE: usize = 16;
/// How often a waiting mux loop checks for cancel.
const POLL: Duration = Duration::from_millis(100);
/// Alternative `.part` names tried when `<final>.part` is taken.
const PART_ATTEMPTS: u32 = 100;

/// Everything an export needs, taken when Export is pressed (EX-022 step 1).
#[derive(Debug, Clone)]
pub struct ExportRequest {
    pub doc: Arc<Document>,
    pub seq: SeqId,
    /// Where each media item's file is on this machine.
    pub media_paths: BTreeMap<MediaId, PathBuf>,
    /// Items whose file is missing: the offline slate and silence (EX-005).
    pub offline: BTreeSet<MediaId>,
    pub settings: ExportSettings,
    /// What to do when the file exists (EX-023).
    pub conflict: ConflictChoice,
    /// Shared with the preview; the job waits for a running system scan.
    pub fonts: FontRegistry,
    /// The animation presets the preview plays (FX-030), so the file
    /// matches the preview frame for frame.
    pub presets: Arc<PresetTable>,
    /// "Export as heard" (AU-012): only these soloed tracks sound. `None`
    /// exports every track, ignoring solo (the default).
    pub solo: Option<BTreeSet<TrackId>>,
    /// Where the user LUTs of colour stacks come from, the preview's own
    /// (CL-017), so the file matches the preview.
    pub luts: Arc<dyn freecut_render::color::LutSource>,
    /// "Retry using slates for unreadable frames" (EX-022 `decode-error`):
    /// a picture that can't be read draws the error slate (EX-005) instead
    /// of stopping the export, and the result lists those frames.
    pub slates_for_unreadable: bool,
}

/// The concrete numbers an export runs with (export-pipeline §4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportPlan {
    pub clock: ExportClock,
    pub spec: EncodeSpec,
    pub folder: PathBuf,
    /// The sanitized file name, without extension.
    pub stem: String,
    pub container: Container,
    pub conflict: ConflictChoice,
}

impl ExportPlan {
    /// Resolves `settings` for exporting all of sequence `seq`. Fails only
    /// on what preflight (EX-036) already blocks: an empty timeline, no
    /// name, no folder.
    pub fn resolve(
        settings: &ExportSettings,
        doc: &Document,
        seq: SeqId,
        conflict: ConflictChoice,
    ) -> Result<ExportPlan, ExportFailure> {
        let project = doc.settings();
        let duration = doc
            .sequence(seq)
            .map(Sequence::duration)
            .ok_or_else(|| ExportFailure::internal("the sequence to export doesn't exist"))?;
        let range = TimeRange::new(Tick::ZERO, duration)
            .ok()
            .filter(|_| duration.is_positive())
            .ok_or_else(|| ExportFailure::internal("the timeline is empty"))?;
        let rate = settings.output_rate(project.frame_rate);
        let clock = ExportClock::new(range, rate)
            .map_err(|e| ExportFailure::internal(format!("frame times: {e}")))?;
        let (width, height) = settings.output_dimensions((project.width, project.height));
        let stem = sanitize_file_stem(&settings.file.name);
        if stem.is_empty() {
            return Err(ExportFailure::internal("the file name is empty"));
        }
        let folder = settings.file.folder.clone().ok_or(ExportFailure {
            code: FailureCode::DestUnavailable,
            detail: "no export folder is chosen".to_owned(),
            subject: None,
        })?;
        let spec = EncodeSpec {
            width,
            height,
            frame_rate: rate,
            video: VideoQuality::TargetKbps(settings.video_kbps(project)),
            audio: settings.audio.include.then_some(AudioSpec {
                sample_rate: settings.audio.sample_rate,
                channels: 2,
                kbps: settings.audio.kbps,
            }),
            // EX-040: the title tag is the name field, nothing else.
            title: Some(stem.clone()),
        };
        Ok(ExportPlan {
            clock,
            spec,
            folder,
            stem,
            container: settings.video.container,
            conflict,
        })
    }

    /// The final file for `self.conflict`, given what exists now. `None`
    /// when Keep both ran out of numbers.
    pub fn target(&self, exists: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
        target_path(
            &self.folder,
            &self.stem,
            self.container,
            self.conflict,
            exists,
        )
    }

    /// Audio sample frames the file must hold (0 without audio).
    pub fn audio_samples(&self) -> Result<u64, ExportFailure> {
        match &self.spec.audio {
            None => Ok(0),
            Some(a) => self
                .clock
                .sample_count(a.sample_rate)
                .map_err(|e| ExportFailure::internal(format!("sample count: {e}"))),
        }
    }
}

/// EX-022's failure catalogue: stable codes shown small under the message,
/// so users can look them up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FailureCode {
    DiskFull,
    FileTooLarge,
    DestUnavailable,
    NoPermission,
    OutOfMemory,
    SourceChanged,
    DecodeError,
    EncoderFailed,
    Internal,
}

/// What a failure view offers (EX-022).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FixAction {
    /// Back to the dialog with the folder picker open.
    ChooseFolder,
    /// Run the same export again (with a new snapshot).
    Retry,
    /// Run it again, drawing the error slate for pictures that can't be
    /// read (EX-022 `decode-error`).
    RetryWithSlates,
}

impl FailureCode {
    /// The stable identifier.
    pub const fn id(self) -> &'static str {
        match self {
            FailureCode::DiskFull => "disk-full",
            FailureCode::FileTooLarge => "file-too-large",
            FailureCode::DestUnavailable => "dest-unavailable",
            FailureCode::NoPermission => "no-permission",
            FailureCode::OutOfMemory => "out-of-memory",
            FailureCode::SourceChanged => "source-changed",
            FailureCode::DecodeError => "decode-error",
            FailureCode::EncoderFailed => "encoder-failed",
            FailureCode::Internal => "internal",
        }
    }

    /// The suggested fixes, as buttons.
    pub const fn fixes(self) -> &'static [FixAction] {
        match self {
            FailureCode::DiskFull
            | FailureCode::FileTooLarge
            | FailureCode::DestUnavailable
            | FailureCode::NoPermission => &[FixAction::ChooseFolder],
            FailureCode::OutOfMemory
            | FailureCode::SourceChanged
            | FailureCode::EncoderFailed
            | FailureCode::Internal => &[FixAction::Retry],
            FailureCode::DecodeError => &[FixAction::RetryWithSlates],
        }
    }
}

/// Why an export stopped (EX-022).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportFailure {
    pub code: FailureCode,
    /// The technical reason, for "Copy details" (may contain paths).
    pub detail: String,
    /// The file or volume the message names, if any.
    pub subject: Option<String>,
}

impl ExportFailure {
    pub fn internal(detail: impl Into<String>) -> ExportFailure {
        ExportFailure {
            code: FailureCode::Internal,
            detail: detail.into(),
            subject: None,
        }
    }

    fn with_subject(mut self, subject: Option<String>) -> ExportFailure {
        if self.subject.is_none() {
            self.subject = subject;
        }
        self
    }

    /// The plain-language sentence the failure view shows (FreeCut's own
    /// wording).
    pub fn message(&self) -> String {
        let volume = self.subject.as_deref();
        match self.code {
            FailureCode::DiskFull => match volume {
                Some(v) => format!("Not enough space on “{v}”. Free some space, or choose another folder."),
                None => "Not enough space on the destination drive. Free some space, or choose another folder.".to_owned(),
            },
            FailureCode::FileTooLarge => format!(
                "{} can't hold a file this large. A smaller quality setting makes a smaller file.",
                volume.map_or_else(|| "This drive".to_owned(), |v| format!("“{v}”"))
            ),
            FailureCode::DestUnavailable => "The export folder is no longer available.".to_owned(),
            FailureCode::NoPermission => {
                if cfg!(target_os = "macos") {
                    "FreeCut isn't allowed to save in this folder. Check System Settings › Privacy & Security › Files and Folders, or choose another folder.".to_owned()
                } else {
                    "FreeCut isn't allowed to save in this folder. Choose another folder.".to_owned()
                }
            }
            FailureCode::OutOfMemory => {
                "Not enough memory to export at this size. Close other apps, or choose a lower resolution.".to_owned()
            }
            FailureCode::SourceChanged => format!(
                "“{}” changed while exporting.",
                volume.unwrap_or("A media file")
            ),
            FailureCode::DecodeError => format!(
                "The picture of {} can't be read. The file may be damaged.",
                volume.unwrap_or("a clip")
            ),
            FailureCode::EncoderFailed => "The video encoder stopped with an error.".to_owned(),
            FailureCode::Internal => "Something went wrong inside FreeCut.".to_owned(),
        }
    }

    /// The text "Copy details" puts on the clipboard: code, message,
    /// settings, versions. File paths appear only here (EX-022).
    pub fn details(&self, settings: &ExportSettings, doc: &Document) -> String {
        let project = doc.settings();
        let (w, h) = settings.output_dimensions((project.width, project.height));
        let rate = settings.output_rate(project.frame_rate);
        let ffmpeg = freecut_media::init().map_or_else(
            |e| format!("unavailable ({e})"),
            |caps| {
                caps.versions
                    .iter()
                    .map(|(lib, v)| format!("{lib} {v}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            },
        );
        format!(
            "FreeCut export failed: {code}\n{message}\nDetail: {detail}\n\
             Settings: {w}x{h}, {rate} fps, H.264/MP4, {kbps} kbps, audio {audio}\n\
             FreeCut {version}; FFmpeg: {ffmpeg}\n",
            code = self.code.id(),
            message = self.message(),
            detail = self.detail,
            kbps = settings.video_kbps(project),
            audio = if settings.audio.include {
                format!(
                    "AAC {} kbps {} Hz",
                    settings.audio.kbps,
                    settings.audio.sample_rate.hz()
                )
            } else {
                "off".to_owned()
            },
            version = env!("CARGO_PKG_VERSION"),
        )
    }
}

/// Maps a file-system error to the catalogue (pure, table-tested).
pub fn failure_from_io(e: &io::Error) -> FailureCode {
    use io::ErrorKind as K;
    match e.kind() {
        K::StorageFull | K::QuotaExceeded => FailureCode::DiskFull,
        K::FileTooLarge => FailureCode::FileTooLarge,
        K::PermissionDenied | K::ReadOnlyFilesystem => FailureCode::NoPermission,
        K::NotFound | K::NotADirectory | K::NotConnected | K::StaleNetworkFileHandle => {
            FailureCode::DestUnavailable
        }
        K::OutOfMemory => FailureCode::OutOfMemory,
        _ => FailureCode::EncoderFailed,
    }
}

/// Maps an encoder error to the catalogue (pure, table-tested). FFmpeg's
/// error numbers are `-errno` with the C library's values, the same on
/// macOS, Linux and the Windows CRT for these codes.
pub fn failure_from_encode(e: &EncodeError) -> FailureCode {
    const EPERM: i32 = 1;
    const ENOENT: i32 = 2;
    const ENOMEM: i32 = 12;
    const EACCES: i32 = 13;
    const EROFS: i32 = 30;
    match e {
        EncodeError::DiskFull => FailureCode::DiskFull,
        EncodeError::FileTooLarge => FailureCode::FileTooLarge,
        EncodeError::Io(io) => failure_from_io(io),
        EncodeError::UnsupportedPath(_) => FailureCode::DestUnavailable,
        EncodeError::Ffmpeg { code, .. } => match -*code {
            ENOENT => FailureCode::DestUnavailable,
            EPERM | EACCES | EROFS => FailureCode::NoPermission,
            ENOMEM => FailureCode::OutOfMemory,
            _ => FailureCode::EncoderFailed,
        },
        EncodeError::InvalidInput(_) | EncodeError::OutOfOrder { .. } => FailureCode::Internal,
        _ => FailureCode::EncoderFailed,
    }
}

fn encode_failure(e: &EncodeError) -> Stop {
    if matches!(e, EncodeError::Cancelled) {
        return Stop::Cancelled;
    }
    Stop::Failed(ExportFailure {
        code: failure_from_encode(e),
        detail: e.to_string(),
        subject: None,
    })
}

fn io_failure(what: &str, e: &io::Error) -> ExportFailure {
    ExportFailure {
        code: failure_from_io(e),
        detail: format!("{what}: {e}"),
        subject: None,
    }
}

/// A finished export, for the result view (EX-039).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportResult {
    pub path: PathBuf,
    pub bytes: u64,
    /// The length of the exported range.
    pub duration: Tick,
    pub size: (u32, u32),
    pub frame_rate: FrameRate,
    pub frames: u64,
    pub audio: bool,
    /// How long the export took.
    pub took: Duration,
    /// Frames drawn with the error slate because a picture couldn't be
    /// read ("Retry using slates", EX-022): the clip and timeline time of
    /// each, in frame order.
    pub slated: Vec<String>,
}

/// How a job ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportEnd {
    Done(ExportResult),
    Failed(ExportFailure),
    /// Stopped by Cancel; nothing was left on disk.
    Cancelled,
}

/// Why the job body stopped early.
#[derive(Debug)]
enum Stop {
    Cancelled,
    Failed(ExportFailure),
}

impl From<ExportFailure> for Stop {
    fn from(f: ExportFailure) -> Stop {
        Stop::Failed(f)
    }
}

fn check(cancel: &CancelToken) -> Result<(), Stop> {
    if cancel.is_cancelled() {
        Err(Stop::Cancelled)
    } else {
        Ok(())
    }
}

/// A running export. Dropping it cancels the job and waits for it, so
/// quitting never leaves a partial file.
#[derive(Debug)]
pub struct ExportJob {
    progress: Arc<ProgressCell>,
    cancel: CancelToken,
    started: Instant,
    rate: FrameRate,
    end: Receiver<ExportEnd>,
    thread: Option<JoinHandle<()>>,
}

impl ExportJob {
    /// Starts `request` on its own threads. `wake` is called when the job
    /// ends (the app passes a repaint request).
    pub fn start(request: ExportRequest, wake: impl Fn() + Send + 'static) -> ExportJob {
        let progress = Arc::new(ProgressCell::default());
        let cancel = CancelToken::new();
        let rate = request
            .settings
            .output_rate(request.doc.settings().frame_rate);
        let (tx, end) = bounded(1);
        let thread = {
            let progress = Arc::clone(&progress);
            let cancel = cancel.clone();
            let tx = tx.clone();
            std::thread::Builder::new()
                .name("freecut-export".to_owned())
                .spawn(move || {
                    let outcome = run(request, &progress, &cancel);
                    progress.set_phase(Phase::Ended);
                    let _ = tx.send(outcome);
                    wake();
                })
        };
        let thread = match thread {
            Ok(handle) => Some(handle),
            Err(e) => {
                let _ = tx.send(ExportEnd::Failed(ExportFailure::internal(format!(
                    "couldn't start the export thread: {e}"
                ))));
                None
            }
        };
        ExportJob {
            progress,
            cancel,
            started: Instant::now(),
            rate,
            end,
            thread,
        }
    }

    pub fn progress(&self) -> Progress {
        self.progress.read()
    }

    pub fn started(&self) -> Instant {
        self.started
    }

    /// The output frame rate, for the speed readout.
    pub fn frame_rate(&self) -> FrameRate {
        self.rate
    }

    /// Asks the job to stop; it ends within about a second.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub fn is_cancelling(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// The outcome once the job has ended, without blocking.
    pub fn poll(&mut self) -> Option<ExportEnd> {
        let end = self.end.try_recv().ok()?;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        Some(end)
    }

    /// Blocks until the job ends or `timeout` passes.
    #[cfg(test)]
    pub fn wait(&mut self, timeout: Duration) -> Option<ExportEnd> {
        let end = self.end.recv_timeout(timeout).ok()?;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        Some(end)
    }
}

impl Drop for ExportJob {
    fn drop(&mut self) {
        if let Some(t) = self.thread.take() {
            self.cancel.cancel();
            let _ = t.join();
        }
    }
}

/// The job thread's body: never panics out, never leaves a `.part` file.
fn run(request: ExportRequest, progress: &ProgressCell, cancel: &CancelToken) -> ExportEnd {
    let started = Instant::now();
    match catch_unwind(AssertUnwindSafe(|| {
        run_inner(request, progress, cancel, started)
    })) {
        Ok(Ok(result)) => ExportEnd::Done(result),
        Ok(Err(Stop::Cancelled)) => ExportEnd::Cancelled,
        Ok(Err(Stop::Failed(f))) => ExportEnd::Failed(f),
        Err(panic) => ExportEnd::Failed(ExportFailure::internal(format!(
            "the export panicked: {}",
            panic_text(panic.as_ref())
        ))),
    }
}

fn panic_text(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".to_owned())
}

fn run_inner(
    request: ExportRequest,
    progress: &ProgressCell,
    cancel: &CancelToken,
    started: Instant,
) -> Result<ExportResult, Stop> {
    let plan = ExportPlan::resolve(
        &request.settings,
        &request.doc,
        request.seq,
        request.conflict,
    )?;
    let frames = plan.clock.frame_count();
    let samples = plan.audio_samples()?;
    progress.set_total(frames);
    check(cancel)?;

    let volume = super::platform::volume_name(&plan.folder);
    let fail_here = |f: ExportFailure| Stop::Failed(f.with_subject(volume.clone()));
    let stamps = SourceStamps::record(
        &request.doc,
        request.seq,
        &request.media_paths,
        &request.offline,
    );
    let target = plan.target(&|p| p.exists()).ok_or_else(|| {
        fail_here(ExportFailure {
            code: FailureCode::DestUnavailable,
            detail: "every Keep-both name is taken".to_owned(),
            subject: None,
        })
    })?;
    let part = PartFile::claim(&target).map_err(|e| fail_here(io_failure("claiming", &e)))?;
    let mut session =
        EncodeSession::create(part.path(), &plan.spec).map_err(|e| match encode_failure(&e) {
            Stop::Failed(f) => fail_here(f),
            stop => stop,
        })?;

    progress.set_phase(Phase::Rendering);
    let slated = match encode_streams(&request, &plan, &mut session, progress, cancel) {
        Ok(slated) => slated,
        Err(stop) => {
            let _ = session.abandon();
            return Err(match stop {
                Stop::Failed(f) => fail_here(f),
                stop => stop,
            });
        }
    };
    if let Err(changed) = stamps.check() {
        let _ = session.abandon();
        return Err(Stop::Failed(changed));
    }

    progress.set_phase(Phase::Finishing);
    let stats = session
        .finish(cancel)
        .map_err(|e| match encode_failure(&e) {
            Stop::Failed(f) => fail_here(f),
            stop => stop,
        })?;
    progress.set_frames(frames, stats.video_packets);
    // P6: success needs a passing read-back of the finished file.
    verify_output_with_cancel(part.path(), cancel)
        .and_then(|info| info.check(&plan.spec, frames, samples))
        .map_err(|e| encode_failure(&e))?;
    check(cancel)?;
    sync_file(part.path()).map_err(|e| fail_here(io_failure("syncing", &e)))?;

    // Keep both is decided again now: a file may have appeared meanwhile.
    let dest = match plan.conflict {
        ConflictChoice::Replace => target,
        ConflictChoice::KeepBoth if !target.exists() => target,
        ConflictChoice::KeepBoth => {
            resolve_conflict(&plan.folder, &plan.stem, plan.container, &|p| p.exists()).ok_or_else(
                || {
                    fail_here(ExportFailure {
                        code: FailureCode::DestUnavailable,
                        detail: "every Keep-both name is taken".to_owned(),
                        subject: None,
                    })
                },
            )?
        }
    };
    let bytes = part
        .place(&dest)
        .map_err(|e| fail_here(io_failure("renaming", &e)))?;
    sync_parent(&dest);
    Ok(ExportResult {
        path: dest,
        bytes,
        duration: plan.clock.range().duration(),
        size: (plan.spec.width, plan.spec.height),
        frame_rate: plan.spec.frame_rate,
        frames,
        audio: plan.spec.audio.is_some(),
        took: started.elapsed(),
        slated,
    })
}

/// A message from the audio thread: a block of interleaved stereo, or why
/// the mix stopped.
type AudioMsg = Result<Vec<f32>, String>;

/// A message from the render thread.
enum VideoMsg {
    Frame(Vec<u8>),
    Failed(ExportFailure),
}

/// Runs the producers and the mux loop. Returns when every frame and sample
/// is in the encoder, or on the first error or cancel; on success, the
/// frames drawn with slates (see [`ExportResult::slated`]).
fn encode_streams(
    request: &ExportRequest,
    plan: &ExportPlan,
    session: &mut EncodeSession,
    progress: &ProgressCell,
    cancel: &CancelToken,
) -> Result<Vec<String>, Stop> {
    let stop = AtomicBool::new(false);
    std::thread::scope(|s| {
        let (video_tx, video_rx) = bounded::<VideoMsg>(VIDEO_QUEUE);
        let (free_tx, free_rx) = unbounded::<Vec<u8>>();
        let stop = &stop;
        let video = std::thread::Builder::new()
            .name("freecut-export-render".to_owned())
            .spawn_scoped(s, move || {
                render_frames(request, plan, &video_tx, &free_rx, stop)
            })
            .map_err(|e| ExportFailure::internal(format!("render thread: {e}")))?;
        let (audio, audio_rx) = match plan.spec.audio {
            None => (None, None),
            Some(a) => {
                let (tx, rx) = bounded::<AudioMsg>(AUDIO_QUEUE);
                let handle = std::thread::Builder::new()
                    .name("freecut-export-audio".to_owned())
                    .spawn_scoped(s, move || {
                        mix_audio(request, plan, a.sample_rate, &tx, stop);
                    })
                    .map_err(|e| ExportFailure::internal(format!("audio thread: {e}")))?;
                (Some(handle), Some((rx, a.sample_rate)))
            }
        };
        let streams = Streams {
            video: &video_rx,
            audio: audio_rx.as_ref().map(|(rx, rate)| (rx, *rate)),
            free: &free_tx,
        };
        let result = mux_loop(session, plan, &streams, progress, cancel);
        // Producers blocked on a full queue wake up with an error.
        stop.store(true, Ordering::Release);
        drop(video_rx);
        drop(audio_rx);
        let (slated, video_panicked) = match video.join() {
            Ok(slated) => (slated, false),
            Err(_) => (Vec::new(), true),
        };
        let audio_panicked = audio.is_some_and(|h| h.join().is_err());
        match result.map(|()| slated) {
            Err(Stop::Cancelled) => Err(Stop::Cancelled),
            _ if video_panicked || audio_panicked => {
                Err(Stop::Failed(ExportFailure::internal(if video_panicked {
                    "the render thread panicked"
                } else {
                    "the audio thread panicked"
                })))
            }
            other => other,
        }
    })
}

struct Streams<'a> {
    video: &'a Receiver<VideoMsg>,
    audio: Option<(&'a Receiver<AudioMsg>, SampleRate)>,
    /// Frame buffers going back to the render thread for reuse.
    free: &'a Sender<Vec<u8>>,
}

/// The render thread: frames in order, through the shared CPU compositor.
/// Returns the frames drawn with slates for unreadable pictures.
fn render_frames(
    request: &ExportRequest,
    plan: &ExportPlan,
    tx: &Sender<VideoMsg>,
    free: &Receiver<Vec<u8>>,
    stop: &AtomicBool,
) -> Vec<String> {
    let compositor = Compositor::new(request.fonts.clone()).with_luts(Arc::clone(&request.luts));
    let renderer = ExportRenderer::with_media(
        Arc::clone(&request.doc),
        request.seq,
        plan.clock,
        (plan.spec.width, plan.spec.height),
        compositor,
        &request.media_paths,
    );
    let presets: Arc<dyn PresetCatalog + Send + Sync> = request.presets.clone();
    let mut renderer = match renderer {
        Ok(r) if request.slates_for_unreadable => r
            .with_presets(presets)
            .with_offline(request.offline.clone())
            .with_slates_for_unreadable(),
        Ok(r) => r
            .with_presets(presets)
            .with_offline(request.offline.clone()),
        Err(e) => {
            let _ = tx.send(VideoMsg::Failed(ExportFailure::internal(format!(
                "can't render: {e}"
            ))));
            return Vec::new();
        }
    };
    let slated = |renderer: &ExportRenderer<_>| {
        renderer
            .unreadable()
            .iter()
            .map(|f| frame_label(&request.doc, f.clip, f.at))
            .collect()
    };
    for k in 0..renderer.frame_count() {
        if stop.load(Ordering::Acquire) {
            return slated(&renderer);
        }
        let msg = match renderer.render_frame(k) {
            Ok(frame) => {
                let mut buf = free.try_recv().unwrap_or_default();
                buf.clear();
                buf.extend_from_slice(&frame.rgba);
                VideoMsg::Frame(buf)
            }
            Err(e) => VideoMsg::Failed(render_failure(&request.doc, k, &e)),
        };
        let failed = matches!(msg, VideoMsg::Failed(_));
        if tx.send(msg).is_err() || failed {
            return slated(&renderer);
        }
    }
    slated(&renderer)
}

/// Maps a render error to the catalogue: a picture that can't be read is
/// `decode-error`, naming the clip and timeline time (EX-022); anything
/// else is a bug (`internal`).
fn render_failure(doc: &Document, frame: u64, e: &ExportRenderError) -> ExportFailure {
    match e {
        ExportRenderError::Decode { clip, at, reason } => ExportFailure {
            code: FailureCode::DecodeError,
            detail: format!("frame {frame}: {reason}"),
            subject: Some(frame_label(doc, *clip, *at)),
        },
        other => ExportFailure::internal(format!("frame {frame}: {other}")),
    }
}

/// "“Interview” at 00:00:01:05": a clip and a timeline time, as messages
/// name them.
fn frame_label(doc: &Document, clip: Option<ClipId>, at: Tick) -> String {
    let rate = doc.settings().frame_rate;
    let time =
        Timecode::from_tick(at, rate, false).map_or_else(|_| at.to_string(), |tc| tc.to_string());
    match clip.and_then(|id| doc.clip(id)) {
        Some(c) => format!(
            "“{}” at {time}",
            crate::ui::timeline::clip_label(doc, &c.content, c.name.as_deref())
        ),
        None => format!("a clip at {time}"),
    }
}

/// The audio thread: the export range through the playback mixer (AU-010).
fn mix_audio(
    request: &ExportRequest,
    plan: &ExportPlan,
    rate: SampleRate,
    tx: &Sender<AudioMsg>,
    stop: &AtomicBool,
) {
    let schedule =
        DocSchedule::new(Arc::clone(&request.doc), request.seq).with_solo(request.solo.clone());
    let opener = FileOpener::from_paths(&request.media_paths, &request.offline);
    let mut mix = match OfflineMix::for_document(&schedule, &plan.clock, rate) {
        Ok(m) => m,
        Err(e) => {
            let _ = tx.send(Err(format!("can't mix the sound: {e}")));
            return;
        }
    };
    // AU-012: solo is ignored unless the user chose "Export as heard".
    mix.set_export_as_heard(request.solo.is_some());
    while !stop.load(Ordering::Acquire) {
        let Some(block) = mix.next_block(&schedule, &opener) else {
            return;
        };
        if tx.send(Ok(block.to_vec())).is_err() {
            return;
        }
    }
}

/// Pulls from whichever stream is behind in presentation time and pushes it
/// into the encoder (export-pipeline §7.2).
fn mux_loop(
    session: &mut EncodeSession,
    plan: &ExportPlan,
    streams: &Streams<'_>,
    progress: &ProgressCell,
    cancel: &CancelToken,
) -> Result<(), Stop> {
    let frames = plan.clock.frame_count();
    let rate = plan.spec.frame_rate;
    let samples = plan.audio_samples()?;
    let (mut next_v, mut next_a) = (0u64, 0u64);
    loop {
        check(cancel)?;
        let video_left = next_v < frames;
        let audio_left = next_a < samples;
        let take_video = match (video_left, audio_left, &streams.audio) {
            (false, false, _) => return Ok(()),
            (true, true, Some((_, sr))) => video_due_before_audio(next_v, rate, next_a, *sr),
            (video, _, _) => video,
        };
        if take_video {
            match recv(streams.video, cancel)? {
                VideoMsg::Frame(buf) => {
                    session
                        .push_video(&buf, next_v)
                        .map_err(|e| encode_failure(&e))?;
                    next_v += 1;
                    let _ = streams.free.send(buf);
                    progress.set_frames(next_v, session.video_packets_written());
                }
                VideoMsg::Failed(failure) => return Err(failure.into()),
            }
        } else {
            let Some((rx, _)) = &streams.audio else {
                return Err(ExportFailure::internal("audio expected without a stream").into());
            };
            let block = recv(rx, cancel)?.map_err(ExportFailure::internal)?;
            if block.is_empty() {
                return Err(ExportFailure::internal("the mixer returned no sound").into());
            }
            session.push_audio(&block).map_err(|e| encode_failure(&e))?;
            next_a += (block.len() / 2) as u64;
        }
    }
}

/// Waits for the next message, checking for cancel every [`POLL`].
fn recv<T>(rx: &Receiver<T>, cancel: &CancelToken) -> Result<T, Stop> {
    loop {
        match rx.recv_timeout(POLL) {
            Ok(msg) => return Ok(msg),
            Err(RecvTimeoutError::Timeout) => check(cancel)?,
            Err(RecvTimeoutError::Disconnected) => {
                check(cancel)?;
                return Err(ExportFailure::internal("a producer stopped early").into());
            }
        }
    }
}

/// The claimed temporary file next to the target. Dropping it deletes the
/// file unless it was placed.
#[derive(Debug)]
struct PartFile {
    path: PathBuf,
    placed: bool,
}

impl PartFile {
    /// Claims `<target>.part` with `create_new`, so FFmpeg (which truncates)
    /// never clobbers a file someone else is writing; a taken name moves on
    /// to `<target>.<n>.part`.
    fn claim(target: &Path) -> io::Result<PartFile> {
        for n in 0..PART_ATTEMPTS {
            let path = part_path(target, n);
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => {
                    return Ok(PartFile {
                        path,
                        placed: false,
                    });
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "too many leftover .part files next to the target",
        ))
    }

    fn path(&self) -> &Path {
        &self.path
    }

    /// Renames the file onto `dest` (an atomic replace on the same volume)
    /// and returns its size.
    fn place(mut self, dest: &Path) -> io::Result<u64> {
        let bytes = std::fs::metadata(&self.path)?.len();
        std::fs::rename(&self.path, dest)?;
        self.placed = true;
        Ok(bytes)
    }
}

impl Drop for PartFile {
    fn drop(&mut self) {
        if !self.placed {
            match std::fs::remove_file(&self.path) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => {
                    eprintln!("FreeCut: couldn't remove {}: {e}", self.path.display());
                }
                _ => {}
            }
        }
    }
}

/// `<target>.part`, or `<target>.<n>.part` for `n > 0`.
pub fn part_path(target: &Path, n: u32) -> PathBuf {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    if n > 0 {
        name.push(format!(".{n}"));
    }
    name.push(".part");
    target.with_file_name(name)
}

fn sync_file(path: &Path) -> io::Result<()> {
    // Windows FlushFileBuffers requires GENERIC_WRITE. Reopen the completed
    // file for writing without creating or truncating it, then make it durable.
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)?
        .sync_all()
}

/// Best effort: makes the rename durable on Unix. Windows can't open a
/// folder as a file, which is fine.
fn sync_parent(path: &Path) {
    if let Some(dir) = path.parent()
        && let Ok(f) = std::fs::File::open(dir)
    {
        let _ = f.sync_all();
    }
}

/// Size and modification time of each source file the export reads,
/// recorded at job start (EX-022 "source check").
#[derive(Debug)]
struct SourceStamps {
    files: Vec<(PathBuf, Option<(u64, SystemTime)>)>,
}

impl SourceStamps {
    fn stamp(path: &Path) -> Option<(u64, SystemTime)> {
        let meta = std::fs::metadata(path).ok()?;
        Some((meta.len(), meta.modified().ok()?))
    }

    fn record(
        doc: &Document,
        seq: SeqId,
        paths: &BTreeMap<MediaId, PathBuf>,
        offline: &BTreeSet<MediaId>,
    ) -> SourceStamps {
        let used: BTreeSet<MediaId> = doc
            .sequence(seq)
            .map(|s| {
                s.tracks()
                    .flat_map(|(_, t)| t.clips())
                    .filter_map(|c| c.content.asset())
                    .collect()
            })
            .unwrap_or_default();
        let files: BTreeSet<&PathBuf> = used
            .iter()
            .filter(|id| !offline.contains(id))
            .filter_map(|id| paths.get(id))
            .collect();
        SourceStamps {
            files: files
                .into_iter()
                .map(|p| (p.clone(), SourceStamps::stamp(p)))
                .collect(),
        }
    }

    /// `source-changed` when a recorded file is gone or differs now.
    fn check(&self) -> Result<(), ExportFailure> {
        for (path, before) in &self.files {
            if before.is_some() && SourceStamps::stamp(path) != *before {
                return Err(ExportFailure {
                    code: FailureCode::SourceChanged,
                    detail: format!("{} changed during the export", path.display()),
                    subject: path.file_name().map(|n| n.to_string_lossy().into_owned()),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use freecut_project::export::{OutputSize, ShortSide};
    use freecut_timeline::model::ProjectSettings;

    use super::*;

    #[test]
    fn io_errors_map_to_the_catalogue() {
        use io::ErrorKind as K;
        let table = [
            (K::StorageFull, FailureCode::DiskFull),
            (K::QuotaExceeded, FailureCode::DiskFull),
            (K::FileTooLarge, FailureCode::FileTooLarge),
            (K::PermissionDenied, FailureCode::NoPermission),
            (K::ReadOnlyFilesystem, FailureCode::NoPermission),
            (K::NotFound, FailureCode::DestUnavailable),
            (K::NotADirectory, FailureCode::DestUnavailable),
            (K::NotConnected, FailureCode::DestUnavailable),
            (K::OutOfMemory, FailureCode::OutOfMemory),
            (K::Other, FailureCode::EncoderFailed),
        ];
        for (kind, code) in table {
            assert_eq!(failure_from_io(&io::Error::from(kind)), code, "{kind:?}");
        }
    }

    #[test]
    fn encoder_errors_map_to_the_catalogue() {
        let ff = |code: i32| EncodeError::Ffmpeg {
            op: "x",
            code,
            msg: String::new(),
        };
        let table = [
            (EncodeError::DiskFull, FailureCode::DiskFull),
            (EncodeError::FileTooLarge, FailureCode::FileTooLarge),
            (
                EncodeError::Io(io::Error::from(io::ErrorKind::StorageFull)),
                FailureCode::DiskFull,
            ),
            (
                EncodeError::UnsupportedPath("not UTF-8"),
                FailureCode::DestUnavailable,
            ),
            (ff(-2), FailureCode::DestUnavailable),
            (ff(-1), FailureCode::NoPermission),
            (ff(-13), FailureCode::NoPermission),
            (ff(-30), FailureCode::NoPermission),
            (ff(-12), FailureCode::OutOfMemory),
            (ff(-22), FailureCode::EncoderFailed),
            (
                EncodeError::EncoderUnavailable("libx264"),
                FailureCode::EncoderFailed,
            ),
            (
                EncodeError::Verify("2 packets".into()),
                FailureCode::EncoderFailed,
            ),
            (
                EncodeError::OutOfOrder {
                    expected: 1,
                    got: 2,
                },
                FailureCode::Internal,
            ),
            (
                EncodeError::InvalidInput("size".into()),
                FailureCode::Internal,
            ),
        ];
        for (e, code) in table {
            assert_eq!(failure_from_encode(&e), code, "{e}");
        }
        assert!(matches!(
            encode_failure(&EncodeError::Cancelled),
            Stop::Cancelled
        ));
    }

    #[test]
    fn every_failure_has_a_code_a_message_and_a_fix() {
        let codes = [
            FailureCode::DiskFull,
            FailureCode::FileTooLarge,
            FailureCode::DestUnavailable,
            FailureCode::NoPermission,
            FailureCode::OutOfMemory,
            FailureCode::SourceChanged,
            FailureCode::DecodeError,
            FailureCode::EncoderFailed,
            FailureCode::Internal,
        ];
        let mut ids = BTreeSet::new();
        for code in codes {
            assert!(ids.insert(code.id()), "unique id {}", code.id());
            assert!(!code.fixes().is_empty());
            let f = ExportFailure {
                code,
                detail: "/secret/path".into(),
                subject: Some("Card A".into()),
            };
            let message = f.message();
            assert!(!message.is_empty());
            assert!(
                !message.contains("/secret/path"),
                "paths only in the copied details"
            );
        }
        let full = ExportFailure {
            code: FailureCode::DiskFull,
            detail: String::new(),
            subject: Some("Data".into()),
        };
        assert!(full.message().contains("“Data”"));
        assert_eq!(FailureCode::DiskFull.fixes(), [FixAction::ChooseFolder]);
        assert_eq!(FailureCode::EncoderFailed.fixes(), [FixAction::Retry]);
        assert_eq!(
            FailureCode::DecodeError.fixes(),
            [FixAction::RetryWithSlates]
        );
    }

    #[test]
    fn part_files_sit_next_to_the_target() {
        let t = Path::new("/out/My clip.mp4");
        assert_eq!(part_path(t, 0), Path::new("/out/My clip.mp4.part"));
        assert_eq!(part_path(t, 3), Path::new("/out/My clip.mp4.3.part"));
    }

    #[test]
    fn claiming_never_clobbers_and_dropping_cleans_up() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.mp4");
        // Someone else's leftover.
        std::fs::write(part_path(&target, 0), b"theirs").unwrap();
        let part = PartFile::claim(&target).unwrap();
        assert_eq!(part.path(), part_path(&target, 1));
        assert_eq!(std::fs::read(part_path(&target, 0)).unwrap(), b"theirs");
        drop(part);
        assert!(
            !part_path(&target, 1).exists(),
            "an unplaced part is deleted"
        );
        assert!(part_path(&target, 0).exists(), "the other one is untouched");
    }

    #[test]
    fn placing_replaces_the_old_file_only_at_the_end() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.mp4");
        std::fs::write(&target, b"old").unwrap();
        let part = PartFile::claim(&target).unwrap();
        std::fs::write(part.path(), b"new file").unwrap();
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"old",
            "old file kept meanwhile"
        );
        let bytes = part.place(&target).unwrap();
        assert_eq!(bytes, 8);
        assert_eq!(std::fs::read(&target).unwrap(), b"new file");
        assert!(!part_path(&target, 0).exists());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    /// A document with one 3 s default text clip.
    fn doc_with(settings: ProjectSettings) -> Document {
        let mut project = crate::session::Project::new(settings).unwrap();
        crate::ui::text::insert_text(
            &mut project,
            freecut_timeline::edit::InsertTarget::FirstFree,
            Tick::ZERO,
        )
        .unwrap();
        project.doc().clone()
    }

    #[test]
    fn plans_resolve_size_rate_bitrate_audio_and_name() {
        let doc = doc_with(ProjectSettings {
            width: 1080,
            height: 1920,
            frame_rate: FrameRate::new(30, 1).unwrap(),
            ..ProjectSettings::new_default().unwrap()
        });
        let mut settings = ExportSettings::defaults("My: clip?", Some("/out".into()), &doc);
        settings.audio.include = true;
        settings.video.size = OutputSize::Preset(ShortSide::P720);
        let plan =
            ExportPlan::resolve(&settings, &doc, doc.root(), ConflictChoice::KeepBoth).unwrap();
        assert_eq!((plan.spec.width, plan.spec.height), (720, 1280));
        assert_eq!(plan.spec.frame_rate, FrameRate::new(30, 1).unwrap());
        assert_eq!(plan.clock.frame_count(), 90);
        assert_eq!(plan.stem, "My_ clip_");
        assert_eq!(plan.spec.title.as_deref(), Some("My_ clip_"));
        assert_eq!(
            plan.spec.video,
            VideoQuality::TargetKbps(settings.video_kbps(doc.settings()))
        );
        let audio = plan.spec.audio.unwrap();
        assert_eq!((audio.channels, audio.kbps), (2, 192));
        assert_eq!(
            plan.audio_samples().unwrap(),
            3 * u64::from(audio.sample_rate.hz())
        );
        plan.spec.validate().unwrap();
        // Keep both: the first free name.
        let taken = |p: &Path| p == Path::new("/out/My_ clip_.mp4");
        assert_eq!(
            plan.target(&taken),
            Some(PathBuf::from("/out/My_ clip_ (2).mp4"))
        );
        let replace = ExportPlan {
            conflict: ConflictChoice::Replace,
            ..plan
        };
        assert_eq!(
            replace.target(&taken),
            Some(PathBuf::from("/out/My_ clip_.mp4"))
        );

        // Audio off: no stream, no samples.
        settings.audio.include = false;
        let silent =
            ExportPlan::resolve(&settings, &doc, doc.root(), ConflictChoice::KeepBoth).unwrap();
        assert_eq!(silent.spec.audio, None);
        assert_eq!(silent.audio_samples().unwrap(), 0);
    }

    #[test]
    fn plans_refuse_what_preflight_blocks() {
        let doc = doc_with(ProjectSettings::new_default().unwrap());
        let mut settings = ExportSettings::defaults("clip", None, &doc);
        let no_folder =
            ExportPlan::resolve(&settings, &doc, doc.root(), ConflictChoice::KeepBoth).unwrap_err();
        assert_eq!(no_folder.code, FailureCode::DestUnavailable);
        settings.file.folder = Some("/out".into());
        settings.file.name = "   ".into();
        let no_name =
            ExportPlan::resolve(&settings, &doc, doc.root(), ConflictChoice::KeepBoth).unwrap_err();
        assert_eq!(no_name.code, FailureCode::Internal);
        let empty = crate::session::Project::new(ProjectSettings::new_default().unwrap())
            .unwrap()
            .doc()
            .clone();
        settings.file.name = "clip".into();
        assert!(
            ExportPlan::resolve(&settings, &empty, empty.root(), ConflictChoice::KeepBoth).is_err()
        );
    }
}
