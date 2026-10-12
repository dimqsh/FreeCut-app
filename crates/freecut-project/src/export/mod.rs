//! Export settings and their pure helpers (EX-021, EX-023, EX-024, EX-026,
//! EX-027, EX-031, EX-036, EX-037; README M4).
//!
//! - [`ExportSettings`]: what the export dialog edits. The last settings
//!   used are saved in the project file (`"export"`, see
//!   [`crate::SaveRequest::export`] and [`crate::Opened::export`]) when an
//!   export starts; they are not undoable edits.
//! - [`output_dimensions`], [`recommended_kbps`] / [`tier_kbps`],
//!   [`estimate_size`]: the numbers an export runs with.
//! - [`sanitize_file_stem`], [`resolve_conflict`], [`target_path`]: the
//!   output file name and the Keep-both rule.
//! - [`validate`]: the preflight checks, from the settings, the document and
//!   facts the app gathered off the UI thread ([`PreflightEnv`]).
//!
//! Everything here is pure: no file system, no clock, no FFmpeg. The app
//! maps the resolved numbers onto `freecut_media::encode::EncodeSpec`.

mod bitrate;
mod names;
mod preflight;
mod settings;

pub use bitrate::{MIN_VIDEO_KBPS, SizeEstimate, estimate_size, recommended_kbps, tier_kbps};
pub use names::{
    ConflictChoice, MAX_CONFLICT_SUFFIX, MAX_FILE_NAME_BYTES, MAX_STEM_CHARS, UNTITLED,
    output_path, resolve_conflict, sanitize_file_stem, target_path,
};
pub use preflight::{
    FolderStatus, Issue, IssueCode, IssueTarget, MIN_FREE_BYTES, PreflightEnv, Severity,
    has_problems, validate,
};
pub use settings::{
    AAC_KBPS_CHOICES, AUDIO_SAMPLE_RATES, AudioExportSettings, Container, DEFAULT_AAC_KBPS,
    DEFAULT_SAMPLE_RATE, ExportSettings, FRAME_RATE_CHOICES, FileTarget, FrameRateChoice,
    MAX_OUTPUT_SIDE, OutputSize, QualityTier, ShortSide, VideoCodec, VideoSettings, compatible,
    output_dimensions,
};
