//! Planar `f32` → interleaved `f32` remixing and resampling (libswresample).
//!
//! Input is always native-rate planar `f32` (the sample-format conversion is
//! done in safe Rust by the audio reader), so this wrapper only remixes the
//! channel count and changes the rate. With `exact_rational` (on by default)
//! the polyphase filter repeats exactly every `in_rate / gcd` input samples,
//! which the audio reader relies on to make random reads match sequential
//! ones.
//!
//! **Stereo output** uses FreeCut's own matrix ([`stereo_matrix`], AU-010)
//! instead of libswresample's default one, which lowers mono by 3 dB and
//! normalises downmixes: mono is copied to both channels at unity, and more
//! than two channels are downmixed with the ITU coefficients (front left and
//! right at unity, centre and surrounds at −3 dB, LFE dropped). Other output
//! counts keep libswresample's default matrices.
//!
//! **Unlabelled streams** (no channel layout, e.g. plain-PCM WAVs from
//! multitrack recorders) are mixed as FFmpeg's default layout for the
//! count, except that no channel is dropped: the channel the default layout
//! calls LFE may be a real track (a 3-channel file is "2.1" by default, a
//! 6-channel poly WAV "5.1"), so it goes to both sides at −3 dB like any
//! unknown channel ([`unlabelled_channel_ids`]).

use std::ffi::c_int;
use std::mem::MaybeUninit;
use std::ptr;

use ffmpeg_next::ffi;

/// A libswresample context converting planar `f32` to interleaved `f32`.
pub(crate) struct Resampler {
    ctx: *mut ffi::SwrContext,
    in_channels: usize,
    out_channels: usize,
}

// SAFETY: the context is a heap object not tied to the creating thread and is
// only used through `&mut self`, never by two threads at once.
unsafe impl Send for Resampler {}

impl std::fmt::Debug for Resampler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Resampler")
            .field("in_channels", &self.in_channels)
            .field("out_channels", &self.out_channels)
            .finish_non_exhaustive()
    }
}

fn default_layout(channels: u16) -> ffi::AVChannelLayout {
    let mut layout = MaybeUninit::<ffi::AVChannelLayout>::zeroed();
    // SAFETY: an all-zero `AVChannelLayout` is a valid "unspecified, 0
    // channels" layout (order 0 = AV_CHANNEL_ORDER_UNSPEC, null opaque);
    // `av_channel_layout_default` then overwrites it with a native layout
    // that owns no heap memory.
    unsafe {
        ffi::av_channel_layout_default(layout.as_mut_ptr(), c_int::from(channels));
        layout.assume_init()
    }
}

/// AU-010's −3 dB: `1/√2`.
const MINUS_3_DB: f64 = std::f64::consts::FRAC_1_SQRT_2;

/// Where one input channel goes in a stereo mix: its gains into left and
/// right.
fn stereo_gains(channel: i32) -> [f64; 2] {
    // FFmpeg `AVChannel` numbers (see `av::layout`).
    match channel {
        // Front left / right, left / right of centre, wide, the stereo
        // downmix pair and binaural: unity on their side.
        0 | 6 | 29 | 31 | 61 => [1.0, 0.0],
        1 | 7 | 30 | 32 | 62 => [0.0, 1.0],
        // LFE (both), and channels marked unused: dropped.
        3 | 35 | 512 => [0.0, 0.0],
        // Left surrounds and heights: back, side, surround direct, top
        // front / back / side, bottom front, side and top surround.
        4 | 9 | 33 | 12 | 15 | 36 | 39 | 41 | 43 => [MINUS_3_DB, 0.0],
        5 | 10 | 34 | 14 | 17 | 37 | 40 | 42 | 44 => [0.0, MINUS_3_DB],
        // Front centre, back / top / bottom centres and anything unknown:
        // both sides at −3 dB.
        _ => [MINUS_3_DB, MINUS_3_DB],
    }
}

/// The stereo mix of input channels `ids` (FFmpeg `AVChannel` numbers, in
/// stream order) as a libswresample matrix: entry `in + n·out` (with `n =
/// ids.len()`) is input channel `in`'s gain in output `out` (0 = left,
/// 1 = right). A single channel is mono whatever it is labelled, and is
/// copied to both sides at unity (AU-010).
pub(crate) fn stereo_matrix(ids: &[i32]) -> Vec<f64> {
    let n = ids.len();
    let mut m = vec![0.0; 2 * n];
    if n == 1 {
        m.fill(1.0);
        return m;
    }
    for (i, &id) in ids.iter().enumerate() {
        let [l, r] = stereo_gains(id);
        m[i] = l;
        m[n + i] = r;
    }
    m
}

/// The speakers assumed for an unlabelled stream of `channels` channels:
/// FFmpeg's default layout, with LFE replaced by "unknown" (both sides at
/// −3 dB) so a real track is never silently dropped.
pub(crate) fn unlabelled_channel_ids(channels: u16) -> Option<Vec<i32>> {
    let mut ids = super::layout::default_channel_ids(channels)?;
    for id in &mut ids {
        if matches!(*id, LFE | LFE_2) {
            *id = UNKNOWN;
        }
    }
    Some(ids)
}

/// `AV_CHAN_LOW_FREQUENCY`, `AV_CHAN_LOW_FREQUENCY_2` and an id no
/// layout uses (`AV_CHAN_NONE`), as plain integers.
const LFE: i32 = 3;
const LFE_2: i32 = 35;
const UNKNOWN: i32 = -1;

impl Resampler {
    /// A converter from `in_channels` planar channels at `in_rate` to
    /// `out_channels` interleaved ones at `out_rate`. `in_ids` names the
    /// speaker of each input channel (FFmpeg `AVChannel` numbers, from
    /// the stream's layout); when it is missing or doesn't match the count,
    /// the stream is unlabelled ([`unlabelled_channel_ids`]).
    pub(crate) fn new(
        in_channels: u16,
        in_rate: u32,
        out_channels: u16,
        out_rate: u32,
        in_ids: Option<&[i32]>,
    ) -> Result<Resampler, ffmpeg_next::Error> {
        let invalid = ffmpeg_next::Error::Other { errno: ffi::EINVAL };
        let (Ok(in_rate), Ok(out_rate)) = (c_int::try_from(in_rate), c_int::try_from(out_rate))
        else {
            return Err(invalid);
        };
        if in_channels == 0 || out_channels == 0 || in_rate <= 0 || out_rate <= 0 {
            return Err(invalid);
        }
        let in_layout = default_layout(in_channels);
        let out_layout = default_layout(out_channels);
        let mut ctx: *mut ffi::SwrContext = ptr::null_mut();
        // SAFETY: the layouts are valid and outlive the call (swresample
        // copies them); `ctx` starts null so a new context is allocated.
        let ret = unsafe {
            ffi::swr_alloc_set_opts2(
                &raw mut ctx,
                &raw const out_layout,
                ffi::AVSampleFormat::AV_SAMPLE_FMT_FLT,
                out_rate,
                &raw const in_layout,
                ffi::AVSampleFormat::AV_SAMPLE_FMT_FLTP,
                in_rate,
                0,
                ptr::null_mut(),
            )
        };
        if ret < 0 || ctx.is_null() {
            // SAFETY: freeing a null or partially set-up context is allowed.
            unsafe { ffi::swr_free(&raw mut ctx) };
            return Err(ffmpeg_next::Error::from(ret.min(-1)));
        }
        if out_channels == 2 {
            let ids = in_ids
                .filter(|ids| ids.len() == usize::from(in_channels))
                .map(<[i32]>::to_vec)
                .or_else(|| unlabelled_channel_ids(in_channels))
                .filter(|ids| ids.len() == usize::from(in_channels))
                .unwrap_or_else(|| vec![-1; usize::from(in_channels)]);
            let matrix = stereo_matrix(&ids);
            let identity = in_channels == 2 && matrix == [1.0, 0.0, 0.0, 1.0];
            if !identity {
                // SAFETY: `ctx` is allocated and not yet initialised, which is
                // when a matrix may be set; `matrix` holds `2 × in_channels`
                // coefficients with stride `in_channels`, and swresample
                // copies them during the call.
                let ret =
                    unsafe { ffi::swr_set_matrix(ctx, matrix.as_ptr(), c_int::from(in_channels)) };
                if ret < 0 {
                    // SAFETY: as above; `ctx` is freed exactly once.
                    unsafe { ffi::swr_free(&raw mut ctx) };
                    return Err(ffmpeg_next::Error::from(ret));
                }
            }
        }
        // SAFETY: `ctx` is a valid, exclusively owned, configured context.
        let ret = unsafe { ffi::swr_init(ctx) };
        if ret < 0 {
            // SAFETY: as above; `ctx` is freed exactly once.
            unsafe { ffi::swr_free(&raw mut ctx) };
            return Err(ffmpeg_next::Error::from(ret));
        }
        Ok(Resampler {
            ctx,
            in_channels: usize::from(in_channels),
            out_channels: usize::from(out_channels),
        })
    }

    /// Feeds one planar block (one slice per input channel, equal lengths)
    /// and appends every output frame that became available to `out`
    /// (interleaved). Returns the number of output frames appended.
    pub(crate) fn convert(
        &mut self,
        planes: &[&[f32]],
        out: &mut Vec<f32>,
    ) -> Result<usize, ffmpeg_next::Error> {
        let invalid = ffmpeg_next::Error::Other { errno: ffi::EINVAL };
        if planes.len() != self.in_channels {
            return Err(invalid);
        }
        let frames = planes.first().map_or(0, |p| p.len());
        if planes.iter().any(|p| p.len() != frames) {
            return Err(invalid);
        }
        let in_count = c_int::try_from(frames).map_err(|_| invalid)?;
        let inputs: Vec<*const u8> = planes.iter().map(|p| p.as_ptr().cast::<u8>()).collect();
        // SAFETY: `self.ctx` is a live, initialized context.
        let room = unsafe { ffi::swr_get_out_samples(self.ctx, in_count) };
        if room < 0 {
            return Err(ffmpeg_next::Error::from(room));
        }
        let room_frames = usize::try_from(room).map_err(|_| invalid)?;
        let mut buf = vec![0f32; room_frames * self.out_channels];
        let mut out_ptr = buf.as_mut_ptr().cast::<u8>();
        // SAFETY: `inputs` holds `in_channels` pointers to `frames` valid
        // `f32`s each (planar input, as configured); `buf` has room for
        // `room` interleaved output frames (the bound swresample reported for
        // this input); all buffers outlive the call.
        let got = unsafe {
            ffi::swr_convert(self.ctx, &raw mut out_ptr, room, inputs.as_ptr(), in_count)
        };
        if got < 0 {
            return Err(ffmpeg_next::Error::from(got));
        }
        let got = usize::try_from(got).map_err(|_| invalid)?.min(room_frames);
        out.extend_from_slice(&buf[..got * self.out_channels]);
        Ok(got)
    }
}

impl Drop for Resampler {
    fn drop(&mut self) {
        // SAFETY: `self.ctx` came from `swr_alloc_set_opts2` and is freed
        // exactly once; `swr_free` nulls the pointer.
        unsafe { ffi::swr_free(&raw mut self.ctx) };
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)] // test code
    use super::*;

    fn run(in_rate: u32, out_rate: u32, input: &[f32], tail: usize) -> Vec<f32> {
        let mut r = Resampler::new(1, in_rate, 1, out_rate, None).unwrap();
        let mut out = Vec::new();
        for chunk in input.chunks(1000) {
            r.convert(&[chunk], &mut out).unwrap();
        }
        r.convert(&[&vec![0.0; tail]], &mut out).unwrap();
        out
    }

    #[test]
    fn output_is_aligned_with_input_time() {
        // An impulse at input sample 1470 (44.1 kHz) is 1600 at 48 kHz.
        let mut input = vec![0.0f32; 4410];
        input[1470] = 1.0;
        let out = run(44_100, 48_000, &input, 512);
        let peak = out
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(i, _)| i)
            .unwrap();
        assert_eq!(peak, 1600, "swresample output must not be delayed");
    }

    #[test]
    fn same_rate_mono_to_stereo_is_stateless_and_at_unity() {
        let mut r = Resampler::new(1, 48_000, 2, 48_000, None).unwrap();
        let mut out = Vec::new();
        r.convert(&[&[0.5, -0.25]], &mut out).unwrap();
        // AU-010: mono is duplicated to both channels at unity gain.
        assert_eq!(out, [0.5, 0.5, -0.25, -0.25]);
    }

    /// One frame of `planes` (one value per input channel) at 48 kHz.
    fn mix_one(ids: Option<&[i32]>, frame: &[f32]) -> [f32; 2] {
        let n = u16::try_from(frame.len()).unwrap();
        let mut r = Resampler::new(n, 48_000, 2, 48_000, ids).unwrap();
        let planes: Vec<[f32; 1]> = frame.iter().map(|&v| [v]).collect();
        let refs: Vec<&[f32]> = planes.iter().map(|p| &p[..]).collect();
        let mut out = Vec::new();
        r.convert(&refs, &mut out).unwrap();
        [out[0], out[1]]
    }

    fn close(a: [f32; 2], b: [f64; 2]) -> bool {
        (f64::from(a[0]) - b[0]).abs() < 1e-6 && (f64::from(a[1]) - b[1]).abs() < 1e-6
    }

    #[test]
    fn stereo_passes_through_unchanged() {
        assert_eq!(mix_one(None, &[0.5, -0.25]), [0.5, -0.25]);
        assert_eq!(mix_one(Some(&[0, 1]), &[0.125, 0.75]), [0.125, 0.75]);
    }

    #[test]
    fn five_one_downmixes_with_itu_coefficients() {
        let g = MINUS_3_DB;
        // FL FR FC LFE SL SR, one channel at a time.
        let side: &[i32] = &[0, 1, 2, 3, 9, 10];
        let cases = [
            (0, [1.0, 0.0]),
            (1, [0.0, 1.0]),
            (2, [g, g]),
            (3, [0.0, 0.0]),
            (4, [g, 0.0]),
            (5, [0.0, g]),
        ];
        for (ch, expected) in cases {
            let mut frame = [0.0f32; 6];
            frame[ch] = 1.0;
            let got = mix_one(Some(side), &frame);
            assert!(close(got, expected), "channel {ch}: {got:?}");
        }
        // Without a layout, six channels are FFmpeg's default 5.1, except
        // that the fourth (LFE there) is kept, at −3 dB on both sides.
        let got = mix_one(None, &[0.0, 0.0, 1.0, 0.0, 0.0, 0.0]);
        assert!(close(got, [g, g]), "{got:?}");
        let got = mix_one(None, &[0.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
        assert!(close(got, [g, 0.0]), "{got:?}");
    }

    /// R6: an unlabelled stream never loses a channel to the LFE rule.
    #[test]
    fn unlabelled_channels_are_never_dropped() {
        let g = MINUS_3_DB;
        for n in [3usize, 4, 5, 6, 7, 8] {
            for ch in 0..n {
                let mut frame = vec![0.0f32; n];
                frame[ch] = 1.0;
                let got = mix_one(None, &frame);
                assert!(
                    got[0] > 0.5 || got[1] > 0.5,
                    "{n} ch, signal on #{ch}: {got:?}"
                );
            }
        }
        // 3 channels: L, R, and the third on both sides at −3 dB.
        assert!(close(mix_one(None, &[0.0, 0.0, 1.0]), [g, g]));
        // A labelled LFE is still dropped (AU-010).
        assert!(close(
            mix_one(Some(&[0, 1, 3]), &[0.0, 0.0, 1.0]),
            [0.0, 0.0]
        ));
    }

    #[test]
    fn stereo_matrix_rules() {
        assert_eq!(stereo_matrix(&[2]), [1.0, 1.0], "mono at unity");
        assert_eq!(stereo_matrix(&[0, 1]), [1.0, 0.0, 0.0, 1.0]);
        let g = MINUS_3_DB;
        // FL FR FC LFE BL BR (5.1 back): left row, then right row.
        assert_eq!(
            stereo_matrix(&[0, 1, 2, 3, 4, 5]),
            [1.0, 0.0, g, 0.0, g, 0.0, 0.0, 1.0, g, 0.0, 0.0, g]
        );
        // Unknown and unused channels.
        assert_eq!(stereo_matrix(&[768, 512]), [g, 0.0, g, 0.0]);
    }
}
