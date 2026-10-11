//! Error types for media work (`thiserror`; the app maps them to user text,
//! PM-037).

use std::fmt;
use std::io;

use freecut_timeline::time::{Tick, TimeError};

/// A library version as FFmpeg packs it (`major << 16 | minor << 8 | micro`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LibVersion {
    /// Incompatible ABI changes.
    pub major: u32,
    /// Additions (new fields, new enum values).
    pub minor: u32,
    /// Fixes.
    pub micro: u32,
}

impl LibVersion {
    /// Unpacks `AV_VERSION_INT`-style integers.
    pub const fn from_int(v: u32) -> LibVersion {
        LibVersion {
            major: v >> 16,
            minor: (v >> 8) & 0xff,
            micro: v & 0xff,
        }
    }
}

impl fmt::Display for LibVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.micro)
    }
}

/// Why media support could not start ([`crate::init`]).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MediaError {
    /// The FFmpeg library loaded at run time has another major version than
    /// the headers the bindings were generated from, or an older minor.
    /// Reading struct fields through such bindings is undefined behaviour,
    /// so media stays off. (A newer minor is accepted.)
    #[error(
        "FFmpeg {library} {runtime} does not match the {built_major}.{built_minor} headers \
         this build was compiled against; rebuild FreeCut against the installed FFmpeg"
    )]
    VersionMismatch {
        /// `"avcodec"`, `"avformat"`, `"avutil"`, `"swscale"`,
        /// `"swresample"` or `"avfilter"`.
        library: &'static str,
        /// Major version of the headers the bindings came from.
        built_major: u32,
        /// Minor version of those headers.
        built_minor: u32,
        /// The version loaded at run time.
        runtime: LibVersion,
    },
}

/// What kind of non-media file was recognized (PM-036 hints).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NotMediaKind {
    /// Nothing FFmpeg recognizes as audio, video or an image.
    Unrecognized,
    /// A zip or similar archive ("unzip first").
    Archive,
    /// A playlist such as `.m3u`.
    Playlist,
    /// A Windows shortcut (`.lnk`).
    Shortcut,
    /// A PDF document.
    Document,
    /// A FreeCut project file ("open it instead").
    Project,
}

impl fmt::Display for NotMediaKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            NotMediaKind::Unrecognized => "unrecognized content",
            NotMediaKind::Archive => "an archive",
            NotMediaKind::Playlist => "a playlist",
            NotMediaKind::Shortcut => "a shortcut",
            NotMediaKind::Document => "a document",
            NotMediaKind::Project => "a FreeCut project",
        })
    }
}

/// Why a file could not be probed (PM-037).
///
/// A missing file or a permission problem is [`ProbeError::Io`] with the
/// matching [`io::ErrorKind`]; see [`ProbeError::is_not_found`].
#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    /// The file can't be read (missing, permission, I/O failure).
    #[error("cannot read the file: {0}")]
    Io(#[from] io::Error),
    /// The file has 0 bytes.
    #[error("the file is empty")]
    Empty,
    /// The content is not audio, video or an image.
    #[error("not a media file ({kind})")]
    NotMedia {
        /// What was recognized instead, if anything.
        kind: NotMediaKind,
    },
    /// A media container without audio or video streams.
    #[error("no video or audio inside")]
    NoStreams,
    /// Every audio/video stream uses a codec this FFmpeg build can't decode.
    #[error("unsupported codec {codec}")]
    UnsupportedCodec {
        /// FFmpeg's codec name, or the container's FourCC when FFmpeg
        /// doesn't know the codec at all.
        codec: String,
    },
    /// The container opened but its content doesn't decode.
    #[error("damaged file: {detail}")]
    Corrupt {
        /// FFmpeg's or our description, for "Copy report".
        detail: String,
    },
    /// The container's index is missing, typically a recording that was
    /// never finalized (MP4/MOV without a `moov` box).
    #[error("the recording looks unfinished (the file has no index)")]
    Incomplete,
    /// The path names something that isn't a regular file (a directory, a
    /// device). Non-UTF-8 paths are supported and never produce this.
    #[error("unsupported path: {reason}")]
    UnsupportedPath {
        /// Why, in plain words.
        reason: &'static str,
    },
    /// The [`crate::CancelToken`] was cancelled.
    #[error("cancelled")]
    Cancelled,
    /// The token's deadline passed (PM-030's 30 s probe timeout).
    #[error("timed out reading the file")]
    Timeout,
    /// FFmpeg could not be initialized.
    #[error(transparent)]
    Media(#[from] MediaError),
}

impl ProbeError {
    /// True when the file doesn't exist (PM-037 "File not found").
    pub fn is_not_found(&self) -> bool {
        matches!(self, ProbeError::Io(e) if e.kind() == io::ErrorKind::NotFound)
    }

    /// True when the OS refused access (PM-037 "Permission denied").
    pub fn is_permission_denied(&self) -> bool {
        matches!(self, ProbeError::Io(e) if e.kind() == io::ErrorKind::PermissionDenied)
    }
}

/// The kind of stream a reader was asked to open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StreamKind {
    /// A picture stream.
    Video,
    /// A sound stream.
    Audio,
}

impl fmt::Display for StreamKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            StreamKind::Video => "video",
            StreamKind::Audio => "audio",
        })
    }
}

/// Why decoding failed.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    /// The file could not be opened; the inner error says why.
    #[error("cannot open the media: {0}")]
    Open(#[from] ProbeError),
    /// The stream index doesn't name a decodable stream of that kind.
    #[error("stream {index} is not a decodable {kind} stream")]
    NoSuchStream {
        /// The container stream index asked for.
        index: usize,
        /// The kind of stream the reader needs.
        kind: StreamKind,
    },
    /// Nothing decodable was found around the requested time.
    #[error("no frame could be decoded near {at}")]
    Corrupt {
        /// The requested media time.
        at: Tick,
    },
    /// An FFmpeg call failed.
    #[error("{op} failed: {msg} (code {code})")]
    Ffmpeg {
        /// The FFmpeg function or step that failed.
        op: &'static str,
        /// The raw `AVERROR` code (0 when not from FFmpeg).
        code: i32,
        /// FFmpeg's description of the error.
        msg: String,
    },
    /// Reading the file failed.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    /// A time computation overflowed (absurd timestamps).
    #[error(transparent)]
    Time(#[from] TimeError),
    /// An argument was out of range.
    #[error("invalid argument: {0}")]
    InvalidArgument(&'static str),
    /// The [`crate::CancelToken`] fired. Control flow, never shown to users.
    #[error("cancelled")]
    Cancelled,
    /// The token's deadline passed.
    #[error("timed out")]
    Timeout,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_unpack() {
        let v = LibVersion::from_int((62 << 16) | (28 << 8) | 101);
        assert_eq!(
            v,
            LibVersion {
                major: 62,
                minor: 28,
                micro: 101
            }
        );
        assert_eq!(v.to_string(), "62.28.101");
    }

    #[test]
    fn io_kinds_are_recognized() {
        let e = ProbeError::from(io::Error::from(io::ErrorKind::NotFound));
        assert!(e.is_not_found());
        assert!(!e.is_permission_denied());
        assert!(!ProbeError::Empty.is_not_found());
    }
}
