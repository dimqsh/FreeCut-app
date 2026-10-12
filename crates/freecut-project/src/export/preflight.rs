//! Export preflight (EX-036): the checks that decide whether an export may
//! start, as a pure function of the settings, the document and facts the
//! app gathered off the UI thread (free space, folder state, encoders, the
//! GPU limit, offline media, substituted fonts).
//!
//! Issues carry a stable code ([`IssueCode::id`]) and data, not English
//! text: the app turns them into localized messages (README §5.13).

use std::collections::BTreeSet;
use std::path::PathBuf;

use freecut_timeline::ids::{ClipId, MediaId, SeqId, TrackId};
use freecut_timeline::model::{ClipContent, Document, MediaKind, Sequence};
use freecut_timeline::time::{FrameRate, Tick};

use super::bitrate::estimate_size;
use super::names::{MAX_FILE_NAME_BYTES, MAX_STEM_CHARS, output_path, sanitize_file_stem};
use super::settings::{
    AAC_KBPS_CHOICES, AUDIO_SAMPLE_RATES, ExportSettings, MAX_OUTPUT_SIDE, compatible,
};

/// Free space below this blocks an export (EX-036: 100 MB).
pub const MIN_FREE_BYTES: u64 = 100 * 1000 * 1000;

/// How bad an issue is (EX-036).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Blocks Export.
    Problem,
    /// Export proceeds after one confirmation listing the warnings.
    Warning,
    /// Shown, never asked about.
    Info,
}

/// What the app found out about the destination folder (EX-023).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub enum FolderStatus {
    /// Not checked yet: no issue is reported for it.
    #[default]
    Unknown,
    Writable,
    /// The folder doesn't exist (FreeCut never creates it silently).
    Missing,
    /// It exists but can't be written (permissions, read-only volume, OS
    /// privacy settings).
    NotWritable,
    /// It is on an external drive that isn't connected; `drive` is its
    /// name ("'Card A' isn't connected").
    Disconnected {
        drive: String,
    },
}

/// Facts that need I/O or other crates, gathered by the app off the UI
/// thread and passed in as data (EX-036 model notes).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreflightEnv {
    pub folder: FolderStatus,
    /// Free bytes on the destination volume; `None` when unknown (some
    /// network shares), which skips the space checks.
    pub free_bytes: Option<u64>,
    /// The destination volume's name, for the low-space message.
    pub volume: Option<String>,
    /// The GPU's largest texture side (`max_texture_dimension_2d`); `None`
    /// when not known yet.
    pub max_texture_side: Option<u32>,
    /// Whether an encoder for the chosen codec exists (the background
    /// encoder probe, EX-029).
    pub encoder_available: bool,
    /// Media items whose files are offline (PM-090).
    pub offline_media: BTreeSet<MediaId>,
    /// Font families that will render with a substitute (TX-007).
    pub substituted_fonts: Vec<String>,
}

/// What an issue is about, so the dialog can jump to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IssueTarget {
    Clip(ClipId),
    Track(TrackId),
    Media(MediaId),
}

/// One preflight finding, with its data (EX-036).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum IssueCode {
    // Problems.
    /// Nothing to export: the sequence is empty.
    EmptyTimeline,
    /// The name is empty or only whitespace ("Enter a file name").
    EmptyFileName,
    /// The sanitized name is over 200 characters, or the file name over the
    /// file-system limit of 255 bytes.
    FileNameTooLong {
        chars: usize,
        bytes: usize,
    },
    /// No destination folder is chosen.
    NoFolder,
    FolderMissing,
    FolderNotWritable,
    DriveDisconnected {
        drive: String,
    },
    /// The output path is the file of a library item (EX-023: "This file
    /// is used in your project").
    TargetIsSource {
        media: MediaId,
    },
    /// The output is larger than the GPU or the encoder can handle.
    OutputTooLarge {
        width: u32,
        height: u32,
        max: u32,
    },
    /// No encoder for the codec on this machine.
    NoEncoder,
    /// The codec can't be written into the container.
    IncompatibleFormat,
    /// The output rate isn't one the dialog offers (a hand-edited file).
    UnsupportedFrameRate {
        rate: FrameRate,
    },
    AudioBitrateUnsupported {
        kbps: u32,
    },
    AudioSampleRateUnsupported {
        hz: u32,
    },
    /// Less than 100 MB free. `to_free` is how much to free at least: up
    /// to 100 MB, or up to the size estimate when that is larger ("Free at
    /// least 1.2 GB on 'Data', or choose another folder").
    LowDiskSpace {
        free_bytes: u64,
        to_free: u64,
        volume: Option<String>,
    },
    // Warnings.
    /// Clips whose media is offline render the offline slate (video) or
    /// silence (audio); the targets list them.
    OfflineMedia {
        clips: usize,
    },
    FontsSubstituted {
        families: Vec<String>,
    },
    /// Every visual track is hidden: only the canvas background shows.
    AllVisualTracksHidden,
    /// The estimate is above the free space (EX-037).
    SizeAboveFreeSpace {
        estimate: u64,
        free_bytes: u64,
    },
    // Info.
    /// The output rate differs from the project rate (EX-026).
    FrameRateConversion {
        from: FrameRate,
        to: FrameRate,
    },
    /// The output's short side is larger than the largest source's (EX-024).
    Upscaling {
        output: u32,
        source: u32,
    },
    /// Hidden or muted tracks contain clips (TL-008); the targets list them.
    HiddenOrMutedTracks {
        tracks: usize,
    },
}

impl IssueCode {
    /// How bad it is.
    pub fn severity(&self) -> Severity {
        use IssueCode::*;
        match self {
            EmptyTimeline
            | EmptyFileName
            | FileNameTooLong { .. }
            | NoFolder
            | FolderMissing
            | FolderNotWritable
            | DriveDisconnected { .. }
            | TargetIsSource { .. }
            | OutputTooLarge { .. }
            | NoEncoder
            | IncompatibleFormat
            | UnsupportedFrameRate { .. }
            | AudioBitrateUnsupported { .. }
            | AudioSampleRateUnsupported { .. }
            | LowDiskSpace { .. } => Severity::Problem,
            OfflineMedia { .. }
            | FontsSubstituted { .. }
            | AllVisualTracksHidden
            | SizeAboveFreeSpace { .. } => Severity::Warning,
            FrameRateConversion { .. } | Upscaling { .. } | HiddenOrMutedTracks { .. } => {
                Severity::Info
            }
        }
    }

    /// A stable id, used as the message id of the localized text.
    pub fn id(&self) -> &'static str {
        use IssueCode::*;
        match self {
            EmptyTimeline => "export-empty-timeline",
            EmptyFileName => "export-empty-file-name",
            FileNameTooLong { .. } => "export-file-name-too-long",
            NoFolder => "export-no-folder",
            FolderMissing => "export-folder-missing",
            FolderNotWritable => "export-folder-not-writable",
            DriveDisconnected { .. } => "export-drive-disconnected",
            TargetIsSource { .. } => "export-target-is-source",
            OutputTooLarge { .. } => "export-output-too-large",
            NoEncoder => "export-no-encoder",
            IncompatibleFormat => "export-incompatible-format",
            UnsupportedFrameRate { .. } => "export-unsupported-frame-rate",
            AudioBitrateUnsupported { .. } => "export-audio-bitrate-unsupported",
            AudioSampleRateUnsupported { .. } => "export-audio-sample-rate-unsupported",
            LowDiskSpace { .. } => "export-low-disk-space",
            OfflineMedia { .. } => "export-offline-media",
            FontsSubstituted { .. } => "export-fonts-substituted",
            AllVisualTracksHidden => "export-all-visual-tracks-hidden",
            SizeAboveFreeSpace { .. } => "export-size-above-free-space",
            FrameRateConversion { .. } => "export-frame-rate-conversion",
            Upscaling { .. } => "export-upscaling",
            HiddenOrMutedTracks { .. } => "export-hidden-or-muted-tracks",
        }
    }
}

/// One preflight result line (EX-036).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub severity: Severity,
    pub code: IssueCode,
    /// What clicking the issue selects; empty when it isn't about items.
    pub targets: Vec<IssueTarget>,
}

impl Issue {
    fn new(code: IssueCode) -> Issue {
        Issue {
            severity: code.severity(),
            code,
            targets: Vec::new(),
        }
    }

    fn with_targets(code: IssueCode, targets: Vec<IssueTarget>) -> Issue {
        Issue {
            targets,
            ..Issue::new(code)
        }
    }
}

/// Whether any issue blocks the export.
pub fn has_problems(issues: &[Issue]) -> bool {
    issues.iter().any(|i| i.severity == Severity::Problem)
}

/// Runs every MVP check (EX-036) for exporting sequence `seq` of `doc` with
/// `settings`. Problems come first, then warnings, then info; within a
/// severity the order is fixed, so the list is stable for the same input.
pub fn validate(
    settings: &ExportSettings,
    doc: &Document,
    seq: SeqId,
    env: &PreflightEnv,
) -> Vec<Issue> {
    let mut out = Vec::new();
    let project = doc.settings();
    let sequence = doc.sequence(seq);
    let duration = sequence.map_or(Tick::ZERO, Sequence::duration);
    let canvas = (project.width, project.height);
    let (width, height) = settings.output_dimensions(canvas);
    let rate = settings.output_rate(project.frame_rate);

    // Problems.
    if !duration.is_positive() {
        out.push(Issue::new(IssueCode::EmptyTimeline));
    }
    let stem = sanitize_file_stem(&settings.file.name);
    if stem.is_empty() {
        out.push(Issue::new(IssueCode::EmptyFileName));
    } else {
        let chars = stem.chars().count();
        let bytes = stem.len() + 1 + settings.video.container.extension().len();
        if chars > MAX_STEM_CHARS || bytes > MAX_FILE_NAME_BYTES {
            out.push(Issue::new(IssueCode::FileNameTooLong { chars, bytes }));
        }
    }
    match &settings.file.folder {
        None => out.push(Issue::new(IssueCode::NoFolder)),
        Some(folder) => {
            match &env.folder {
                FolderStatus::Unknown | FolderStatus::Writable => {}
                FolderStatus::Missing => out.push(Issue::new(IssueCode::FolderMissing)),
                FolderStatus::NotWritable => out.push(Issue::new(IssueCode::FolderNotWritable)),
                FolderStatus::Disconnected { drive } => {
                    out.push(Issue::new(IssueCode::DriveDisconnected {
                        drive: drive.clone(),
                    }));
                }
            }
            if !stem.is_empty() {
                let target = output_path(folder, &stem, settings.video.container);
                if let Some(media) = source_at(doc, &target) {
                    out.push(Issue::with_targets(
                        IssueCode::TargetIsSource { media },
                        vec![IssueTarget::Media(media)],
                    ));
                }
            }
        }
    }
    let max = env
        .max_texture_side
        .map_or(MAX_OUTPUT_SIDE, |gpu| gpu.min(MAX_OUTPUT_SIDE));
    if width > max || height > max {
        out.push(Issue::new(IssueCode::OutputTooLarge { width, height, max }));
    }
    if !env.encoder_available {
        out.push(Issue::new(IssueCode::NoEncoder));
    }
    if !compatible(settings.video.codec, settings.video.container) {
        out.push(Issue::new(IssueCode::IncompatibleFormat));
    }
    if !settings.video.frame_rate.is_offered(project.frame_rate) {
        out.push(Issue::new(IssueCode::UnsupportedFrameRate { rate }));
    }
    if settings.audio.include {
        if !AAC_KBPS_CHOICES.contains(&settings.audio.kbps) {
            out.push(Issue::new(IssueCode::AudioBitrateUnsupported {
                kbps: settings.audio.kbps,
            }));
        }
        let hz = settings.audio.sample_rate.hz();
        if !AUDIO_SAMPLE_RATES.contains(&hz) {
            out.push(Issue::new(IssueCode::AudioSampleRateUnsupported { hz }));
        }
    }
    let estimate = estimate_size(settings, project, duration).bytes;
    if let Some(free) = env.free_bytes
        && free < MIN_FREE_BYTES
    {
        out.push(Issue::new(IssueCode::LowDiskSpace {
            free_bytes: free,
            to_free: MIN_FREE_BYTES.max(estimate) - free,
            volume: env.volume.clone(),
        }));
    }

    // Warnings.
    if let Some(seq) = sequence {
        let offline: Vec<IssueTarget> = seq
            .tracks()
            .flat_map(|(_, t)| t.clips())
            .filter(|c| {
                c.content
                    .asset()
                    .is_some_and(|a| env.offline_media.contains(&a))
            })
            .map(|c| IssueTarget::Clip(c.id))
            .collect();
        if !offline.is_empty() {
            out.push(Issue::with_targets(
                IssueCode::OfflineMedia {
                    clips: offline.len(),
                },
                offline,
            ));
        }
    }
    if !env.substituted_fonts.is_empty() {
        out.push(Issue::new(IssueCode::FontsSubstituted {
            families: env.substituted_fonts.clone(),
        }));
    }
    if let Some(seq) = sequence
        && seq.visual_tracks().iter().all(|t| t.header.flags.hidden)
        && seq.visual_tracks().iter().any(|t| !t.is_empty())
    {
        out.push(Issue::new(IssueCode::AllVisualTracksHidden));
    }
    if let Some(free) = env.free_bytes
        && free >= MIN_FREE_BYTES
        && estimate > free
    {
        out.push(Issue::new(IssueCode::SizeAboveFreeSpace {
            estimate,
            free_bytes: free,
        }));
    }

    // Info.
    if rate != project.frame_rate {
        out.push(Issue::new(IssueCode::FrameRateConversion {
            from: project.frame_rate,
            to: rate,
        }));
    }
    if let Some(source) = sequence.and_then(|s| largest_source_short_side(doc, s)) {
        let output = width.min(height);
        if output > source {
            out.push(Issue::new(IssueCode::Upscaling { output, source }));
        }
    }
    if let Some(seq) = sequence {
        let tracks: Vec<IssueTarget> = seq
            .tracks()
            .filter(|(_, t)| (t.header.flags.hidden || t.header.flags.muted) && !t.is_empty())
            .map(|(_, t)| IssueTarget::Track(t.id))
            .collect();
        if !tracks.is_empty() {
            out.push(Issue::with_targets(
                IssueCode::HiddenOrMutedTracks {
                    tracks: tracks.len(),
                },
                tracks,
            ));
        }
    }
    // Pushed in severity order already; a stable sort keeps it so if the
    // checks above are ever reordered.
    out.sort_by_key(|i| i.severity);
    out
}

/// Whether anything in `seq` makes sound: an enabled clip on an unmuted
/// track whose volume isn't −∞ dB, with its own sound on, not muted and not
/// at −∞ dB throughout (EX-031; with volume keyframes, some key is above
/// −∞).
pub(crate) fn has_audible_clips(seq: &Sequence) -> bool {
    seq.tracks()
        .filter(|(_, t)| !t.header.flags.muted && !t.mix.gain.is_silent())
        .flat_map(|(_, t)| t.clips())
        .any(|c| {
            c.enabled
                && c.audio.active().is_some_and(|a| {
                    let keys = a.gain.keys();
                    let silent = if keys.is_empty() {
                        a.gain.static_value().is_silent()
                    } else {
                        keys.iter().all(|k| k.value.is_silent())
                    };
                    !a.muted && !silent
                })
        })
}

/// The library item whose file is `target`, compared without regard to
/// case: on the case-insensitive file systems of macOS and Windows that is
/// the same file, and elsewhere refusing a name that differs only in case
/// costs a rename at worst.
fn source_at(doc: &Document, target: &std::path::Path) -> Option<MediaId> {
    let target = normalized(target);
    doc.library()
        .items()
        .find(|item| normalized(std::path::Path::new(&item.path.absolute)) == target)
        .map(|item| item.id)
}

/// `path` lowercased, with `.` components and doubled separators gone.
fn normalized(path: &std::path::Path) -> String {
    let clean: PathBuf = path.components().collect();
    clean.to_string_lossy().to_lowercase()
}

/// The shortest side of the largest picture the sequence shows: video
/// streams and stills of its visual clips.
fn largest_source_short_side(doc: &Document, seq: &Sequence) -> Option<u32> {
    seq.visual_tracks()
        .iter()
        .flat_map(|t| t.clips())
        .filter_map(|c| {
            let item = doc.library().get(c.content.asset()?)?;
            match &c.content {
                ClipContent::Media {
                    kind: MediaKind::Video,
                    ..
                } => {
                    let info = &item.info;
                    let stream = info
                        .default_video
                        .and_then(|i| info.video.iter().find(|v| v.index == i))
                        .or_else(|| info.video.first())?;
                    Some(stream.width.min(stream.height))
                }
                ClipContent::Image { .. } => item.info.still.map(|(w, h)| w.min(h)),
                _ => None,
            }
        })
        .max()
}
