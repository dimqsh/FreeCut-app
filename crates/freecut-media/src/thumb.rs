//! Posters (PM-070): one representative frame per media item, sized for
//! tiles, with display geometry applied.
//!
//! The poster time is the first frame for clips shorter than one second and
//! `min(1 s, 10 % of the duration)` otherwise (FreeCut default; it avoids
//! black first frames). If that frame can't be decoded the first frame is
//! tried. The picture is scaled to square pixels (sample aspect ratio), fit
//! so its longer side is at most `max_side` (never upscaled), then mirrored
//! and rotated for display. Stills are decoded with the `image` crate with
//! their EXIF orientation applied; FFmpeg is the fallback for formats it
//! doesn't read.

use std::path::Path;

use freecut_timeline::model::VideoStreamInfo;
use freecut_timeline::time::{TICKS_PER_SECOND, Tick};

use crate::cancel::CancelToken;
use crate::color::HdrTransfer;
use crate::error::{DecodeError, ProbeError, StreamKind};
use crate::geometry::{fit_within, orient_rgba, square_pixel_size};
use crate::probe::{PROBE_TIMEOUT, probe_detail};
use crate::video::{SeekMode, VideoDecoder};

/// An 8-bit RGBA picture, rows top to bottom, straight alpha.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes.
    pub rgba: Vec<u8>,
    /// The HDR transfer (PQ or HLG) of the video the picture came from: it
    /// was converted to SDR, as every decode of it is (CL-002). `None` for
    /// SDR sources and stills.
    pub hdr: Option<HdrTransfer>,
}

/// The poster time for a video stream (see the module docs).
pub(crate) fn poster_time(v: &VideoStreamInfo) -> Tick {
    let second = Tick::from_raw(TICKS_PER_SECOND);
    let offset = if v.duration < second {
        Tick::ZERO
    } else {
        Tick::from_raw(v.duration.raw() / 10).min(second)
    };
    v.start.checked_add(offset).unwrap_or(v.start)
}

/// A poster of `path` whose longer side is at most `max_side` pixels.
pub fn poster(path: &Path, max_side: u32) -> Result<RgbaImage, DecodeError> {
    if max_side == 0 {
        return Err(DecodeError::InvalidArgument("max_side must be positive"));
    }
    let cancel = CancelToken::new().with_timeout(PROBE_TIMEOUT);
    let (info, default_video, hdr) = probe_detail(path, &cancel)?;
    if info.still.is_some() {
        match still_poster(path, max_side) {
            Ok(img) => return Ok(img),
            Err(e) => tracing::debug!("image crate could not read the still, using FFmpeg: {e}"),
        }
    }
    let Some(index) = default_video else {
        return Err(DecodeError::NoSuchStream {
            index: 0,
            kind: StreamKind::Video,
        });
    };
    let Some(v) = info.video.iter().find(|v| v.index as usize == index) else {
        return Err(DecodeError::Open(ProbeError::NoStreams));
    };
    let (sw, sh) = square_pixel_size(v.width, v.height, v.sar);
    let (tw, th) = fit_within(sw, sh, max_side);
    let mut dec = VideoDecoder::open(path, index)?;
    let t = poster_time(v);
    let frame = match dec.frame_at_size(t, SeekMode::Exact, tw, th) {
        Ok(f) => f,
        Err(DecodeError::Cancelled | DecodeError::Timeout) => return Err(DecodeError::Cancelled),
        Err(e) => {
            tracing::debug!("poster frame at {t} failed ({e}); trying the first frame");
            dec.frame_at_size(v.start, SeekMode::Exact, tw, th)?
        }
    };
    let (rgba, width, height) = orient_rgba(&frame.rgba, tw, th, v.rotation, v.mirrored);
    Ok(RgbaImage {
        width,
        height,
        rgba,
        hdr,
    })
}

fn still_poster(path: &Path, max_side: u32) -> Result<RgbaImage, image::ImageError> {
    use image::ImageDecoder;
    let mut decoder = image::ImageReader::open(path)?
        .with_guessed_format()?
        .into_decoder()?;
    let orientation = decoder.orientation()?;
    let mut img = image::DynamicImage::from_decoder(decoder)?;
    img.apply_orientation(orientation);
    let (w, h) = fit_within(img.width(), img.height(), max_side);
    if (w, h) != (img.width(), img.height()) {
        img = img.resize_exact(w, h, image::imageops::FilterType::Triangle);
    }
    let rgba = img.to_rgba8();
    Ok(RgbaImage {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
        hdr: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use freecut_timeline::model::Rotation;

    fn stream(start: i64, duration: i64) -> VideoStreamInfo {
        VideoStreamInfo {
            index: 0,
            codec: "h264".into(),
            width: 16,
            height: 16,
            sar: (1, 1),
            rotation: Rotation::None,
            mirrored: false,
            rate: None,
            variable_rate: false,
            has_alpha: false,
            bit_depth: 8,
            start: Tick::from_raw(start),
            duration: Tick::from_raw(duration),
        }
    }

    #[test]
    fn poster_time_rule() {
        let s = TICKS_PER_SECOND;
        assert_eq!(poster_time(&stream(0, s / 2)), Tick::ZERO);
        assert_eq!(poster_time(&stream(0, 4 * s)), Tick::from_raw(4 * s / 10));
        assert_eq!(poster_time(&stream(0, 60 * s)), Tick::from_raw(s));
        assert_eq!(poster_time(&stream(100, 60 * s)), Tick::from_raw(s + 100));
    }
}
