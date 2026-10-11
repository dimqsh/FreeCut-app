//! FX-030's photosensitivity rule as a checker (TX-056's design rules):
//! how often a preset changes brightness or opacity sharply, and how often
//! it dips. The app offers only presets that pass ([`Photosensitivity::is_safe`]);
//! [`super::PresetTable`] caches the result per manifest.

use super::eval::{BlockOffsets, MotionScale, UnitContext, pose_with};
use super::manifest::{ChannelProp, PresetKind, PresetManifest};
use crate::model::{AnimationSpec, PerMille, Slot};
use crate::time::{TICKS_PER_SECOND, Tick};

/// How a preset fares against the photosensitivity rule, at its worst over
/// Intensity 0, 50 and 100, its default length and (In and Out) FX-018's
/// 0.1 s minimum, and, when brightness depends on the seed, several seeds
/// and units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Photosensitivity {
    /// The most flashes (pairs of opposing sharp changes) in any second.
    pub flashes_per_second: u32,
    /// The most dips (a sharp fall followed by a sharp rise) starting in
    /// any second.
    pub dips_per_second: u32,
    /// The kind's flash limit: [`Photosensitivity::MAX_FLASHES`], or
    /// [`Photosensitivity::MAX_LOOP_FLASHES`] for loops.
    pub flash_limit: u32,
}

impl Photosensitivity {
    /// FX-030: no more than 3 flashes in any second.
    pub const MAX_FLASHES: u32 = 3;
    /// Loops repeat, so they are held to 2 (FX-030, TX-056).
    pub const MAX_LOOP_FLASHES: u32 = 2;
    /// Flicker-type motion: at most 2 dips in any second (FX-030).
    pub const MAX_DIPS: u32 = 2;

    /// Within every limit: the preset may be offered.
    pub const fn is_safe(&self) -> bool {
        self.flashes_per_second <= self.flash_limit && self.dips_per_second <= Self::MAX_DIPS
    }
}

/// A swing of at least 10 % of the full range is a sharp change (the
/// three-flash guideline's threshold).
const SWING: f64 = 0.1;

/// Samples per second of preset time.
const RATE: f64 = 240.0;

/// The photosensitivity of `manifest` (FX-030's rule, TX-056's design
/// rules). The brightness signals are the layer's opacity, and that
/// opacity times the white tint, the fill lightening, the two together as
/// the renderer adds them, half the glow factor (each 0–1) and the
/// relative luminance of six saturated fills turned by the hue channel. A
/// sharp change is a swing of at least 10 % (with hysteresis); a flash is
/// two of them. Entrances are checked as In and as Out, at the default
/// length and at 0.1 s; loops over three seconds of cycles at their
/// default cycle (shorter cycles are bounded by [`min_safe_cycle`]);
/// drifts and combined presets over their default span.
pub fn photosensitivity(manifest: &PresetManifest) -> Photosensitivity {
    survey(manifest, Depth::Full, None)
}

/// The shortest cycle a loop plays at (FX-019, FX-030): the shortest cycle
/// on a 10 ms grid, from FX-019's 0.1 s minimum up to the preset's default
/// cycle, from which every grid cycle up to the default passes
/// [`photosensitivity`], so shortening the cycle can never make a loop
/// strobe. 0.1 s for loops that don't change brightness and for presets
/// other than loops; the default cycle (within 0.1–10 s) for a loop that
/// fails even there, which is never offered.
pub fn min_safe_cycle(manifest: &PresetManifest) -> Tick {
    let floor = AnimationSpec::MIN_DURATION;
    if manifest.kind != PresetKind::Loop {
        return floor;
    }
    let safe = |cycle: Tick| survey(manifest, Depth::Full, Some(cycle)).is_safe();
    if safe(floor) {
        return floor;
    }
    let top = manifest
        .default_duration
        .clamp(floor, AnimationSpec::MAX_CYCLE);
    if !safe(top) {
        return top;
    }
    // Down the grid from the default until a cycle fails: sampled counts
    // near a limit are not monotone in the cycle, so no bisection.
    let step = TICKS_PER_SECOND / 100;
    let mut shortest = top;
    let mut next = (top.raw() - 1) / step * step;
    while next > floor.raw() && safe(Tick::from_raw(next)) {
        shortest = Tick::from_raw(next);
        next -= step;
    }
    shortest
}

/// The most flashes in any one second of a preset at its defaults only
/// (Intensity 50, its default length, seed 0): the quick form of
/// [`photosensitivity`] (about a sixth of its work), for callers that
/// can't cache. Grids should ask a cached catalogue instead
/// ([`super::PresetTable::availability`]).
pub fn max_flashes_per_second(manifest: &PresetManifest) -> u32 {
    survey(manifest, Depth::Defaults, None).flashes_per_second
}

/// How much of the parameter space a survey covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Depth {
    /// Intensity 50, the default length, seed 0.
    Defaults,
    /// Intensity 0, 50 and 100; In and Out also at 0.1 s; several seeds
    /// and units when a seeded channel changes brightness.
    Full,
}

/// The survey of `manifest`; a loop at `cycle` instead of its default.
fn survey(manifest: &PresetManifest, depth: Depth, cycle: Option<Tick>) -> Photosensitivity {
    let full = depth == Depth::Full;
    let mut base = manifest.default_spec();
    if manifest.kind == PresetKind::Loop
        && let Some(cycle) = cycle
    {
        base.duration = cycle;
    }
    let scale = MotionScale {
        se_px: 1080.0,
        em_px: 64.0,
        travel_px: 1000.0,
    };
    let secs = |t: Tick| t.as_secs_f64();
    let d = secs(base.duration);
    let min = secs(AnimationSpec::MIN_DURATION);
    let timings: Vec<(Slot, Tick, f64)> = match manifest.kind {
        PresetKind::Loop => vec![(Slot::Continuous, base.duration, 3.0)],
        PresetKind::Drift | PresetKind::Combined => vec![(Slot::Continuous, base.duration, d)],
        PresetKind::Entrance => [Slot::Entrance, Slot::Exit]
            .into_iter()
            .flat_map(|s| [(s, base.duration, d), (s, AnimationSpec::MIN_DURATION, min)])
            .collect(),
        PresetKind::Exit => vec![
            (Slot::Exit, base.duration, d),
            (Slot::Exit, AnimationSpec::MIN_DURATION, min),
        ],
    };
    // The quick survey keeps the default lengths only.
    let timings: Vec<(Slot, Tick, f64)> = timings
        .into_iter()
        .filter(|(_, length, _)| full || *length == base.duration)
        .collect();
    // Seeded brightness: try a few seeds and units.
    let seeded_brightness = manifest
        .channels
        .iter()
        .any(|c| c.property == ChannelProp::FlickerOpacity);
    let variants: &[(u32, u32)] = if seeded_brightness && full {
        &[(0, 0), (1, 0), (2, 0), (0, 1), (1, 2), (7, 5)]
    } else {
        &[(0, 0)]
    };
    let mut flashes = 0;
    let mut dips = 0;
    for &(seed, index) in variants {
        let intensities: &[u16] = if full { &[0, 500, 1000] } else { &[500] };
        for &intensity in intensities {
            for &(slot, length, watch) in &timings {
                let mut spec = base.clone();
                spec.duration = length;
                spec.params.seed = seed;
                spec.params.intensity = PerMille::new(intensity);
                let unit = UnitContext {
                    index,
                    ..UnitContext::BLOCK
                };
                let span = secs(length).max(1e-9);
                let n = ((watch * RATE).ceil().clamp(2.0, 1e6)) as u32;
                let mut signals: [Vec<(f64, f64)>; SIGNALS] = Default::default();
                for i in 0..=n {
                    let t = f64::from(i) / RATE;
                    let p = match manifest.kind {
                        PresetKind::Loop => (t / span).fract(),
                        _ => (t / span).min(1.0),
                    };
                    let o = pose_with(manifest, &spec, slot, p, &scale, &unit);
                    for (s, v) in signals.iter_mut().zip(brightness(&o)) {
                        s.push((t, v));
                    }
                }
                for signal in &signals {
                    let found = changes(signal);
                    let times: Vec<f64> = found.iter().map(|c| c.0).collect();
                    flashes = flashes.max(most_in_a_second(&times) / 2);
                    let dip_times: Vec<f64> = found
                        .windows(2)
                        .filter(|w| w[0].1 < 0 && w[1].1 > 0)
                        .map(|w| w[0].0)
                        .collect();
                    dips = dips.max(most_in_a_second(&dip_times));
                }
            }
        }
    }
    let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    Photosensitivity {
        flashes_per_second: count(flashes),
        dips_per_second: count(dips),
        flash_limit: if manifest.kind == PresetKind::Loop {
            Photosensitivity::MAX_LOOP_FLASHES
        } else {
            Photosensitivity::MAX_FLASHES
        },
    }
}

/// How many brightness signals [`brightness`] gives.
const SIGNALS: usize = 5 + HUES.len();

/// The hues (degrees) of the saturated fills whose luminance a hue turn
/// changes: red, yellow, green, cyan, blue and magenta.
const HUES: [f64; 6] = [0.0, 60.0, 120.0, 180.0, 240.0, 300.0];

/// The brightness signals of a pose (each 0–1): opacity; opacity times the
/// white tint, the fill lightening, both together as the renderer adds
/// them per glyph (`freecut-render`'s glyph tint, which also bounds the
/// whole-text path's lighten-then-tint), and half the glow factor; and,
/// for each of [`HUES`], opacity times the relative luminance of that
/// saturated fill turned by the hue channel (a hue turn on saturated text
/// swings its luminance: blue to yellow is a large change).
fn brightness(o: &BlockOffsets) -> [f64; SIGNALS] {
    let a = o.opacity;
    let light = o.lightness.clamp(0.0, 1.0);
    let mut out = [0.0; SIGNALS];
    out[..5].copy_from_slice(&[
        a,
        a * o.tint,
        a * light,
        a * (o.lightness.max(0.0) + o.tint).clamp(0.0, 1.0),
        a * (o.glow / 2.0).clamp(0.0, 1.0),
    ]);
    for (v, h) in out[5..].iter_mut().zip(HUES) {
        *v = a * hue_luminance(h + o.hue);
    }
    out
}

/// The relative luminance (linear, Rec. 709 weights) of the fully
/// saturated sRGB colour of hue `deg`.
fn hue_luminance(deg: f64) -> f64 {
    let h = deg.rem_euclid(360.0) / 60.0;
    // The sRGB channels of the pure hue (HSV with full saturation and
    // value): each 0–1, piecewise linear.
    let channel = |n: f64| {
        let k = (n + h).rem_euclid(6.0);
        1.0 - k.min(4.0 - k).clamp(0.0, 1.0)
    };
    let (r, g, b) = (channel(5.0), channel(3.0), channel(1.0));
    let linear = |c: f64| {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

/// Whether a preset may be offered (FX-030): [`photosensitivity`] within
/// every limit. Uncached; [`super::PresetTable`] caches it.
pub fn is_photosafe(manifest: &PresetManifest) -> bool {
    photosensitivity(manifest).is_safe()
}

/// Times of swings of at least 10 % (hysteresis on the running extreme),
/// each with its sign: +1 rising, −1 falling.
fn changes(samples: &[(f64, f64)]) -> Vec<(f64, i8)> {
    let mut out = Vec::new();
    let Some(&(_, first)) = samples.first() else {
        return out;
    };
    let (mut dir, mut ext) = (0i8, first);
    for &(t, v) in samples {
        let turned = match dir {
            0 => (v - ext).abs() >= SWING,
            1 => ext - v >= SWING,
            _ => v - ext >= SWING,
        };
        if turned {
            dir = if v > ext { 1 } else { -1 };
            out.push((t, dir));
            ext = v;
        } else if (dir == 1 && v > ext) || (dir == -1 && v < ext) {
            ext = v;
        }
    }
    out
}

/// The most entries of `times` (sorted) inside any one second.
fn most_in_a_second(times: &[f64]) -> usize {
    (0..times.len())
        .map(|i| {
            times[i..]
                .iter()
                .take_while(|&&t| t < times[i] + 1.0)
                .count()
        })
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_counting_follows_the_three_flash_rule() {
        // A fade: one change, no flash.
        let fade: Vec<_> = (0..=100)
            .map(|i| (f64::from(i) / 100.0, f64::from(i) / 100.0))
            .collect();
        assert_eq!(changes(&fade).len(), 1);
        assert_eq!(most_in_a_second(&[0.0]) / 2, 0);
        // Ten on/off flips in a second: five flashes.
        let flicker: Vec<_> = (0..=10)
            .map(|i| (f64::from(i) / 10.0, if i % 2 == 0 { 1.0 } else { 0.0 }))
            .collect();
        let found = changes(&flicker);
        let times: Vec<f64> = found.iter().map(|c| c.0).collect();
        assert_eq!(most_in_a_second(&times) / 2, 5);
        assert_eq!(found[0].1, -1);
        assert_eq!(found[1].1, 1);
        // Small ripples don't count.
        let ripple: Vec<_> = (0..=100)
            .map(|i| (f64::from(i) / 100.0, 0.95 + 0.04 * (f64::from(i)).sin()))
            .collect();
        assert!(changes(&ripple).is_empty());
    }

    #[test]
    fn hue_luminance_follows_the_colour_wheel() {
        let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
        assert!(near(hue_luminance(0.0), 0.2126), "red");
        assert!(near(hue_luminance(60.0), 0.2126 + 0.7152), "yellow");
        assert!(near(hue_luminance(120.0), 0.7152), "green");
        assert!(near(hue_luminance(240.0), 0.0722), "blue");
        assert!(near(hue_luminance(-120.0), 0.0722), "blue, turning back");
        assert!(near(hue_luminance(360.0 + 60.0), hue_luminance(60.0)));
    }
}
