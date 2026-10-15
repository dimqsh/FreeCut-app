//! One-time FFmpeg setup and the runtime capability table (PM-036).
//!
//! [`init`] runs once per process (later calls return the same result) and
//! is safe to call from any thread. It refuses to enable media when an
//! FFmpeg library loaded at run time has a different major version than the
//! headers the bindings were generated from, or an older minor: the bindings
//! read struct fields at fixed offsets, which an older minor may not have
//! (README §5.7). A newer minor is accepted (distribution FFmpeg updates):
//! within a major, FFmpeg only appends fields, and the enum values a newer
//! minor may add are never read as Rust enums — the crate reads enum-typed
//! fields as integers in its `av` module (codec ids, media types, pixel and
//! sample formats, side-data types) and opens decoders there too.
//!
//! FFmpeg's own logging is silenced unless `FREECUT_FFMPEG_LOG` is set to
//! `error`, `warning`, `info`, `verbose` or `debug` (libraries never print,
//! README §5.6).

use std::sync::OnceLock;

use ffmpeg_next::codec::Id;
use ffmpeg_next::ffi;
use ffmpeg_next::log::Level;

use crate::error::{LibVersion, MediaError};

/// Whether one codec is available in the linked FFmpeg.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecSupport {
    /// FreeCut's name for the codec (`"h264"`, `"aac"`, `"libx264"`, …).
    pub name: &'static str,
    /// The FFmpeg implementation that would be used, when available
    /// (for example `"libdav1d"` for AV1 decoding).
    pub implementation: Option<String>,
}

impl CodecSupport {
    /// True when the linked FFmpeg has an implementation.
    pub fn is_available(&self) -> bool {
        self.implementation.is_some()
    }
}

/// What the linked FFmpeg build can do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capabilities {
    /// Runtime versions of the libraries, by library name.
    pub versions: Vec<(&'static str, LibVersion)>,
    /// Decoders FreeCut cares about, available or not.
    pub decoders: Vec<CodecSupport>,
    /// Encoders FreeCut cares about, available or not.
    pub encoders: Vec<CodecSupport>,
}

impl Capabilities {
    /// True when the decoder named `name` (as listed in `decoders`) exists.
    pub fn has_decoder(&self, name: &str) -> bool {
        self.decoders
            .iter()
            .any(|c| c.name == name && c.is_available())
    }

    /// True when the encoder named `name` (as listed in `encoders`) exists.
    pub fn has_encoder(&self, name: &str) -> bool {
        self.encoders
            .iter()
            .any(|c| c.name == name && c.is_available())
    }

    /// The runtime version of `library` (`"avcodec"`, `"avformat"`, …).
    pub fn version(&self, library: &str) -> Option<LibVersion> {
        self.versions
            .iter()
            .find(|(name, _)| *name == library)
            .map(|(_, v)| *v)
    }
}

/// Decoders looked up by codec id, so any implementation counts.
const DECODERS: [(&str, Id); 27] = [
    ("h264", Id::H264),
    ("hevc", Id::HEVC),
    ("vp8", Id::VP8),
    ("vp9", Id::VP9),
    ("av1", Id::AV1),
    ("prores", Id::PRORES),
    ("mpeg2video", Id::MPEG2VIDEO),
    ("mpeg4", Id::MPEG4),
    ("mjpeg", Id::MJPEG),
    ("dnxhd", Id::DNXHD),
    ("ffv1", Id::FFV1),
    ("qtrle", Id::QTRLE),
    ("png", Id::PNG),
    ("gif", Id::GIF),
    ("webp", Id::WEBP),
    ("aac", Id::AAC),
    ("mp3", Id::MP3),
    ("opus", Id::OPUS),
    ("vorbis", Id::VORBIS),
    ("flac", Id::FLAC),
    ("alac", Id::ALAC),
    ("ac3", Id::AC3),
    ("eac3", Id::EAC3),
    ("pcm_s16le", Id::PCM_S16LE),
    ("pcm_s24le", Id::PCM_S24LE),
    ("pcm_f32le", Id::PCM_F32LE),
    ("pcm_s16be", Id::PCM_S16BE),
];

/// Decoders that matter by implementation name (VP8/VP9 alpha, L9).
const NAMED_DECODERS: [&str; 2] = ["libvpx", "libvpx-vp9"];

/// Encoders looked up by implementation name.
const ENCODERS: [&str; 16] = [
    "libx264",
    "libx265",
    "h264_videotoolbox",
    "hevc_videotoolbox",
    "libsvtav1",
    "libvpx-vp9",
    "prores_ks",
    "mjpeg",
    "png",
    "gif",
    "aac",
    "libopus",
    "flac",
    "pcm_s16le",
    "ffv1",
    "mpeg4",
];

/// Checks one library's runtime version against the headers we were built
/// with: same major, same or newer minor (see the module docs).
pub(crate) fn check_version(
    library: &'static str,
    built_major: i32,
    built_minor: i32,
    runtime: u32,
) -> Result<LibVersion, MediaError> {
    let v = LibVersion::from_int(runtime);
    let compatible = i64::from(v.major) == i64::from(built_major)
        && i64::from(v.minor) >= i64::from(built_minor);
    if compatible {
        Ok(v)
    } else {
        Err(MediaError::VersionMismatch {
            library,
            built_major: u32::try_from(built_major).unwrap_or(0),
            built_minor: u32::try_from(built_minor).unwrap_or(0),
            runtime: v,
        })
    }
}

fn log_level_from_env() -> Level {
    match std::env::var("FREECUT_FFMPEG_LOG").as_deref() {
        Ok("error") => Level::Error,
        Ok("warning") => Level::Warning,
        Ok("info") => Level::Info,
        Ok("verbose") => Level::Verbose,
        Ok("debug") => Level::Debug,
        _ => Level::Quiet,
    }
}

fn initialize() -> Result<Capabilities, MediaError> {
    let libraries = [
        (
            "avcodec",
            ffi::LIBAVCODEC_VERSION_MAJOR,
            ffi::LIBAVCODEC_VERSION_MINOR,
            ffmpeg_next::codec::version(),
        ),
        (
            "avformat",
            ffi::LIBAVFORMAT_VERSION_MAJOR,
            ffi::LIBAVFORMAT_VERSION_MINOR,
            ffmpeg_next::format::version(),
        ),
        (
            "avutil",
            ffi::LIBAVUTIL_VERSION_MAJOR,
            ffi::LIBAVUTIL_VERSION_MINOR,
            ffmpeg_next::util::version(),
        ),
        (
            "swscale",
            ffi::LIBSWSCALE_VERSION_MAJOR,
            ffi::LIBSWSCALE_VERSION_MINOR,
            ffmpeg_next::software::scaling::version(),
        ),
        (
            "swresample",
            ffi::LIBSWRESAMPLE_VERSION_MAJOR,
            ffi::LIBSWRESAMPLE_VERSION_MINOR,
            ffmpeg_next::software::resampling::version(),
        ),
    ];
    let mut versions = Vec::with_capacity(libraries.len());
    for (name, major, minor, runtime) in libraries {
        versions.push((name, check_version(name, major, minor, runtime)?));
    }

    // Only registers error strings with FFmpeg ≥ 5; cannot fail.
    if let Err(e) = ffmpeg_next::init() {
        tracing::warn!("ffmpeg_next::init reported {e}");
    }
    ffmpeg_next::log::set_level(log_level_from_env());

    let mut decoders: Vec<CodecSupport> = DECODERS
        .iter()
        .map(|&(name, id)| CodecSupport {
            name,
            implementation: ffmpeg_next::decoder::find(id).map(|c| c.name().to_owned()),
        })
        .collect();
    decoders.extend(NAMED_DECODERS.iter().map(|&name| CodecSupport {
        name,
        implementation: ffmpeg_next::decoder::find_by_name(name).map(|c| c.name().to_owned()),
    }));
    let encoders = ENCODERS
        .iter()
        .map(|&name| CodecSupport {
            name,
            implementation: ffmpeg_next::encoder::find_by_name(name).map(|c| c.name().to_owned()),
        })
        .collect();

    tracing::debug!(?versions, "FFmpeg initialized");
    Ok(Capabilities {
        versions,
        decoders,
        encoders,
    })
}

static INIT: OnceLock<Result<Capabilities, MediaError>> = OnceLock::new();

/// Initializes FFmpeg once and reports what the linked build supports.
///
/// Idempotent and thread-safe: the first call does the work (version check,
/// logging setup, capability table) and every call returns its result.
/// Every other entry point of this crate calls it implicitly, so calling it
/// first is only needed to show the capability table or fail early.
pub fn init() -> Result<Capabilities, MediaError> {
    INIT.get_or_init(initialize).clone()
}

/// [`init`] without cloning the table: `Ok` when media can be used.
pub(crate) fn ensure_init() -> Result<(), MediaError> {
    match INIT.get_or_init(initialize) {
        Ok(_) => Ok(()),
        Err(e) => Err(e.clone()),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)] // test code
    use super::*;

    const fn int(major: u32, minor: u32, micro: u32) -> u32 {
        (major << 16) | (minor << 8) | micro
    }

    #[test]
    fn version_check_requires_same_major_and_no_older_minor() {
        assert!(check_version("avcodec", 62, 28, int(62, 28, 101)).is_ok());
        assert!(check_version("avcodec", 62, 28, int(62, 28, 0)).is_ok());
        let older = check_version("avcodec", 62, 28, int(62, 27, 100)).unwrap_err();
        assert!(matches!(
            older,
            MediaError::VersionMismatch {
                library: "avcodec",
                built_major: 62,
                built_minor: 28,
                ..
            }
        ));
        assert!(check_version("avcodec", 62, 28, int(61, 28, 100)).is_err());
        assert!(check_version("avcodec", 62, 28, int(63, 0, 100)).is_err());
    }

    /// M2-MEDIA-12: a distribution update to a newer minor keeps media on.
    #[test]
    fn a_newer_runtime_minor_is_accepted() {
        let newer = int(62, 29, 100);
        assert!(
            check_version("avcodec", 62, 28, newer).is_ok(),
            "FFmpeg 62.29 runtime refused against 62.28 headers"
        );
        assert!(check_version("avformat", 62, 3, int(62, 12, 0)).is_ok());
    }

    #[test]
    fn init_is_idempotent_and_finds_core_codecs() {
        let a = init().unwrap();
        let b = init().unwrap();
        assert_eq!(a, b);
        assert_eq!(
            a.version("avcodec").map(|v| v.major),
            Some(ffi::LIBAVCODEC_VERSION_MAJOR)
        );
        assert!(a.has_decoder("h264"));
        assert!(a.has_decoder("aac"));
        assert!(a.has_encoder("aac"));
        assert!(!a.has_decoder("no-such-codec"));
    }
}
