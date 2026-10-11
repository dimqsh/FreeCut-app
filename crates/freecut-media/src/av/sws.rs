//! Frame → RGBA8 conversion with FFmpeg 8's dynamic `sws_scale_frame`.
//!
//! Every conversion property (size, format, matrix, range, chroma siting) is
//! taken from the frames themselves. Before converting, the source's
//! unspecified colour tags are resolved with the CL-002 defaults: BT.601 below
//! 720 lines and BT.709 otherwise; limited range for Y′CbCr, full for RGB. The
//! destination keeps the source's primaries and transfer, so swscale performs
//! only the matrix, range and chroma step.
//!
//! **HDR and wide gamut** (CL-002). PQ and HLG frames, and frames with
//! BT.2020 primaries, are converted to 16-bit RGBA (still in the source's
//! transfer and primaries) and then mapped to SDR Rec.709 by
//! [`crate::color::ToneMapper`] (README §5.4 item 8: not swscale's own tone
//! mapping, so every CPU path follows the one CL-002 curve). The PQ peak
//! comes from the frame's content light level (MaxCLL) or mastering display
//! metadata.

use std::ffi::c_int;
use std::ptr;

use ffmpeg_next::ffi;
use ffmpeg_next::frame;

use super::pixfmt;
use crate::color::{DEFAULT_PEAK_NITS, HdrTransfer, ToneMapper};

// `SwsFlags` values from libswscale/swscale.h (stable ABI constants).
const SWS_BICUBIC: u32 = 1 << 2;
const SWS_FULL_CHR_H_INT: u32 = 1 << 13;
const SWS_ACCURATE_RND: u32 = 1 << 18;

/// A reusable conversion context. Reconfigures itself when the source
/// format or size changes (dynamic mode).
pub(crate) struct Scaler {
    ctx: *mut ffi::SwsContext,
    /// The HDR → SDR mapper of the last HDR or wide-gamut source.
    tone: Option<ToneMapper>,
}

// SAFETY: the context is a heap object not tied to the creating thread. It is
// only touched through `&mut self`, so it is never used by two threads at
// once (I5); moving it between threads is fine.
unsafe impl Send for Scaler {}

impl std::fmt::Debug for Scaler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scaler").finish_non_exhaustive()
    }
}

impl Scaler {
    pub(crate) fn new() -> Result<Scaler, ffmpeg_next::Error> {
        // SAFETY: `sws_alloc_context` has no preconditions; a null result is
        // handled below.
        let ctx = unsafe { ffi::sws_alloc_context() };
        if ctx.is_null() {
            return Err(ffmpeg_next::Error::Other { errno: ffi::ENOMEM });
        }
        // SAFETY: `ctx` is a freshly allocated, exclusively owned context;
        // `flags` and `threads` are plain public fields. One thread keeps the
        // conversion deterministic and avoids a thread pool per decoder.
        unsafe {
            (*ctx).flags = SWS_BICUBIC | SWS_ACCURATE_RND | SWS_FULL_CHR_H_INT;
            (*ctx).threads = 1;
        }
        Ok(Scaler { ctx, tone: None })
    }

    /// Converts `src` to tightly packed RGBA8 (`dst_w * dst_h * 4` bytes),
    /// scaling when the sizes differ.
    pub(crate) fn convert_rgba(
        &mut self,
        src: &frame::Video,
        dst_w: u32,
        dst_h: u32,
    ) -> Result<Vec<u8>, ffmpeg_next::Error> {
        let invalid = ffmpeg_next::Error::Other { errno: ffi::EINVAL };
        let (Ok(w), Ok(h)) = (c_int::try_from(dst_w), c_int::try_from(dst_h)) else {
            return Err(invalid);
        };
        if w == 0 || h == 0 || src.width() == 0 || src.height() == 0 {
            return Err(invalid);
        }
        let info = pixfmt::frame_info(src).ok_or(invalid)?;
        if info.is_hwaccel {
            return Err(invalid);
        }

        // A new reference to the same buffers, so the colour tags can be
        // resolved without touching the caller's frame.
        let mut source = frame::Video::empty();
        // SAFETY: both pointers are valid frames owned by ffmpeg-next
        // wrappers; `source` is freshly allocated and empty, as
        // `av_frame_ref` requires.
        let ret = unsafe { ffi::av_frame_ref(source.as_mut_ptr(), src.as_ptr()) };
        if ret < 0 {
            return Err(ffmpeg_next::Error::from(ret));
        }

        let mut dst = frame::Video::empty();
        let wide;
        // SAFETY: `source` and `dst` are exclusively owned frames. Colour
        // fields are read and written as raw integers so no out-of-range enum
        // value is ever materialized; every value written is a valid
        // `AVColorSpace`/`AVColorRange`/`AVPixelFormat`.
        unsafe {
            let s = source.as_mut_ptr();
            let space_ptr = ptr::addr_of_mut!((*s).colorspace).cast::<c_int>();
            let range_ptr = ptr::addr_of_mut!((*s).color_range).cast::<c_int>();
            if space_ptr.read() == ffi::AVColorSpace::AVCOL_SPC_UNSPECIFIED as c_int && !info.is_rgb
            {
                let space = if src.height() >= 720 {
                    ffi::AVColorSpace::AVCOL_SPC_BT709
                } else {
                    ffi::AVColorSpace::AVCOL_SPC_SMPTE170M
                };
                space_ptr.write(space as c_int);
            }
            if range_ptr.read() == ffi::AVColorRange::AVCOL_RANGE_UNSPECIFIED as c_int {
                let range = if info.is_rgb {
                    ffi::AVColorRange::AVCOL_RANGE_JPEG
                } else {
                    ffi::AVColorRange::AVCOL_RANGE_MPEG
                };
                range_ptr.write(range as c_int);
            }

            let prim = ptr::addr_of!((*s).color_primaries).cast::<c_int>().read();
            let trc = ptr::addr_of!((*s).color_trc).cast::<c_int>().read();
            wide = wide_source(trc, prim).map(|tf| (tf, prim == PRI_BT2020));
            let d = dst.as_mut_ptr();
            (*d).format = if wide.is_some() {
                ffi::AVPixelFormat::AV_PIX_FMT_RGBA64LE as c_int
            } else {
                ffi::AVPixelFormat::AV_PIX_FMT_RGBA as c_int
            };
            (*d).width = w;
            (*d).height = h;
            (*d).colorspace = ffi::AVColorSpace::AVCOL_SPC_RGB;
            (*d).color_range = ffi::AVColorRange::AVCOL_RANGE_JPEG;
            ptr::addr_of_mut!((*d).color_primaries)
                .cast::<c_int>()
                .write(prim);
            ptr::addr_of_mut!((*d).color_trc).cast::<c_int>().write(trc);
        }

        // SAFETY: `self.ctx` is a live context used only here (exclusive
        // borrow); `dst` has format and size set and no buffers, which
        // `sws_scale_frame` then allocates; `source` holds valid image data.
        let ret = unsafe { ffi::sws_scale_frame(self.ctx, dst.as_mut_ptr(), source.as_ptr()) };
        if ret < 0 {
            return Err(ffmpeg_next::Error::from(ret));
        }

        let bytes_per_pixel = if wide.is_some() { 8 } else { 4 };
        let row = dst_w as usize * bytes_per_pixel;
        let stride = dst.stride(0);
        let data = dst.data(0);
        let rows = dst_h as usize;
        if stride < row || data.len() < stride * (rows - 1) + row {
            return Err(ffmpeg_next::Error::Bug);
        }
        let mut out = Vec::with_capacity(row * rows);
        for y in 0..rows {
            out.extend_from_slice(&data[y * stride..y * stride + row]);
        }
        let Some((tf, bt2020)) = wide else {
            return Ok(out);
        };
        let peak = match tf {
            HdrTransfer::Pq => pq_peak_nits(src).unwrap_or(DEFAULT_PEAK_NITS),
            HdrTransfer::Hlg | HdrTransfer::Sdr => DEFAULT_PEAK_NITS,
        };
        let mapper = match self.tone.take() {
            Some(m) if m.is_for(tf, peak, bt2020) => m,
            _ => ToneMapper::new(tf, peak, bt2020),
        };
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get().min(8));
        let rgba = mapper.map_rgba64le(&out, dst_w as usize, threads);
        self.tone = Some(mapper);
        Ok(rgba)
    }
}

// `AVColorTransferCharacteristic` / `AVColorPrimaries` values (stable ABI).
const TRC_SMPTE2084: c_int = 16;
const TRC_ARIB_STD_B67: c_int = 18;
const PRI_BT2020: c_int = 9;

/// How a source with transfer `trc` and primaries `prim` (raw values) must
/// be mapped to SDR Rec.709, or `None` when it is SDR Rec.709-ish already.
fn wide_source(trc: c_int, prim: c_int) -> Option<HdrTransfer> {
    hdr_transfer(trc).or((prim == PRI_BT2020).then_some(HdrTransfer::Sdr))
}

/// The HDR transfer of raw transfer value `trc` (PQ or HLG), or `None` for
/// an SDR curve (CL-002).
pub(crate) fn hdr_transfer(trc: c_int) -> Option<HdrTransfer> {
    match trc {
        TRC_SMPTE2084 => Some(HdrTransfer::Pq),
        TRC_ARIB_STD_B67 => Some(HdrTransfer::Hlg),
        _ => None,
    }
}

/// The source peak of a PQ frame from its side data: MaxCLL, else the
/// mastering display's maximum luminance (cd/m²).
fn pq_peak_nits(frame: &frame::Video) -> Option<f32> {
    // Byte offsets in the public, ABI-stable structs of
    // libavutil/mastering_display_metadata.h: `AVContentLightMetadata` is
    // `{ unsigned MaxCLL, MaxFALL; }`; `AVMasteringDisplayMetadata` holds
    // eight `AVRational`s (primaries, white point, min luminance) before
    // `max_luminance` at byte 72, then `has_primaries` and `has_luminance`
    // (ints at 80 and 84).
    let read = |kind: ffi::AVFrameSideDataType, offset: usize| -> Option<[u8; 4]> {
        // SAFETY: `frame` wraps a valid AVFrame; `av_frame_get_side_data`
        // only reads its side-data list and returns null or a pointer to an
        // entry owned by the frame, alive while `frame` is borrowed. Its
        // `data` holds `size` bytes, and only bytes below `size` are read.
        unsafe {
            let sd = ffi::av_frame_get_side_data(frame.as_ptr(), kind);
            if sd.is_null() || (*sd).data.is_null() || (*sd).size < offset + 4 {
                return None;
            }
            Some(ptr::read_unaligned(
                (*sd).data.add(offset).cast::<[u8; 4]>(),
            ))
        }
    };
    let cll = ffi::AVFrameSideDataType::AV_FRAME_DATA_CONTENT_LIGHT_LEVEL;
    if let Some(max_cll) = read(cll, 0).map(u32::from_ne_bytes)
        && max_cll > 0
    {
        return Some(max_cll as f32);
    }
    let mastering = ffi::AVFrameSideDataType::AV_FRAME_DATA_MASTERING_DISPLAY_METADATA;
    let has_luminance = read(mastering, 84).map(i32::from_ne_bytes)?;
    let num = read(mastering, 72).map(i32::from_ne_bytes)?;
    let den = read(mastering, 76).map(i32::from_ne_bytes)?;
    (has_luminance != 0 && num > 0 && den > 0).then(|| num as f32 / den as f32)
}

impl Drop for Scaler {
    fn drop(&mut self) {
        // SAFETY: `self.ctx` came from `sws_alloc_context` and is freed once;
        // `sws_free_context` nulls the pointer.
        unsafe { ffi::sws_free_context(&raw mut self.ctx) };
    }
}

// More `SwsFlags` values (stable ABI constants).
const SWS_AREA: u32 = 1 << 5;
const SWS_FULL_CHR_H_INP: u32 = 1 << 14;

/// RGBA8 → 8-bit Y′CbCr for the encoders (EX-032): BT.709 matrix, limited
/// range, left-sited 4:2:0 chroma.
///
/// The colour tags come from the frames, which [`YuvConverter::convert`]
/// sets itself, so the matrix and range used are the ones the stream is
/// tagged with (P4). Both sides carry BT.709 primaries and transfer, so
/// swscale does the matrix, range and chroma step only. Chroma is taken
/// from every input pixel (`FULL_CHR_H_INP`) and reduced with the area
/// (box) filter; the rounding is accurate and single-threaded, so the same
/// input always gives the same planes.
pub(crate) struct YuvConverter {
    ctx: *mut ffi::SwsContext,
}

// SAFETY: as for `Scaler`: a heap object used only through `&mut self`.
unsafe impl Send for YuvConverter {}

impl std::fmt::Debug for YuvConverter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("YuvConverter").finish_non_exhaustive()
    }
}

impl YuvConverter {
    pub(crate) fn new() -> Result<YuvConverter, ffmpeg_next::Error> {
        // SAFETY: no preconditions; null is handled below.
        let ctx = unsafe { ffi::sws_alloc_context() };
        if ctx.is_null() {
            return Err(ffmpeg_next::Error::Other { errno: ffi::ENOMEM });
        }
        // SAFETY: freshly allocated, exclusively owned context; plain fields.
        unsafe {
            (*ctx).flags = SWS_AREA | SWS_ACCURATE_RND | SWS_FULL_CHR_H_INP;
            (*ctx).threads = 1;
        }
        Ok(YuvConverter { ctx })
    }

    /// Converts `src` (RGBA, any colour tags) into `dst`, an allocated
    /// writable frame of the same size in an 8-bit Y′CbCr format. Both
    /// frames are re-tagged first: `src` as full-range RGB with BT.709
    /// primaries and transfer, `dst` as BT.709 limited range, left siting.
    pub(crate) fn convert(
        &mut self,
        src: &mut frame::Video,
        dst: &mut frame::Video,
    ) -> Result<(), ffmpeg_next::Error> {
        use ffmpeg_next::color::{Primaries, Range, Space, TransferCharacteristic};
        if src.width() != dst.width() || src.height() != dst.height() || src.width() == 0 {
            return Err(ffmpeg_next::Error::Other { errno: ffi::EINVAL });
        }
        src.set_color_space(Space::RGB);
        src.set_color_range(Range::JPEG);
        src.set_color_primaries(Primaries::BT709);
        src.set_color_transfer_characteristic(TransferCharacteristic::BT709);
        dst.set_color_space(Space::BT709);
        dst.set_color_range(Range::MPEG);
        dst.set_color_primaries(Primaries::BT709);
        dst.set_color_transfer_characteristic(TransferCharacteristic::BT709);
        super::encode::set_frame_chroma_left(dst);
        // SAFETY: `self.ctx` is a live context used only here (exclusive
        // borrow). `dst` has format, size and buffers set (the caller made
        // it writable), which `sws_scale_frame` writes into; `src` holds
        // valid image data of the same size.
        let ret = unsafe { ffi::sws_scale_frame(self.ctx, dst.as_mut_ptr(), src.as_ptr()) };
        if ret < 0 {
            Err(ffmpeg_next::Error::from(ret))
        } else {
            Ok(())
        }
    }
}

impl Drop for YuvConverter {
    fn drop(&mut self) {
        // SAFETY: allocated by `sws_alloc_context`, freed once; nulls it.
        unsafe { ffi::sws_free_context(&raw mut self.ctx) };
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)] // test code
    use super::*;
    use ffmpeg_next::format::Pixel;

    fn gray_frame(w: u32, h: u32, y: u8) -> frame::Video {
        let mut f = frame::Video::new(Pixel::YUV420P, w, h);
        for plane in 0..3 {
            let v = if plane == 0 { y } else { 128 };
            f.data_mut(plane).fill(v);
        }
        f
    }

    #[test]
    fn limited_range_gray_maps_to_full_range_rgb() {
        let mut s = Scaler::new().unwrap();
        // Untagged Y′CbCr is limited range (CL-002): 16 → 0, 235 → 255.
        let black = s.convert_rgba(&gray_frame(64, 48, 16), 64, 48).unwrap();
        let white = s.convert_rgba(&gray_frame(64, 48, 235), 64, 48).unwrap();
        assert_eq!(black.len(), 64 * 48 * 4);
        assert!(black.chunks(4).all(|p| p[0] <= 1 && p[3] == 255));
        assert!(white.chunks(4).all(|p| p[0] >= 254 && p[1] >= 254));
    }

    #[test]
    fn scales_to_the_requested_size() {
        let mut s = Scaler::new().unwrap();
        let out = s.convert_rgba(&gray_frame(64, 48, 126), 17, 9).unwrap();
        assert_eq!(out.len(), 17 * 9 * 4);
        assert!(out.chunks(4).all(|p| p[0].abs_diff(128) <= 2));
    }

    fn rgba_frame(w: u32, h: u32, rgb: (u8, u8, u8)) -> frame::Video {
        let mut f = frame::Video::new(Pixel::RGBA, w, h);
        let stride = f.stride(0);
        let data = f.data_mut(0);
        for y in 0..h as usize {
            for x in 0..w as usize {
                let i = y * stride + x * 4;
                data[i..i + 4].copy_from_slice(&[rgb.0, rgb.1, rgb.2, 255]);
            }
        }
        f
    }

    /// BT.709 limited-range code values of an 8-bit R′G′B′ colour (EX-032).
    fn bt709(r: u8, g: u8, b: u8) -> (u8, u8, u8) {
        let (r, g, b) = (
            f64::from(r) / 255.0,
            f64::from(g) / 255.0,
            f64::from(b) / 255.0,
        );
        let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let cb = (b - y) / 1.8556;
        let cr = (r - y) / 1.5748;
        (
            (16.0 + 219.0 * y).round() as u8,
            (128.0 + 224.0 * cb).round() as u8,
            (128.0 + 224.0 * cr).round() as u8,
        )
    }

    #[test]
    fn rgb_converts_with_the_bt709_limited_formulas() {
        let mut c = YuvConverter::new().unwrap();
        let colors = [
            (0, 0, 0),
            (255, 255, 255),
            (255, 0, 0),
            (0, 255, 0),
            (0, 0, 255),
            (128, 128, 128),
            (200, 120, 40),
        ];
        for rgb in colors {
            let mut src = rgba_frame(16, 8, rgb);
            let mut dst = frame::Video::new(Pixel::YUV420P, 16, 8);
            c.convert(&mut src, &mut dst).unwrap();
            let want = bt709(rgb.0, rgb.1, rgb.2);
            let got = (dst.data(0)[0], dst.data(1)[0], dst.data(2)[0]);
            for (g, w) in [(got.0, want.0), (got.1, want.1), (got.2, want.2)] {
                assert!(g.abs_diff(w) <= 1, "{rgb:?}: got {got:?}, want {want:?}");
            }
            // Flat input stays flat (no dither, no ringing).
            assert!(dst.data(0)[..16].iter().all(|&v| v == got.0));
        }
    }

    #[test]
    fn mismatched_sizes_are_refused() {
        let mut c = YuvConverter::new().unwrap();
        let mut src = rgba_frame(16, 8, (1, 2, 3));
        let mut dst = frame::Video::new(Pixel::YUV420P, 8, 8);
        assert!(c.convert(&mut src, &mut dst).is_err());
    }

    #[test]
    fn zero_sizes_are_refused() {
        let mut s = Scaler::new().unwrap();
        assert!(s.convert_rgba(&gray_frame(64, 48, 16), 0, 9).is_err());
    }
}
