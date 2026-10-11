//! Why an export encode failed (`thiserror`; the app maps these to the
//! EX-022 failure codes).

use std::io;

use crate::error::{DecodeError, MediaError};

/// Why an [`EncodeSession`](super::EncodeSession) or
/// [`verify_output`](super::verify_output) failed.
#[derive(Debug, thiserror::Error)]
pub enum EncodeError {
    /// The [`EncodeSpec`](super::EncodeSpec) can't be encoded (odd or zero
    /// size, unsupported audio settings, …). Checked before any file is
    /// created.
    #[error("invalid export settings: {0}")]
    InvalidSpec(&'static str),
    /// A pushed frame or audio block doesn't fit the session (wrong buffer
    /// size, audio without an audio stream, …).
    #[error("invalid encoder input: {0}")]
    InvalidInput(String),
    /// Frames must be pushed in order without gaps.
    #[error("frame {got} was pushed but frame {expected} was expected next")]
    OutOfOrder {
        /// The frame index the session needs next.
        expected: u64,
        /// The frame index that was pushed.
        got: u64,
    },
    /// The linked FFmpeg has no such encoder (EX-029: some distribution
    /// builds lack libx264).
    #[error("this FFmpeg build has no {0} encoder")]
    EncoderUnavailable(&'static str),
    /// An encoder or the muxer did not recognize an option we passed. Never
    /// ignored: a misspelled option would silently change the output.
    #[error("the {component} did not accept the option `{key}`")]
    UnknownOption {
        /// `"video encoder"`, `"audio encoder"` or `"muxer"`.
        component: &'static str,
        /// The rejected option key.
        key: String,
    },
    /// The output path can't be handed to FFmpeg (not valid UTF-8, or it
    /// contains a NUL character).
    #[error("the output path can't be used: {0}")]
    UnsupportedPath(&'static str),
    /// The destination volume ran out of space (EX-022 `disk-full`).
    #[error("not enough space on the destination volume")]
    DiskFull,
    /// The file system's per-file size limit was hit (EX-022
    /// `file-too-large`).
    #[error("the file is too large for the destination file system")]
    FileTooLarge,
    /// An FFmpeg call failed (EX-022 `encoder-failed`).
    #[error("{op} failed: {msg} (code {code})")]
    Ffmpeg {
        /// The FFmpeg function or step that failed.
        op: &'static str,
        /// The raw `AVERROR` code.
        code: i32,
        /// FFmpeg's description of the error.
        msg: String,
    },
    /// A file operation failed (deleting a partial file, reading it back).
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    /// The finished file could not be read back for verification.
    #[error("the written file can't be read back: {0}")]
    Reread(#[from] DecodeError),
    /// The finished file doesn't contain what was encoded (P6: success
    /// needs a passing verification probe).
    #[error("the written file failed verification: {0}")]
    Verify(String),
    /// The [`CancelToken`](crate::CancelToken) fired. The partial file has
    /// been deleted. Control flow, never shown to users.
    #[error("cancelled")]
    Cancelled,
    /// FFmpeg could not be initialized.
    #[error(transparent)]
    Media(#[from] MediaError),
}

impl EncodeError {
    /// Wraps an FFmpeg error code, recognizing a full disk and the file size
    /// limit (`ENOSPC` / `EFBIG`, wherever FFmpeg reports them).
    #[cfg_attr(not(feature = "ffmpeg"), allow(dead_code))]
    pub(crate) fn from_av(op: &'static str, code: i32, msg: String) -> EncodeError {
        // `AVERROR(e)` is `-e` with C `errno` values, and `EFBIG` (27) and
        // `ENOSPC` (28) are the same in glibc, macOS and the Windows CRT
        // FFmpeg is built against. (`io::Error::from_raw_os_error` would read
        // them as Win32 error codes on Windows.)
        const AVERROR_EFBIG: i32 = -27;
        const AVERROR_ENOSPC: i32 = -28;
        match code {
            AVERROR_ENOSPC => EncodeError::DiskFull,
            AVERROR_EFBIG => EncodeError::FileTooLarge,
            _ => EncodeError::Ffmpeg { op, code, msg },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_errors_are_recognized() {
        assert!(matches!(
            EncodeError::from_av("x", -28, String::new()),
            EncodeError::DiskFull
        ));
        assert!(matches!(
            EncodeError::from_av("x", -27, String::new()),
            EncodeError::FileTooLarge
        ));
        assert!(matches!(
            EncodeError::from_av("x", -22, "Invalid argument".into()),
            EncodeError::Ffmpeg { code: -22, .. }
        ));
        // AVERROR_EOF-style tags are not errno values.
        assert!(matches!(
            EncodeError::from_av("x", -0x2046_4f45, String::new()),
            EncodeError::Ffmpeg { .. }
        ));
    }
}
