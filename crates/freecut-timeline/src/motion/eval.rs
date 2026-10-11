//! Pure evaluation of presets (FX-018, FX-030, TX-022): where a slot is at a
//! time, and the offsets its channels give there.
//!
//! Time stays rational up to here; progress becomes an `f64` only to feed
//! the curves, and the offsets are floats in canvas pixels, degrees and
//! factors, for the render edge. The same [`pose_at`] serves the whole
//! block (folded into the layer transform by [`crate::plan`]) and each
//! unit of a per-unit text preset (applied by the renderer, which knows
//! the layout).

use glam::DVec2;

use super::manifest::{
    ChannelProp, ChannelUnit, Curve, CurvePoint, Pivot, PresetKind, PresetManifest, StyleChannel,
};
use super::seeded::{self, FLICKER_DIP_SECS, Stream};
use crate::model::{AnimUnit, AnimationSpec, Direction, Interp, Slot, SlotWindow, Travel};
use crate::time::Tick;

/// The layer at rest, for distances (FX-030): its axis-aligned bounds on
/// the canvas and the length of an em, all in canvas pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RestBounds {
    /// Canvas width and height.
    pub canvas: DVec2,
    /// The layer's bounds at rest on the canvas.
    pub min: DVec2,
    pub max: DVec2,
    /// One em: the font size on text, 1/16 of the short edge elsewhere.
    pub em_px: f64,
    /// Text uses ems for the Short travel (3·k em instead of 0.15·k short
    /// edges, TX-056).
    pub text: bool,
}

impl RestBounds {
    /// The canvas short edge.
    pub fn short_edge(&self) -> f64 {
        self.canvas.x.min(self.canvas.y)
    }
}

/// What channel units measure in canvas pixels, for one evaluation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionScale {
    pub se_px: f64,
    pub em_px: f64,
    /// The slide distance of [`ChannelUnit::Travel`] (positive).
    pub travel_px: f64,
}

/// The offsets of one slot at one instant (FX-018): for the whole block,
/// or for one unit of text. Positions add, factors multiply, rotations add
/// (FX-030); [`BlockOffsets::REST`] changes nothing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlockOffsets {
    /// Canvas pixels, +y down.
    pub translate: DVec2,
    /// Width and height factors.
    pub scale: DVec2,
    /// Degrees, clockwise.
    pub rotation: f64,
    /// 3D turns in degrees: `x` about the horizontal axis, `y` about the
    /// vertical axis, with FX-030's perspective.
    pub turn: DVec2,
    /// 0..=1, multiplies the layer opacity.
    pub opacity: f64,
    /// Blur radius in canvas pixels.
    pub blur: f64,
    /// The shown share, 0..=1: a wipe from `reveal_from` on a block, a
    /// per-unit reveal on text.
    pub reveal: f64,
    pub reveal_from: Direction,
    /// White tint, 0..=1.
    pub tint: f64,
    /// Decode's substitution amount (TX-056).
    pub decode: f64,
    /// Style modulation (Loops, TX-022 step 1): hue turn in degrees, glow
    /// intensity factor and fill lightening.
    pub hue: f64,
    pub glow: f64,
    pub lightness: f64,
    pub pivot: Pivot,
    /// The unit is drawn clipped to its line's box (Word rise's slot,
    /// [`ChannelProp::LineRise`]); set only while it is moved by that
    /// channel.
    pub line_clip: bool,
}

impl BlockOffsets {
    pub const REST: BlockOffsets = BlockOffsets {
        translate: DVec2::ZERO,
        scale: DVec2::ONE,
        rotation: 0.0,
        turn: DVec2::ZERO,
        opacity: 1.0,
        blur: 0.0,
        reveal: 1.0,
        reveal_from: Direction::Left,
        tint: 0.0,
        decode: 0.0,
        hue: 0.0,
        glow: 1.0,
        lightness: 0.0,
        pivot: Pivot::Anchor,
        line_clip: false,
    };

    /// Whether every offset is at rest, within `eps`.
    pub fn is_rest(&self, eps: f64) -> bool {
        let r = BlockOffsets::REST;
        let near = |a: f64, b: f64| (a - b).abs() <= eps;
        self.translate.abs_diff_eq(r.translate, eps)
            && self.scale.abs_diff_eq(r.scale, eps)
            && near(self.rotation, r.rotation)
            && self.turn.abs_diff_eq(r.turn, eps)
            && near(self.opacity, r.opacity)
            && near(self.blur, r.blur)
            && near(self.reveal, r.reveal)
            && near(self.tint, r.tint)
            && near(self.decode, r.decode)
            && near(self.hue, r.hue)
            && near(self.glow, r.glow)
            && near(self.lightness, r.lightness)
            && !self.line_clip
    }

    /// Whether the offsets need a 3D turn (a projective transform).
    pub fn has_turn(&self) -> bool {
        self.turn.x != 0.0 || self.turn.y != 0.0
    }
}

impl Default for BlockOffsets {
    fn default() -> BlockOffsets {
        BlockOffsets::REST
    }
}

/// How the slot's time drives the manifest's curves.
#[derive(Debug, Clone, Copy)]
enum Ease {
    Plain(Interp),
    /// Played backwards in time: `1 − e(1 − p)`.
    Mirrored(Interp),
}

impl Ease {
    fn apply(self, p: f64) -> f64 {
        match self {
            Ease::Plain(i) => i.ease(p),
            Ease::Mirrored(i) => i.ease_mirrored(p),
        }
    }
}

/// The unit a spec moves (TX-023): its own choice when the preset offers
/// it, else the preset's default; [`AnimUnit::Block`] for whole-block
/// presets and for layers that aren't text.
pub fn effective_unit(manifest: &PresetManifest, spec: &AnimationSpec, text: bool) -> AnimUnit {
    let Some(units) = manifest.units.as_ref().filter(|_| text) else {
        return AnimUnit::Block;
    };
    match spec.params.unit {
        Some(u) if units.contains(&u) => u,
        _ => units.first().copied().unwrap_or(AnimUnit::Block),
    }
}

/// The direction a spec plays in: its own, or the preset's default. An
/// exit that plays its entrance backwards defaults to the opposite side,
/// so a default In and Out read as one movement (FX-030). `None` for
/// presets without directions.
pub fn effective_direction(
    manifest: &PresetManifest,
    spec: &AnimationSpec,
    slot: Slot,
) -> Option<Direction> {
    let dirs = manifest.directions.as_ref()?;
    let default = dirs.first().copied()?;
    let default = if slot == Slot::Exit && manifest.kind == PresetKind::Entrance {
        default.opposite()
    } else {
        default
    };
    Some(
        spec.direction
            .filter(|d| dirs.contains(d))
            .unwrap_or(default),
    )
}

/// Slot progress in `[0, 1]` at clip-local time `local_t` (FX-018,
/// FX-019): the share of `window` played for In, Out, drifts and combined
/// presets; the position in the cycle for loops, measured from the clip's
/// start plus the spec's phase (so a split continues seamlessly).
pub fn slot_progress(
    manifest: &PresetManifest,
    spec: &AnimationSpec,
    local_t: Tick,
    window: SlotWindow,
) -> f64 {
    if manifest.kind == PresetKind::Loop {
        let cycle = spec.duration.raw();
        if cycle <= 0 {
            return 0.0;
        }
        let elapsed = i128::from(local_t.raw()) + i128::from(spec.phase.raw());
        let r = elapsed.rem_euclid(i128::from(cycle));
        return r as f64 / cycle as f64;
    }
    let len = window.len.raw();
    if len <= 0 {
        return 1.0;
    }
    let num = i128::from(local_t.raw()) - i128::from(window.start.raw());
    (num as f64 / len as f64).clamp(0.0, 1.0)
}

/// What posing one unit needs besides its progress (TX-023, TX-056): which
/// unit it is, for seeded channels, and where it sits in its line, for
/// Spread and Word rise. Lengths are in the units [`MotionScale`] measures
/// in: canvas pixels for the plan's whole-block poses, em for the
/// renderer's per-unit poses (it passes `em_px = 1`).
///
/// **What the renderer supplies per unit** (from its layout, TX-023):
/// `index`, the unit's index in reading order among timed units (the same
/// index TX-023's order permutes, so a unit keeps its seeded motion
/// whatever the order); `to_line_center`, the vector from the unit's own
/// centre (ink-box centre for letters, box centre for words and lines) to
/// its line's box centre; `line_height`, its line's height. The plan's
/// [`crate::plan::TextAnimPlan`] supplies `frame` and has already applied
/// `amount`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnitContext {
    /// The unit's index in reading order; 0 for a whole block.
    pub index: u32,
    /// From the unit's centre to its line's centre (0 for a block, which
    /// then never moves by [`ChannelProp::TowardLineCenter`]).
    pub to_line_center: DVec2,
    /// The unit's line height; `None` takes 1.2 em.
    pub line_height: Option<f64>,
    /// The sequence frame duration: Flicker dips last 2 frames (TX-056);
    /// `None` takes 2 frames at 30 fps.
    pub frame: Option<Tick>,
    /// The share of every amplitude that plays, 0..=1: TX-022's seam
    /// envelope ([`seam_envelope`]); 1 normally.
    pub amount: f64,
}

impl UnitContext {
    /// A whole block (or the first unit) at full strength.
    pub const BLOCK: UnitContext = UnitContext {
        index: 0,
        to_line_center: DVec2::ZERO,
        line_height: None,
        frame: None,
        amount: 1.0,
    };
}

impl Default for UnitContext {
    fn default() -> UnitContext {
        UnitContext::BLOCK
    }
}

/// The offsets of `manifest` played by `spec` in `slot` at progress `p`
/// (FX-030) for a whole block or the first unit: [`pose_with`] and
/// [`UnitContext::BLOCK`].
pub fn pose_at(
    manifest: &PresetManifest,
    spec: &AnimationSpec,
    slot: Slot,
    p: f64,
    scale: &MotionScale,
) -> BlockOffsets {
    pose_with(manifest, spec, slot, p, scale, &UnitContext::BLOCK)
}

/// The offsets of `manifest` played by `spec` in `slot` at progress `p` for
/// one unit (FX-030): each channel's curve times its amplitude (times `k`
/// when it scales with intensity, times `unit.amount`), in canvas pixels
/// per `scale`. Entrances end at rest at `p = 1`; exits start at rest at
/// `p = 0` (those without a manifest of their own play their entrance
/// backwards, easing mirrored). Translations are written for a slide from
/// the left and turned to the effective direction; Right and
/// CounterClockwise mirror rotations; [`ChannelProp::TurnFromSide`] picks
/// its axis from the direction. Seeded channels draw from
/// `spec.params.seed` and `unit.index`; jitter, flicker and sparkle run on
/// `spec.duration` (the cycle of a loop), so a loop stays periodic.
pub fn pose_with(
    manifest: &PresetManifest,
    spec: &AnimationSpec,
    slot: Slot,
    p: f64,
    scale: &MotionScale,
    unit: &UnitContext,
) -> BlockOffsets {
    let p = if p.is_nan() { 0.0 } else { p.clamp(0.0, 1.0) };
    let amount = if unit.amount.is_nan() {
        1.0
    } else {
        unit.amount.clamp(0.0, 1.0)
    };
    let base = spec.easing.unwrap_or(manifest.easing);
    // (progress the curves see, easing, curves written as an exit)
    let (q, ease, exit_curves) = match (manifest.kind, slot) {
        (PresetKind::Entrance, Slot::Exit) => (
            1.0 - p,
            spec.easing
                .map_or(Ease::Plain(manifest.easing), Ease::Mirrored),
            false,
        ),
        (PresetKind::Exit, Slot::Exit) => (p, Ease::Plain(base), true),
        (PresetKind::Exit, Slot::Entrance) => (
            1.0 - p,
            spec.easing
                .map_or(Ease::Plain(manifest.easing), Ease::Mirrored),
            true,
        ),
        _ => (p, Ease::Plain(base), false),
    };
    let k = spec.params.k();
    let seed = spec.params.seed;
    let span = spec.duration.as_secs_f64();
    let dip = unit
        .frame
        .map_or(FLICKER_DIP_SECS, |f| 2.0 * f.as_secs_f64());
    let direction = effective_direction(manifest, spec, slot);
    let mut out = BlockOffsets {
        pivot: manifest.pivot,
        reveal_from: direction.unwrap_or(Direction::Left),
        ..BlockOffsets::REST
    };
    // Written for a slide from the left; turned to the direction below.
    let mut translate = DVec2::ZERO;
    let mut rotation = 0.0;
    // Seeded and line-relative motion: the same from every side.
    let mut free_translate = DVec2::ZERO;
    let mut free_rotation = 0.0;
    let mut side_turn = 0.0;
    let mut rise = 0.0;
    for ch in &manifest.channels {
        let c = curve_value(&ch.curve, q, ease, exit_curves);
        let travel_scales = ch.unit == ChannelUnit::Travel && spec.params.travel == Travel::Short;
        let gain = if ch.scales_with_intensity || travel_scales {
            k
        } else {
            1.0
        };
        let unit_px = match ch.unit {
            ChannelUnit::Se => scale.se_px,
            ChannelUnit::Em => scale.em_px,
            ChannelUnit::Travel => scale.travel_px,
            ChannelUnit::Deg | ChannelUnit::Ratio => 1.0,
        };
        let v = ch.amplitude * c * gain * unit_px * amount;
        let i = unit.index;
        match ch.property {
            ChannelProp::TranslateX => translate.x += v,
            ChannelProp::TranslateY => translate.y += v,
            ChannelProp::Scale => out.scale *= 1.0 + v,
            ChannelProp::ScaleX => out.scale.x *= 1.0 + v,
            ChannelProp::ScaleY => out.scale.y *= 1.0 + v,
            ChannelProp::Rotate => rotation += v,
            ChannelProp::TurnX => out.turn.x += v,
            ChannelProp::TurnY => out.turn.y += v,
            ChannelProp::Opacity => out.opacity *= 1.0 + v,
            ChannelProp::Blur => out.blur += v,
            ChannelProp::Reveal => out.reveal *= 1.0 + v,
            ChannelProp::Tint => out.tint += v,
            ChannelProp::Decode => out.decode += v,
            ChannelProp::Style(StyleChannel::Hue) => out.hue += v,
            ChannelProp::Style(StyleChannel::GlowIntensity) => out.glow *= 1.0 + v,
            ChannelProp::Style(StyleChannel::Lightness) => out.lightness += v,
            ChannelProp::RandomOffset => free_translate += seeded::scatter(seed, i) * v,
            ChannelProp::RandomRotate => {
                free_rotation += v * seeded::signed(seed, i, Stream::Rotate);
            }
            ChannelProp::TowardLineCenter => free_translate += unit.to_line_center * v,
            ChannelProp::LineRise => {
                // A share of the line height, or a length in its own unit.
                rise += if ch.unit == ChannelUnit::Ratio {
                    v * unit.line_height.unwrap_or(1.2 * scale.em_px)
                } else {
                    v
                };
            }
            ChannelProp::JitterX => {
                free_translate.x += v * seeded::jitter(seed, i, Stream::JitterX, q, span);
            }
            ChannelProp::JitterY => {
                free_translate.y += v * seeded::jitter(seed, i, Stream::JitterY, q, span);
            }
            ChannelProp::JitterRotate => {
                free_rotation += v * seeded::jitter(seed, i, Stream::JitterRotate, q, span);
            }
            ChannelProp::FlickerOpacity => {
                out.opacity *= 1.0 - v * seeded::flicker_dip(seed, i, q, span, dip);
            }
            ChannelProp::SparkleScale => out.scale *= 1.0 + v * seeded::sparkle(seed, i, q, span),
            ChannelProp::TurnFromSide => side_turn += v,
        }
    }
    out.translate = match direction {
        Some(Direction::Right) => DVec2::new(-translate.x, translate.y),
        Some(Direction::Up) => DVec2::new(translate.y, translate.x),
        Some(Direction::Down) => DVec2::new(-translate.y, -translate.x),
        _ => translate,
    } + free_translate;
    out.rotation = match direction {
        Some(Direction::Right | Direction::CounterClockwise) => -rotation,
        _ => rotation,
    } + free_rotation;
    if rise != 0.0 {
        out.translate.y += rise;
        out.line_clip = true;
    }
    match direction {
        Some(Direction::Right) => out.turn.y -= side_turn,
        Some(Direction::Up) => out.turn.x += side_turn,
        Some(Direction::Down) => out.turn.x -= side_turn,
        _ => out.turn.y += side_turn,
    }
    out.scale = out.scale.max(DVec2::ZERO);
    out.opacity = out.opacity.clamp(0.0, 1.0);
    out.reveal = out.reveal.clamp(0.0, 1.0);
    out.blur = out.blur.max(0.0);
    out.tint = out.tint.clamp(0.0, 1.0);
    out.glow = out.glow.max(0.0);
    out
}

/// TX-022's seam rule: the share of a Continuous slot's amplitude that
/// plays at clip-local `local_t`, for `spec` playing in `window` on a clip
/// of `clip_len`. Where the window starts after the clip start (an In
/// plays before it), the amplitude eases in over the first
/// `min(250 ms, window / 4)`; where it ends before the clip end (an Out
/// follows), it eases out over the last `min(250 ms, window / 4)`, both
/// with smoothstep. So the pose is at rest where In hands over and where
/// Out takes over, and a Continuous slot over the whole clip is
/// untouched. A split piece keeps the seams and ramps of the unsplit clip
/// ([`AnimationSpec::seams_in`]), so a split changes no frame.
pub fn seam_envelope(
    spec: &AnimationSpec,
    window: SlotWindow,
    clip_len: Tick,
    local_t: Tick,
) -> f64 {
    let Some(seams) = spec.seams_in(window, clip_len) else {
        return 1.0;
    };
    let ramp = seams.ramp.raw();
    if ramp <= 0 {
        return 1.0;
    }
    let ramp = i128::from(ramp);
    let share = |ticks: i128| {
        let x = (ticks.clamp(0, ramp) as f64) / (ramp as f64);
        x * x * (3.0 - 2.0 * x)
    };
    // Loop time: clip-local time plus the phase, as the seams are kept.
    let t = i128::from(local_t.raw()) + i128::from(spec.phase.raw());
    let mut e = 1.0;
    if let Some(start) = seams.start {
        e *= share(t - i128::from(start.raw()));
    }
    if let Some(end) = seams.end {
        e *= share(i128::from(end.raw()) - t);
    }
    e
}

/// One channel's weight at progress `q`.
fn curve_value(curve: &Curve, q: f64, ease: Ease, exit_curves: bool) -> f64 {
    let toward_rest = |e: f64| if exit_curves { e } else { 1.0 - e };
    match curve {
        Curve::Eased => toward_rest(ease.apply(q)),
        Curve::Ease(i) => toward_rest(i.ease(q)),
        Curve::Sampled { points, eased } => {
            monotone_cubic(points, if *eased { ease.apply(q) } else { q })
        }
        Curve::Sine => (std::f64::consts::TAU * q).sin(),
        Curve::Step => {
            let started = q > 0.0;
            if exit_curves == started { 1.0 } else { 0.0 }
        }
    }
}

/// Fritsch–Carlson monotone cubic interpolation through `points`, flat
/// outside them.
fn monotone_cubic(points: &[CurvePoint], x: f64) -> f64 {
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return 0.0;
    };
    if points.len() == 1 || x <= first.t {
        return first.v;
    }
    if x >= last.t {
        return last.v;
    }
    let n = points.len();
    let secant = |i: usize| {
        let dt = points[i + 1].t - points[i].t;
        if dt > 0.0 {
            (points[i + 1].v - points[i].v) / dt
        } else {
            0.0
        }
    };
    let tangent = |i: usize| -> f64 {
        if i == 0 {
            return secant(0);
        }
        if i == n - 1 {
            return secant(n - 2);
        }
        let (a, b) = (secant(i - 1), secant(i));
        if a * b <= 0.0 {
            0.0
        } else {
            // Harmonic mean keeps the curve monotone between points.
            2.0 * a * b / (a + b)
        }
    };
    // The segment holding x: points[i].t <= x < points[i + 1].t.
    let i = points
        .partition_point(|p| p.t <= x)
        .saturating_sub(1)
        .min(n - 2);
    let (p0, p1) = (points[i], points[i + 1]);
    let h = p1.t - p0.t;
    if h <= 0.0 {
        return p1.v;
    }
    let delta = secant(i);
    let (mut m0, mut m1) = (tangent(i), tangent(i + 1));
    if delta == 0.0 {
        m0 = 0.0;
        m1 = 0.0;
    } else {
        let (a, b) = (m0 / delta, m1 / delta);
        let s = a * a + b * b;
        if s > 9.0 {
            let tau = 3.0 / s.sqrt();
            m0 = tau * a * delta;
            m1 = tau * b * delta;
        }
    }
    let t = (x - p0.t) / h;
    let (t2, t3) = (t * t, t * t * t);
    let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
    let h10 = t3 - 2.0 * t2 + t;
    let h01 = -2.0 * t3 + 3.0 * t2;
    let h11 = t3 - t2;
    h00 * p0.v + h10 * h * m0 + h01 * p1.v + h11 * h * m1
}

/// The slide distance in canvas pixels (FX-030): **From edge** moves the
/// layer until its bounds at rest sit just past the canvas edge on
/// `direction`'s side, at least one layer size so a layer already
/// off-canvas still moves; **Short** is 0.15·k short edges (3·k em on
/// text; `k` is applied by [`pose_at`]).
pub fn travel_px(direction: Option<Direction>, travel: Travel, rest: &RestBounds) -> f64 {
    if travel == Travel::Short {
        return if rest.text {
            3.0 * rest.em_px
        } else {
            0.15 * rest.short_edge()
        };
    }
    let size = rest.max - rest.min;
    match direction.unwrap_or(Direction::Left) {
        Direction::Left => rest.max.x.max(size.x),
        Direction::Right => (rest.canvas.x - rest.min.x).max(size.x),
        Direction::Up => rest.max.y.max(size.y),
        Direction::Down => (rest.canvas.y - rest.min.y).max(size.y),
        Direction::Clockwise | Direction::CounterClockwise => 0.0,
    }
}

/// The whole-block offsets of `manifest` played by `spec` in `slot`, at
/// clip-local time `local_t`, for a slot playing in `window` (FX-018's
/// `animation_offsets`): pure, so preview and export agree. `rest` gives
/// the distances.
pub fn animation_offsets(
    manifest: &PresetManifest,
    spec: &AnimationSpec,
    slot: Slot,
    local_t: Tick,
    window: SlotWindow,
    rest: &RestBounds,
) -> BlockOffsets {
    animation_offsets_with(
        manifest,
        spec,
        slot,
        local_t,
        window,
        rest,
        &UnitContext::BLOCK,
    )
}

/// [`animation_offsets`] with a [`UnitContext`]: the plan passes the frame
/// duration and TX-022's seam envelope ([`seam_envelope`]).
pub fn animation_offsets_with(
    manifest: &PresetManifest,
    spec: &AnimationSpec,
    slot: Slot,
    local_t: Tick,
    window: SlotWindow,
    rest: &RestBounds,
    unit: &UnitContext,
) -> BlockOffsets {
    let p = slot_progress(manifest, spec, local_t, window);
    let direction = effective_direction(manifest, spec, slot);
    let scale = MotionScale {
        se_px: rest.short_edge(),
        em_px: rest.em_px,
        travel_px: travel_px(direction, spec.params.travel, rest),
    };
    pose_with(manifest, spec, slot, p, &scale, unit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::TICKS_PER_SECOND;

    #[test]
    fn monotone_cubic_passes_through_points_and_never_overshoots() {
        let pts = [
            CurvePoint { t: 0.0, v: 0.0 },
            CurvePoint { t: 0.3, v: 1.0 },
            CurvePoint { t: 0.6, v: 1.0 },
            CurvePoint { t: 1.0, v: 0.0 },
        ];
        for p in pts {
            assert!((monotone_cubic(&pts, p.t) - p.v).abs() < 1e-12);
        }
        for k in 0..=1000 {
            let v = monotone_cubic(&pts, f64::from(k) / 1000.0);
            assert!((-1e-12..=1.0 + 1e-12).contains(&v), "{k}: {v}");
        }
        assert_eq!(monotone_cubic(&pts, -1.0), 0.0);
        assert_eq!(monotone_cubic(&pts[..1], 0.5), 0.0);
        assert_eq!(monotone_cubic(&[], 0.5), 0.0);
    }

    #[test]
    fn the_seam_envelope_ramps_only_next_to_in_and_out() {
        let ms = |v: i64| Tick::from_raw(TICKS_PER_SECOND / 1000 * v);
        let clip = ms(4000);
        let looped = AnimationSpec::new(crate::model::PresetId::new("test.loop"), ms(1000));
        // A Loop between In (0.4 s) and Out (0.4 s): 250 ms ramps.
        let w = SlotWindow {
            start: ms(400),
            len: ms(3200),
        };
        assert_eq!(seam_envelope(&looped, w, clip, ms(400)), 0.0);
        assert_eq!(seam_envelope(&looped, w, clip, ms(525)), 0.5);
        assert_eq!(seam_envelope(&looped, w, clip, ms(650)), 1.0);
        assert_eq!(seam_envelope(&looped, w, clip, ms(2000)), 1.0);
        assert_eq!(seam_envelope(&looped, w, clip, ms(3350)), 1.0);
        assert_eq!(seam_envelope(&looped, w, clip, ms(3475)), 0.5);
        assert_eq!(seam_envelope(&looped, w, clip, ms(3600)), 0.0);
        // Monotone and continuous through the ramp.
        let mut last = 0.0;
        for t in 400..=650 {
            let e = seam_envelope(&looped, w, clip, ms(t));
            assert!(e >= last && e - last < 0.02, "{t}: {last} → {e}");
            last = e;
        }
        // A short window ramps over a quarter of it each way.
        let short = SlotWindow {
            start: ms(100),
            len: ms(400),
        };
        assert_eq!(seam_envelope(&looped, short, ms(600), ms(200)), 1.0);
        assert_eq!(seam_envelope(&looped, short, ms(600), ms(150)), 0.5);
        // The whole clip, or an edge of it: no ramp there.
        let whole = SlotWindow {
            start: Tick::ZERO,
            len: clip,
        };
        assert_eq!(seam_envelope(&looped, whole, clip, Tick::ZERO), 1.0);
        assert_eq!(seam_envelope(&looped, whole, clip, ms(3999)), 1.0);
        let after_in = SlotWindow {
            start: ms(400),
            len: ms(3600),
        };
        assert_eq!(seam_envelope(&looped, after_in, clip, ms(3999)), 1.0);
        // Degenerate windows play as they are.
        let tiny = SlotWindow {
            start: ms(1),
            len: Tick::from_raw(3),
        };
        assert_eq!(seam_envelope(&looped, tiny, clip, ms(1)), 1.0);
    }

    #[test]
    fn amount_scales_every_channel_and_block_is_the_default() {
        let m = &super::super::manifest::samples()[1]; // Rise
        let spec = m.default_spec();
        let scale = MotionScale {
            se_px: 1.0,
            em_px: 100.0,
            travel_px: 1.0,
        };
        let full = pose_at(m, &spec, Slot::Entrance, 0.0, &scale);
        assert_eq!(
            full,
            pose_with(
                m,
                &spec,
                Slot::Entrance,
                0.0,
                &scale,
                &UnitContext::default()
            )
        );
        let half = pose_with(
            m,
            &spec,
            Slot::Entrance,
            0.0,
            &scale,
            &UnitContext {
                amount: 0.5,
                ..UnitContext::BLOCK
            },
        );
        assert_eq!(half.translate.y, full.translate.y / 2.0);
        assert_eq!(half.opacity, 0.5);
        let none = UnitContext {
            amount: 0.0,
            ..UnitContext::BLOCK
        };
        assert!(pose_with(m, &spec, Slot::Entrance, 0.0, &scale, &none).is_rest(0.0));
    }
}
