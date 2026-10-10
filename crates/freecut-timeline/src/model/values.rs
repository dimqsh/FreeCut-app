//! [`Interpolate`] for the fixed-point values of the transform (TR-001,
//! TR-015) and the text style (TX-021).
//!
//! Every type interpolates numerically from the exact progress of its
//! segment and rounds to its own step only when a value is evaluated
//! (TR-027). Eased progress may leave `[0, 1]` (TR-031); values then
//! extrapolate and saturate at their type's width, and properties with a
//! range are clamped where the whole transform or style is evaluated
//! ([`super::Transform::clamped`], [`super::TextClipData::clamped`]).
//! Colours interpolate in linear-light RGB with straight alpha (TX-021,
//! TR-027); angles of the text style wrap and take the shorter way round.

use super::animate::{Interpolate, Progress, lerp_i64};
use super::text::{DeciDegrees, RefPx, Rgba8, SignedPerMille, TailTip};
use super::transform::{Angle, CanvasPos, PerMille, Scale};

fn lerp_i32(a: i32, b: i32, p: Progress) -> i32 {
    let v = lerp_i64(i64::from(a), i64::from(b), p);
    i32::try_from(v).unwrap_or(if v < 0 { i32::MIN } else { i32::MAX })
}

/// Opacity, widths and other ratios: saturated at 0 and `u16::MAX`.
impl Interpolate for PerMille {
    fn interpolate(a: &PerMille, b: &PerMille, p: Progress) -> PerMille {
        let v = lerp_i64(i64::from(a.get()), i64::from(b.get()), p);
        PerMille::new(u16::try_from(v.max(0)).unwrap_or(u16::MAX))
    }
}

impl Interpolate for Scale {
    fn interpolate(a: &Scale, b: &Scale, p: Progress) -> Scale {
        Scale::from_basis_points(lerp_i32(a.basis_points(), b.basis_points(), p))
    }
}

/// Rotation interpolates numerically: 0° → 720° spins twice (TR-027).
impl Interpolate for Angle {
    fn interpolate(a: &Angle, b: &Angle, p: Progress) -> Angle {
        Angle::from_hundredths(lerp_i32(a.hundredths(), b.hundredths(), p))
    }
}

/// Position moves on a straight line (TR-031: easing changes the speed,
/// not the path).
impl Interpolate for CanvasPos {
    fn interpolate(a: &CanvasPos, b: &CanvasPos, p: Progress) -> CanvasPos {
        CanvasPos::new(lerp_i32(a.x, b.x, p), lerp_i32(a.y, b.y, p))
    }
}

impl Interpolate for RefPx {
    fn interpolate(a: &RefPx, b: &RefPx, p: Progress) -> RefPx {
        RefPx::from_hundredths(lerp_i32(a.hundredths(), b.hundredths(), p))
    }
}

impl Interpolate for SignedPerMille {
    fn interpolate(a: &SignedPerMille, b: &SignedPerMille, p: Progress) -> SignedPerMille {
        let v = lerp_i64(i64::from(a.get()), i64::from(b.get()), p);
        let v = v.clamp(i64::from(i16::MIN), i64::from(i16::MAX));
        // Clamped into i16 range just above.
        SignedPerMille::new(i16::try_from(v).unwrap_or(0))
    }
}

/// Style angles (shadow, glow, gradient) are stored wrapped into one turn,
/// so they take the shorter way round: 350° → 10° passes through 0°.
impl Interpolate for DeciDegrees {
    fn interpolate(a: &DeciDegrees, b: &DeciDegrees, p: Progress) -> DeciDegrees {
        let from = i64::from(a.tenths());
        let mut delta = i64::from(b.tenths()) - from;
        if delta > 1800 {
            delta -= 3600;
        } else if delta <= -1800 {
            delta += 3600;
        }
        let v = lerp_i64(from, from + delta, p);
        // Any i64 wraps into one turn; the residue fits an i32.
        DeciDegrees::new(i32::try_from(v.rem_euclid(3600)).unwrap_or(0))
    }
}

/// The bubble's tail tip moves on a straight line.
impl Interpolate for TailTip {
    fn interpolate(a: &TailTip, b: &TailTip, p: Progress) -> TailTip {
        TailTip {
            x: RefPx::interpolate(&a.x, &b.x, p),
            y: RefPx::interpolate(&a.y, &b.y, p),
        }
    }
}

/// sRGB colours interpolate in linear light (TX-021, CL), alpha linearly;
/// the result is clamped into gamut and rounded to 8 bits.
impl Interpolate for Rgba8 {
    fn interpolate(a: &Rgba8, b: &Rgba8, p: Progress) -> Rgba8 {
        let t = p.as_f64();
        let mix = |x: u8, y: u8| {
            let (lx, ly) = (srgb_to_linear(x), srgb_to_linear(y));
            linear_to_srgb(lx + (ly - lx) * t)
        };
        let alpha = lerp_i64(i64::from(a.a), i64::from(b.a), p).clamp(0, 255);
        Rgba8::new(
            mix(a.r, b.r),
            mix(a.g, b.g),
            mix(a.b, b.b),
            // Clamped into 0..=255 just above.
            u8::try_from(alpha).unwrap_or(255),
        )
    }
}

/// An 8-bit sRGB code value as linear light in `[0, 1]`.
pub(crate) fn srgb_to_linear(v: u8) -> f64 {
    let c = f64::from(v) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear light (clamped into `[0, 1]`) as the nearest 8-bit sRGB code
/// value.
pub(crate) fn linear_to_srgb(l: f64) -> u8 {
    let l = if l.is_nan() { 0.0 } else { l.clamp(0.0, 1.0) };
    let c = if l <= 0.003_130_8 {
        l * 12.92
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    };
    // In 0..=255 after the clamp and scale.
    (c * 255.0).round().clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Animatable;
    use crate::time::Tick;

    fn t(v: i64) -> Tick {
        Tick::from_raw(v)
    }

    fn half() -> Progress {
        Progress::new(1, 2).unwrap()
    }

    /// Keys at 0 and 100 from `a` to `b`, evaluated at 50.
    fn mid<T: Interpolate>(a: T, b: T) -> T {
        Animatable::new(a.clone())
            .with_key(t(0), a)
            .with_key(t(100), b)
            .value_at(t(50))
    }

    #[test]
    fn every_value_type_animates() {
        assert_eq!(
            mid(PerMille::new(0), PerMille::new(1000)),
            PerMille::new(500)
        );
        assert_eq!(
            mid(Scale::from_percent(100), Scale::from_percent(200)),
            Scale::from_percent(150)
        );
        assert_eq!(
            mid(Angle::from_degrees(0), Angle::from_degrees(720)),
            Angle::from_degrees(360),
            "rotation interpolates numerically"
        );
        assert_eq!(
            mid(CanvasPos::new(0, 100), CanvasPos::new(1000, -100)),
            CanvasPos::new(500, 0)
        );
        assert_eq!(
            mid(RefPx::from_px(10), RefPx::from_px(11)),
            RefPx::from_hundredths(1050)
        );
        assert_eq!(
            mid(SignedPerMille::new(-200), SignedPerMille::new(400)),
            SignedPerMille::new(100)
        );
        assert_eq!(
            mid(DeciDegrees::new(3500), DeciDegrees::new(100)),
            DeciDegrees::ZERO,
            "the shorter way round"
        );
        let tip = |x, y| TailTip {
            x: RefPx::from_px(x),
            y: RefPx::from_px(y),
        };
        assert_eq!(mid(tip(0, 10), tip(20, -10)), tip(10, 0));
        assert_eq!(
            mid(Rgba8::new(0, 0, 0, 0), Rgba8::new(255, 255, 255, 255)),
            Rgba8::new(188, 188, 188, 128),
            "linear light: half the light is sRGB 188"
        );
    }

    #[test]
    fn colours_mix_in_linear_light_and_round_trip() {
        for v in 0..=255u8 {
            assert_eq!(linear_to_srgb(srgb_to_linear(v)), v);
        }
        let red = Rgba8::new(255, 0, 0, 255);
        let green = Rgba8::new(0, 255, 0, 255);
        let m = Rgba8::interpolate(&red, &green, half());
        assert_eq!((m.r, m.g, m.b, m.a), (188, 188, 0, 255));
        // Overshoot stays in gamut.
        let over = Rgba8::interpolate(&red, &green, Progress::from_eased(1.3));
        assert_eq!((over.r, over.g), (0, 255));
    }

    #[test]
    fn overshoot_saturates_at_the_type_width() {
        let p = Progress::from_eased(1.5);
        assert_eq!(
            PerMille::interpolate(&PerMille::new(0), &PerMille::new(u16::MAX), p),
            PerMille::new(u16::MAX)
        );
        let below = Progress::from_eased(-0.5);
        assert_eq!(
            PerMille::interpolate(&PerMille::new(0), &PerMille::new(1000), below),
            PerMille::ZERO
        );
        assert_eq!(
            SignedPerMille::interpolate(&SignedPerMille::new(0), &SignedPerMille::new(i16::MAX), p),
            SignedPerMille::new(i16::MAX)
        );
    }
}
