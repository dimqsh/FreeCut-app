//! HDR → SDR conversion (CL-002): FreeCut's CPU reference curve.
//!
//! PQ and HLG sources (BT.2020 primaries) are mapped to SDR Rec.709 with
//! gamma-2.4 encoding, the working space of the compositor and the export:
//!
//! 1. **EOTF** to display light in cd/m²: PQ (SMPTE ST 2084) directly; HLG
//!    by its inverse OETF and the BT.2100 OOTF at a 1000 cd/m² nominal peak
//!    (system gamma 1.2).
//! 2. **Gamut**: BT.2020 → BT.709 matrix, then colours outside Rec.709 are
//!    compressed softly toward the gamut edge (a per-channel distance curve
//!    that keeps hue and lightness), not clipped per channel.
//! 3. **Tone map**: BT.2390's EETF (a Hermite shoulder in the PQ domain)
//!    applied to `max(R, G, B)`, and R, G and B scaled together so hue is
//!    kept. Light is divided by [`SDR_WHITE_NITS`] (HDR reference white,
//!    203 cd/m², BT.2408), so the EETF maps the source range onto SDR
//!    0..=100 %; its shoulder starts below reference white and places it at
//!    about 90 % of the signal for a 1000 cd/m² source (HLG's 75 % reference
//!    white → code 230 of 255), a little lower for brighter masters (about
//!    83 % at 4000 cd/m²). The source peak is the stream's metadata
//!    (MaxCLL, else the mastering display's peak) or 1000 cd/m² (HLG, and
//!    PQ without metadata).
//! 4. **Encode** with gamma 1/2.4 (BT.1886 with a zero black level).
//!
//! SDR sources tagged with BT.2020 primaries get step 2 only (decoded and
//! re-encoded with gamma 2.4, no tone mapping). Gamut compression comes
//! before tone mapping because it is scale-invariant and the tone map on
//! `max(R, G, B)` then guarantees nothing exceeds SDR white.
//!
//! [`ToneMapper`] is the per-frame form (lookup tables, used by the decoder
//! for every PQ, HLG or BT.2020 frame); it matches [`hdr_to_sdr`] within a
//! code value. This module is pure Rust and always built (no FFmpeg).

/// The transfer function of a source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HdrTransfer {
    /// SMPTE ST 2084 (absolute, up to 10 000 cd/m²).
    Pq,
    /// ARIB STD-B67 / BT.2100 hybrid log-gamma (relative).
    Hlg,
    /// An SDR gamma curve (decoded with gamma 2.4): only the gamut step
    /// applies.
    Sdr,
}

/// The display light SDR 100 % stands for: HDR reference white (BT.2408),
/// cd/m². It is also the EETF's target peak.
pub const SDR_WHITE_NITS: f32 = 203.0;

/// The assumed source peak for HLG and for PQ without metadata, cd/m².
pub const DEFAULT_PEAK_NITS: f32 = 1000.0;

/// `hdr_to_sdr` for one pixel: `rgb` is the non-linear source signal
/// (0..=1, BT.2020 primaries for PQ and HLG and for wide-gamut SDR) and
/// the result is the gamma-2.4 Rec.709 signal (0..=1).
///
/// `peak_nits` is the source peak for PQ (ignored for HLG, whose peak is
/// 1000 cd/m², and for SDR).
pub fn hdr_to_sdr(rgb: [f32; 3], tf: HdrTransfer, peak_nits: f32) -> [f32; 3] {
    Curve::new(tf, peak_nits, true).map(rgb.map(|v| decode(tf, v)))
}

/// The tone and gamut mapping after decoding, shared by the reference and
/// the table-driven path.
#[derive(Debug, Clone, Copy)]
struct Curve {
    tf: HdrTransfer,
    bt2020: bool,
    /// Source peak, cd/m² (PQ and HLG).
    peak: f32,
    eetf: Option<Eetf>,
}

impl Curve {
    fn new(tf: HdrTransfer, peak_nits: f32, bt2020: bool) -> Curve {
        let peak = match tf {
            HdrTransfer::Pq if peak_nits.is_finite() && peak_nits > 0.0 => {
                peak_nits.clamp(SDR_WHITE_NITS, 10_000.0)
            }
            _ => DEFAULT_PEAK_NITS,
        };
        let eetf = match tf {
            HdrTransfer::Sdr => None,
            HdrTransfer::Pq | HdrTransfer::Hlg => Eetf::new(peak, SDR_WHITE_NITS),
        };
        Curve {
            tf,
            bt2020,
            peak,
            eetf,
        }
    }

    /// `lin` is decoded light: cd/m² for PQ, scene light (0..=1) for HLG,
    /// relative light for SDR.
    fn map(&self, lin: [f32; 3]) -> [f32; 3] {
        let lin = lin.map(|v| if v.is_finite() { v.max(0.0) } else { 0.0 });
        let display = match self.tf {
            HdrTransfer::Hlg => hlg_ootf(lin),
            HdrTransfer::Pq | HdrTransfer::Sdr => lin,
        };
        let rgb = if self.bt2020 {
            compress_gamut(bt2020_to_bt709(display))
        } else {
            display
        };
        let m = rgb[0].max(rgb[1]).max(rgb[2]);
        let rel = match (self.tf, self.eetf) {
            (HdrTransfer::Sdr, _) => {
                if m > 1.0 {
                    rgb.map(|c| c / m)
                } else {
                    rgb
                }
            }
            (_, eetf) if m > 0.0 => {
                let mapped = eetf.map_or(m.min(SDR_WHITE_NITS), |e| e.apply(m));
                let k = mapped / (m * SDR_WHITE_NITS);
                rgb.map(|c| c * k)
            }
            _ => [0.0; 3],
        };
        rel.map(|c| c.clamp(0.0, 1.0).powf(1.0 / 2.4))
    }
}

/// Decodes one non-linear component (see [`Curve::map`] for the units).
fn decode(tf: HdrTransfer, v: f32) -> f32 {
    let v = if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        0.0
    };
    match tf {
        HdrTransfer::Pq => pq_eotf(v),
        HdrTransfer::Hlg => hlg_inverse_oetf(v),
        HdrTransfer::Sdr => v.powf(2.4),
    }
}

// SMPTE ST 2084 constants.
const PQ_M1: f32 = 2610.0 / 16384.0;
const PQ_M2: f32 = 2523.0 / 4096.0 * 128.0;
const PQ_C1: f32 = 3424.0 / 4096.0;
const PQ_C2: f32 = 2413.0 / 4096.0 * 32.0;
const PQ_C3: f32 = 2392.0 / 4096.0 * 32.0;

/// PQ signal → cd/m².
fn pq_eotf(e: f32) -> f32 {
    let p = e.powf(1.0 / PQ_M2);
    let num = (p - PQ_C1).max(0.0);
    let den = PQ_C2 - PQ_C3 * p;
    10_000.0 * (num / den).powf(1.0 / PQ_M1)
}

/// cd/m² → PQ signal.
fn pq_inverse_eotf(nits: f32) -> f32 {
    let y = (nits / 10_000.0).clamp(0.0, 1.0).powf(PQ_M1);
    ((PQ_C1 + PQ_C2 * y) / (1.0 + PQ_C3 * y)).powf(PQ_M2)
}

// BT.2100 HLG constants.
const HLG_A: f32 = 0.178_832_77;
const HLG_B: f32 = 0.284_668_92;
const HLG_C: f32 = 0.559_910_7;

/// HLG signal → scene light (0..=1).
fn hlg_inverse_oetf(e: f32) -> f32 {
    if e <= 0.5 {
        e * e / 3.0
    } else {
        (((e - HLG_C) / HLG_A).exp() + HLG_B) / 12.0
    }
}

/// HLG scene light → display light (cd/m²) at a 1000 cd/m² nominal peak.
fn hlg_ootf(scene: [f32; 3]) -> [f32; 3] {
    const GAMMA: f32 = 1.2;
    let ys = 0.2627 * scene[0] + 0.6780 * scene[1] + 0.0593 * scene[2];
    if ys <= 0.0 {
        return [0.0; 3];
    }
    let k = DEFAULT_PEAK_NITS * ys.powf(GAMMA - 1.0);
    scene.map(|c| c * k)
}

/// Linear BT.2020 → linear BT.709 (same white point).
fn bt2020_to_bt709(c: [f32; 3]) -> [f32; 3] {
    [
        1.660_491 * c[0] - 0.587_641 * c[1] - 0.072_850 * c[2],
        -0.124_55 * c[0] + 1.1329 * c[1] - 0.008_349 * c[2],
        -0.018_151 * c[0] - 0.100_579 * c[1] + 1.118_73 * c[2],
    ]
}

/// Soft gamut compression: each channel's distance below the largest one,
/// `(max − c) / max`, is left alone up to 0.8 and compressed above it so
/// the BT.2020 primaries land on the Rec.709 edge (distance 1) instead of
/// going negative. Per-channel limits are the distances of the BT.2020
/// primaries in Rec.709 (cyan, magenta, yellow), rounded up.
fn compress_gamut(rgb: [f32; 3]) -> [f32; 3] {
    const THRESHOLD: f32 = 0.8;
    const LIMITS: [f32; 3] = [1.52, 1.08, 1.09];
    const POWER: f32 = 1.2;
    let ach = rgb[0].max(rgb[1]).max(rgb[2]);
    if ach <= 0.0 {
        return [0.0; 3];
    }
    let mut out = rgb;
    for (c, limit) in out.iter_mut().zip(LIMITS) {
        let d = (ach - *c) / ach;
        if d <= THRESHOLD {
            continue;
        }
        let scale = (limit - THRESHOLD)
            / (((1.0 - THRESHOLD) / (limit - THRESHOLD)).powf(-POWER) - 1.0).powf(1.0 / POWER);
        let x = (d - THRESHOLD) / scale;
        let compressed = THRESHOLD + scale * x / (1.0 + x.powf(POWER)).powf(1.0 / POWER);
        *c = ach * (1.0 - compressed.min(1.0));
    }
    out
}

/// BT.2390's EETF from a source peak to a target peak (cd/m², black at 0),
/// in the PQ domain.
#[derive(Debug, Clone, Copy)]
struct Eetf {
    /// PQ code of the source peak.
    src_peak: f32,
    /// Target peak relative to the source peak (PQ domain).
    max_lum: f32,
    /// Knee start (PQ domain, relative).
    knee: f32,
}

impl Eetf {
    /// `None` when the source fits the target (no compression).
    fn new(source_peak: f32, target_peak: f32) -> Option<Eetf> {
        if source_peak <= target_peak {
            return None;
        }
        let src_peak = pq_inverse_eotf(source_peak);
        let max_lum = pq_inverse_eotf(target_peak) / src_peak;
        Some(Eetf {
            src_peak,
            max_lum,
            knee: 1.5 * max_lum - 0.5,
        })
    }

    /// cd/m² → cd/m² (at most the target peak).
    fn apply(&self, nits: f32) -> f32 {
        let e1 = (pq_inverse_eotf(nits) / self.src_peak).clamp(0.0, 1.0);
        let e2 = if e1 < self.knee {
            e1
        } else {
            let t = (e1 - self.knee) / (1.0 - self.knee);
            let (t2, t3) = (t * t, t * t * t);
            (2.0 * t3 - 3.0 * t2 + 1.0) * self.knee
                + (t3 - 2.0 * t2 + t) * (1.0 - self.knee)
                + (-2.0 * t3 + 3.0 * t2) * self.max_lum
        };
        pq_eotf(e2 * self.src_peak)
    }
}

/// Entries of the tone-curve table (over `sqrt(m / peak)`).
const TONE_STEPS: usize = 4096;
/// Entries of the encode table (over `sqrt(y)`).
const ENCODE_STEPS: usize = 4096;

/// The conversion of whole frames: [`hdr_to_sdr`] with lookup tables for
/// the transfer functions and the tone curve. Build one per source
/// (transfer, peak, primaries) and reuse it.
#[derive(Debug, Clone)]
pub struct ToneMapper {
    key: (HdrTransfer, u32, bool),
    curve: Curve,
    /// 16-bit code → decoded light.
    decode: Vec<f32>,
    /// `sqrt(m / tone_max)` → `curve(m) / (m · SDR_WHITE_NITS)`.
    tone: Vec<f32>,
    tone_max: f32,
    /// `sqrt(y)` → 8-bit code of `y^(1/2.4)`.
    encode: Vec<u8>,
}

impl ToneMapper {
    /// A mapper for sources with transfer `tf`, peak `peak_nits` (PQ only)
    /// and BT.2020 (`bt2020`) or Rec.709 primaries.
    pub fn new(tf: HdrTransfer, peak_nits: f32, bt2020: bool) -> ToneMapper {
        let curve = Curve::new(tf, peak_nits, bt2020);
        let decode = (0..=u16::MAX)
            .map(|code| decode(tf, f32::from(code) / 65_535.0))
            .collect();
        // Display light peaks at the source peak (values above it map to
        // the target peak anyway).
        let tone_max = match tf {
            HdrTransfer::Pq | HdrTransfer::Hlg => curve.peak,
            HdrTransfer::Sdr => 1.0,
        };
        let tone = (0..=TONE_STEPS)
            .map(|i| {
                let x = i as f32 / TONE_STEPS as f32;
                let m = (x * x * tone_max).max(1e-6 * tone_max);
                match curve.eetf {
                    Some(e) => e.apply(m) / (m * SDR_WHITE_NITS),
                    None if tf == HdrTransfer::Sdr => 1.0,
                    None => m.min(SDR_WHITE_NITS) / (m * SDR_WHITE_NITS),
                }
            })
            .collect();
        let encode = (0..=ENCODE_STEPS)
            .map(|i| {
                let s = i as f32 / ENCODE_STEPS as f32;
                // y = s², encoded y^(1/2.4) = s^(1/1.2).
                (s.powf(1.0 / 1.2) * 255.0).round() as u8
            })
            .collect();
        ToneMapper {
            key: (tf, peak_nits.to_bits(), bt2020),
            curve,
            decode,
            tone,
            tone_max,
            encode,
        }
    }

    /// Whether this mapper was built for these settings.
    pub fn is_for(&self, tf: HdrTransfer, peak_nits: f32, bt2020: bool) -> bool {
        self.key == (tf, peak_nits.to_bits(), bt2020)
    }

    /// One pixel of 16-bit codes → 8-bit Rec.709 codes.
    pub fn map_pixel(&self, rgb: [u16; 3]) -> [u8; 3] {
        let lin = rgb.map(|c| self.decode[usize::from(c)]);
        let lin = lin.map(|v| v.max(0.0));
        let display = match self.curve.tf {
            HdrTransfer::Hlg => hlg_ootf(lin),
            HdrTransfer::Pq | HdrTransfer::Sdr => lin,
        };
        let rgb = if self.curve.bt2020 {
            compress_gamut(bt2020_to_bt709(display))
        } else {
            display
        };
        let m = rgb[0].max(rgb[1]).max(rgb[2]);
        let k = if m.is_nan() || m <= 0.0 {
            0.0
        } else if self.curve.tf == HdrTransfer::Sdr {
            if m > 1.0 { 1.0 / m } else { 1.0 }
        } else {
            let x = ((m / self.tone_max).min(1.0)).sqrt() * TONE_STEPS as f32;
            let i = (x as usize).min(TONE_STEPS - 1);
            let f = x - i as f32;
            let ratio = self.tone[i] + (self.tone[i + 1] - self.tone[i]) * f;
            if m > self.tone_max {
                // Above the table: the curve is flat at the target peak.
                self.tone[TONE_STEPS] * self.tone_max / m
            } else {
                ratio
            }
        };
        rgb.map(|c| {
            let y = (c * k).clamp(0.0, 1.0);
            self.encode[((y.sqrt() * ENCODE_STEPS as f32).round() as usize).min(ENCODE_STEPS)]
        })
    }

    /// Converts tightly packed little-endian RGBA64 (`width * height * 8`
    /// bytes) into RGBA8, alpha scaled to 8 bits. Rows are spread over up
    /// to `threads` threads; the result doesn't depend on the count.
    pub fn map_rgba64le(&self, src: &[u8], width: usize, threads: usize) -> Vec<u8> {
        let pixels = src.len() / 8;
        let mut out = vec![0u8; pixels * 4];
        let rows = pixels / width.max(1);
        let bands = threads.clamp(1, 16).min(rows.max(1));
        let rows_per_band = rows.div_ceil(bands).max(1);
        let band = |src: &[u8], dst: &mut [u8]| {
            for (s, d) in src
                .as_chunks::<8>()
                .0
                .iter()
                .zip(dst.as_chunks_mut::<4>().0)
            {
                let c = |i: usize| u16::from_le_bytes([s[i], s[i + 1]]);
                let [r, g, b] = self.map_pixel([c(0), c(2), c(4)]);
                let a = ((u32::from(c(6)) * 255 + 32_767) / 65_535) as u8;
                d.copy_from_slice(&[r, g, b, a]);
            }
        };
        if bands == 1 {
            band(src, &mut out);
            return out;
        }
        std::thread::scope(|scope| {
            for (s, d) in src
                .chunks(rows_per_band * width * 8)
                .zip(out.chunks_mut(rows_per_band * width * 4))
            {
                scope.spawn(move || band(s, d));
            }
        });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(v: f32) -> u8 {
        (v * 255.0).round() as u8
    }

    #[test]
    fn pq_round_trips() {
        for nits in [0.1f32, 1.0, 100.0, 203.0, 1000.0, 4000.0] {
            let back = pq_eotf(pq_inverse_eotf(nits));
            assert!((back - nits).abs() / nits < 1e-3, "{nits} → {back}");
        }
        assert!((pq_inverse_eotf(203.0) - 0.5806).abs() < 1e-3);
    }

    #[test]
    fn hlg_reference_white_is_203_nits() {
        // BT.2408: 75 % HLG is reference white, ≈ 203 cd/m² at 1000 cd/m².
        let scene = hlg_inverse_oetf(0.75);
        let nits = hlg_ootf([scene; 3])[0];
        assert!((nits - 203.0).abs() < 2.0, "{nits}");
    }

    #[test]
    fn reference_white_lands_near_90_percent() {
        let hlg = hdr_to_sdr([0.75; 3], HdrTransfer::Hlg, 0.0);
        let pq = hdr_to_sdr([pq_inverse_eotf(203.0); 3], HdrTransfer::Pq, 1000.0);
        for (label, out) in [("HLG", hlg), ("PQ", pq)] {
            let spread = out[0].max(out[1]).max(out[2]) - out[0].min(out[1]).min(out[2]);
            assert!(spread < 1e-4, "{label}: grey stays grey: {out:?}");
            assert!((0.86..=0.92).contains(&out[0]), "{label}: {out:?}");
        }
        // HLG 75 % → code 230 (191 when passed through unconverted).
        assert_eq!(code(hlg[0]), 230);
        // Brighter masters put it a little lower.
        let bright = hdr_to_sdr([pq_inverse_eotf(203.0); 3], HdrTransfer::Pq, 4000.0);
        assert!((0.80..0.86).contains(&bright[0]), "{bright:?}");
    }

    #[test]
    fn the_curve_is_monotonic_and_rolls_off_to_white() {
        for (tf, peak) in [
            (HdrTransfer::Hlg, 0.0),
            (HdrTransfer::Pq, 1000.0),
            (HdrTransfer::Pq, 4000.0),
        ] {
            let mut last = -1.0f32;
            for i in 0..=1000 {
                let v = hdr_to_sdr([i as f32 / 1000.0; 3], tf, peak)[0];
                assert!(v >= last - 1e-6, "{tf:?} {peak}: not monotonic at {i}");
                last = v;
            }
            // The source peak reaches SDR white without clipping below it.
            let peak_signal = match tf {
                HdrTransfer::Pq => pq_inverse_eotf(peak),
                _ => 1.0,
            };
            let top = hdr_to_sdr([peak_signal; 3], tf, peak)[0];
            assert!(top > 0.99, "{tf:?} {peak}: {top}");
        }
        // Dark tones keep their proportions (below the knee).
        let a = hdr_to_sdr([pq_inverse_eotf(10.0); 3], HdrTransfer::Pq, 1000.0)[0];
        let b = hdr_to_sdr([pq_inverse_eotf(20.0); 3], HdrTransfer::Pq, 1000.0)[0];
        assert!(((b / a).powf(2.4) - 2.0).abs() < 0.01);
    }

    #[test]
    fn wide_gamut_colours_are_compressed_not_clipped() {
        // Pure BT.2020 green, SDR: in range, no negative channel, still
        // clearly green.
        let g = hdr_to_sdr([0.0, 1.0, 0.0], HdrTransfer::Sdr, 0.0);
        assert!(g.iter().all(|c| (0.0..=1.0).contains(c)), "{g:?}");
        assert!(g[1] > 0.9 && g[0] < 0.6 && g[2] < 0.6, "{g:?}");
        // Neutral SDR stays exactly where it was.
        let grey = hdr_to_sdr([0.5; 3], HdrTransfer::Sdr, 0.0);
        assert!(grey.iter().all(|c| (c - 0.5).abs() < 1e-4), "{grey:?}");
        // Colours inside the threshold are untouched by compression.
        let c = [0.5, 0.4, 0.45];
        assert_eq!(compress_gamut(c), c);
    }

    #[test]
    fn the_tables_match_the_reference() {
        for (tf, peak, bt2020) in [
            (HdrTransfer::Hlg, 1000.0, true),
            (HdrTransfer::Pq, 1000.0, true),
            (HdrTransfer::Pq, 4000.0, true),
            (HdrTransfer::Sdr, 0.0, true),
        ] {
            let mapper = ToneMapper::new(tf, peak, bt2020);
            let mut worst = 0;
            for r in (0..=65_535u32).step_by(4_369) {
                for g in (0..=65_535u32).step_by(8_191) {
                    for b in (0..=65_535u32).step_by(13_107) {
                        let codes = [r as u16, g as u16, b as u16];
                        let got = mapper.map_pixel(codes);
                        let want = Curve::new(tf, peak, bt2020)
                            .map(codes.map(|c| decode(tf, f32::from(c) / 65_535.0)))
                            .map(code);
                        for k in 0..3 {
                            worst = worst.max(got[k].abs_diff(want[k]));
                        }
                    }
                }
            }
            assert!(worst <= 1, "{tf:?}: tables differ by {worst}");
        }
    }

    #[test]
    fn frames_convert_the_same_on_any_thread_count() {
        let mapper = ToneMapper::new(HdrTransfer::Hlg, 0.0, true);
        let mut src = Vec::new();
        for i in 0..(37 * 23) {
            let v = (i * 97 % 65_536) as u16;
            for c in [v, v / 2, 65_535 - v, 65_535] {
                src.extend_from_slice(&c.to_le_bytes());
            }
        }
        let one = mapper.map_rgba64le(&src, 37, 1);
        let many = mapper.map_rgba64le(&src, 37, 5);
        assert_eq!(one.len(), 37 * 23 * 4);
        assert!(one == many);
        assert!(one.chunks(4).all(|p| p[3] == 255));
    }
}
