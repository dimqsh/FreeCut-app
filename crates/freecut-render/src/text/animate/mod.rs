//! Per-unit text animation (TX-022 step 3, TX-023, TX-056): units, their
//! time windows and play order, and every glyph's pose at a time `t`.
//!
//! This is the renderer's own contract, decoupled from the timeline's
//! animation model: a [`TextMotion`] names the unit kind, the slot timing
//! (In or Out windows, or a Loop's phase delays), the order, the slot-local
//! time and the per-unit channel values, given either as a small
//! [`ChannelTable`] or by a callback ([`UnitFn`]) that evaluates the preset
//! (the timeline's manifest evaluator, through an adapter). Evaluating it
//! against a layout gives a [`MotionState`]: one [`GlyphState`] per glyph,
//! which [`crate::text::TextRenderer::rasterize_animated`] draws.
//!
//! **Time stays exact.** Windows, delays and decode steps are integer
//! [`Tick`] arithmetic; a unit's progress is a ratio of ticks turned into
//! an `f32` only when it is handed to the channels.
//!
//! **Frozen algorithms.** The play order's random shuffle
//! ([`SplitMix64`] + Fisher–Yates, bounded draws by Lemire's multiply),
//! [`unit_noise`] and the Decode stand-in choice are part of the look of
//! saved projects; golden vectors in the tests pin them (text architecture
//! R6). `rand` is never used: its generators are not reproducible across
//! releases.

mod units;

use std::fmt;
use std::sync::Arc;

use freecut_timeline::model::PerMille;
use freecut_timeline::time::Tick;

pub use units::{UnitMap, is_joining_script};

use crate::text::TextLayout;

/// What moves on its own (TX-023's Unit).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum AnimUnit {
    /// Extended grapheme clusters (letters).
    #[default]
    Grapheme,
    /// UAX #29 words; spaces and punctuation go with the word before them.
    Word,
    /// Laid-out lines.
    Line,
}

/// The order units play in (TX-023).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum UnitOrder {
    /// Reading order.
    #[default]
    Forward,
    Backward,
    /// From the middle outwards; of two units equally far from the
    /// middle, the left (earlier) one first.
    CenterOut,
    /// The first and last units, then the second and second-to-last, …,
    /// ending in the middle.
    EdgesIn,
    /// A seeded shuffle ([`SplitMix64`] + Fisher–Yates).
    Random {
        seed: u32,
    },
}

/// How a slot's time is shared among the units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnitTiming {
    /// In: `span` is split among the timed units ([`unit_windows`]); a
    /// unit's progress runs from 0 (its start pose) at its window's start
    /// to 1 (rest) at its end.
    In { span: Tick, overlap: PerMille },
    /// Out: the same windows, each unit's motion played backwards in time
    /// (progress 1 → 0, so the easing is mirrored), the units still in
    /// play order (TX-023).
    Out { span: Tick, overlap: PerMille },
    /// Loop: the unit at play position `r` runs the cycle delayed by
    /// [`loop_delay`]; its progress is the phase, 0..1, wrapping.
    Loop { cycle: Tick, overlap: PerMille },
}

impl UnitTiming {
    fn windows(&self, n: u32) -> Option<Vec<(Tick, Tick)>> {
        match *self {
            UnitTiming::In { span, overlap } | UnitTiming::Out { span, overlap } => {
                Some(unit_windows(span, n, overlap))
            }
            UnitTiming::Loop { .. } => None,
        }
    }
}

/// One glyph's animated pose: its unit's channel values at `t`, applied
/// in the text's local frame about the unit's own centre (TX-023). The
/// default ([`GlyphAnim::REST`]) changes nothing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphAnim {
    /// Offset in em (fractions of the font size), x right, y down.
    pub offset: (f32, f32),
    /// Uniform scale factor.
    pub scale: f32,
    /// Per-axis scale factors on top of `scale`.
    pub scale_xy: (f32, f32),
    /// Clockwise rotation, degrees.
    pub rotation: f32,
    /// 3D turns (degrees) about the unit's horizontal (x) and vertical (y)
    /// axes through its centre, in perspective with the eye 1.5 × the
    /// unit's width away (FX-030); applied before the rotation.
    pub turn: (f32, f32),
    /// 0..=1.
    pub opacity: f32,
    /// Gaussian blur radius in em.
    pub blur: f32,
    /// The fraction of the unit shown, 0..=1, from its reading-order start
    /// edge; it multiplies with the Reveal keyframe (TX-021) and drives the
    /// background box (TX-013).
    pub reveal: f32,
    /// How far the glyph's fill moves toward [`TextMotion::tint_color`],
    /// 0..=1 (per-unit style channels: Shimmer's band of brightness is a
    /// tint toward white that travels across the units). Outline, glow
    /// and shadow keep their colours.
    pub tint: f32,
    /// Hue turn of the glyph's fill and outline in degrees, lightness and
    /// saturation kept (the per-unit form of Hue cycle's style channel);
    /// applied before the tint.
    pub hue: f32,
    /// Factor on the glow's intensity around the glyph (the per-unit form
    /// of Breathing glow's style channel); 1 leaves it as set. The frame
    /// plan turns on a temporary glow when the text has none (TX-056).
    pub glow: f32,
    /// The glyph is drawn clipped to its line's box at rest (Word rise's
    /// slot, TX-056): ink moved above or below the line is cut away.
    pub line_clip: bool,
}

impl GlyphAnim {
    pub const REST: GlyphAnim = GlyphAnim {
        offset: (0.0, 0.0),
        scale: 1.0,
        scale_xy: (1.0, 1.0),
        rotation: 0.0,
        turn: (0.0, 0.0),
        opacity: 1.0,
        blur: 0.0,
        reveal: 1.0,
        tint: 0.0,
        hue: 0.0,
        glow: 1.0,
        line_clip: false,
    };

    /// Non-finite values replaced by rest, opacity, reveal and tint
    /// clamped to 0..=1, blur to 0..=4 em, offsets to ±64 em, scales to
    /// ±64, angles (the hue turn too) to ±36,000°, the glow factor to
    /// 0..=64.
    pub fn sanitized(self) -> GlyphAnim {
        let f = |v: f32, rest: f32, lo: f32, hi: f32| {
            if v.is_finite() { v.clamp(lo, hi) } else { rest }
        };
        GlyphAnim {
            offset: (
                f(self.offset.0, 0.0, -64.0, 64.0),
                f(self.offset.1, 0.0, -64.0, 64.0),
            ),
            scale: f(self.scale, 1.0, -64.0, 64.0),
            scale_xy: (
                f(self.scale_xy.0, 1.0, -64.0, 64.0),
                f(self.scale_xy.1, 1.0, -64.0, 64.0),
            ),
            rotation: f(self.rotation, 0.0, -36_000.0, 36_000.0),
            turn: (
                f(self.turn.0, 0.0, -36_000.0, 36_000.0),
                f(self.turn.1, 0.0, -36_000.0, 36_000.0),
            ),
            opacity: f(self.opacity, 1.0, 0.0, 1.0),
            blur: f(self.blur, 0.0, 0.0, 4.0),
            reveal: f(self.reveal, 1.0, 0.0, 1.0),
            tint: f(self.tint, 0.0, 0.0, 1.0),
            hue: f(self.hue, 0.0, -36_000.0, 36_000.0),
            glow: f(self.glow, 1.0, 0.0, 64.0),
            line_clip: self.line_clip,
        }
    }

    /// Whether only the offset differs from rest in the glyph's shape
    /// (opacity, blur and reveal aside): the glyph is drawn crisp at a
    /// moved pen instead of resampled.
    pub fn is_translation(&self) -> bool {
        self.scale == 1.0
            && self.scale_xy == (1.0, 1.0)
            && self.rotation == 0.0
            && self.turn == (0.0, 0.0)
    }
}

impl Default for GlyphAnim {
    fn default() -> GlyphAnim {
        GlyphAnim::REST
    }
}

/// What the channels are told about one timed unit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnitSample {
    /// The unit's index in reading order (among timed units).
    pub unit: u32,
    /// Its position in play order.
    pub rank: u32,
    /// How many timed units there are.
    pub count: u32,
    /// In and Out: the pose's progress, 0 = start pose, 1 = rest (already
    /// reversed for Out). Loop: the phase, 0..1.
    pub progress: f32,
    /// In and Out: the unit's window in slot-local time. Loop: its delay
    /// and the delay plus one cycle.
    pub window: (Tick, Tick),
    /// The unit rests in its start pose outside its window: In before the
    /// window starts, Out from its end. (Progress alone can't tell "just
    /// started" from "not started": both are 0.)
    pub idle: bool,
    /// Where the unit sits in its line (TX-056's Spread and Word rise).
    pub geometry: UnitGeometry,
}

/// A timed unit's place in its line, in em of the text
/// ([`TextLayout::em`]), from the rest layout.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct UnitGeometry {
    /// From the centre of the unit's box (its glyphs' advances, from the
    /// line's top to its bottom) to the centre of its line's box (the
    /// line's content after alignment, top to bottom); x right, y down.
    pub to_line_center: (f32, f32),
    /// The height of the unit's line; 0 when the unit has no box.
    pub line_height: f32,
}

impl UnitGeometry {
    /// The geometry of every timed unit of `map` in `layout`.
    pub fn of_units(map: &UnitMap, layout: &TextLayout) -> Vec<UnitGeometry> {
        let em = if layout.em.is_finite() && layout.em > 0.0 {
            layout.em
        } else {
            // A layout without its em (built by hand): the first glyph's
            // size, as the caret measures.
            layout
                .glyphs
                .first()
                .map_or(1.0, |g| g.size)
                .max(f32::MIN_POSITIVE)
        };
        (0..map.count as usize)
            .map(|u| {
                let (Some(Some(b)), Some(Some(line))) = (map.boxes.get(u), map.lines.get(u)) else {
                    return UnitGeometry::default();
                };
                let Some(l) = layout.lines.get(*line as usize) else {
                    return UnitGeometry::default();
                };
                let unit = b.center();
                let centre = (l.left + l.width.max(0.0) / 2.0, (l.top + l.bottom) / 2.0);
                UnitGeometry {
                    to_line_center: ((centre.0 - unit.x) / em, (centre.1 - unit.y) / em),
                    line_height: ((l.bottom - l.top) / em).max(0.0),
                }
            })
            .collect()
    }
}

/// A per-glyph property a channel drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GlyphChannel {
    OffsetX,
    OffsetY,
    Scale,
    ScaleX,
    ScaleY,
    Rotation,
    TurnX,
    TurnY,
    Opacity,
    Blur,
    Reveal,
    /// [`GlyphAnim::tint`].
    Tint,
    /// [`GlyphAnim::hue`].
    Hue,
    /// [`GlyphAnim::glow`].
    Glow,
}

/// Per-unit seeded variation: the channel's value is multiplied by
/// [`unit_noise`]`(seed, unit, salt)` (Scatter, Fall away).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Noise {
    pub seed: u32,
    /// Tells channels of one preset apart (x and y of Scatter).
    pub salt: u32,
}

/// One channel as points of (progress, value): linear between points,
/// held before the first and after the last; with `hold`, each point's
/// value holds until the next point (a step).
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelCurve {
    pub channel: GlyphChannel,
    /// Ascending progress, 0..=1.
    pub points: Vec<(f32, f32)>,
    pub hold: bool,
    /// The value while the unit is idle ([`UnitSample::idle`]), when it
    /// differs from the value at progress 0: Typewriter's reveal is 0
    /// while idle and 1 from the start of the window.
    pub idle: Option<f32>,
    pub noise: Option<Noise>,
}

impl ChannelCurve {
    /// The value at `s`, `None` without points.
    pub fn at(&self, s: &UnitSample) -> Option<f32> {
        let p = s.progress;
        let first = self.points.first()?;
        let mut v = first.1;
        if s.idle
            && let Some(idle) = self.idle
        {
            v = idle;
        } else if p > first.0 {
            for pair in self.points.windows(2) {
                let ((a, va), (b, vb)) = (pair[0], pair[1]);
                if p < a {
                    break;
                }
                v = if p >= b {
                    vb
                } else if self.hold || b - a <= f32::EPSILON {
                    va
                } else {
                    va + (vb - va) * (p - a) / (b - a)
                };
            }
        }
        Some(match self.noise {
            Some(n) => v * unit_noise(n.seed, s.unit, n.salt),
            None => v,
        })
    }
}

/// A small table of channels. Several curves on one property combine as
/// the composition order does: offsets, rotations, turns, blurs, tints and
/// hue turns add; scales, opacity, reveal and glow multiply. Properties
/// without a curve stay at rest.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChannelTable(pub Vec<ChannelCurve>);

impl ChannelTable {
    pub fn eval(&self, s: &UnitSample) -> GlyphAnim {
        let mut a = GlyphAnim::REST;
        for curve in &self.0 {
            let Some(v) = curve.at(s) else {
                continue;
            };
            match curve.channel {
                GlyphChannel::OffsetX => a.offset.0 += v,
                GlyphChannel::OffsetY => a.offset.1 += v,
                GlyphChannel::Scale => a.scale *= v,
                GlyphChannel::ScaleX => a.scale_xy.0 *= v,
                GlyphChannel::ScaleY => a.scale_xy.1 *= v,
                GlyphChannel::Rotation => a.rotation += v,
                GlyphChannel::TurnX => a.turn.0 += v,
                GlyphChannel::TurnY => a.turn.1 += v,
                GlyphChannel::Opacity => a.opacity *= v,
                GlyphChannel::Blur => a.blur += v,
                GlyphChannel::Reveal => a.reveal *= v,
                GlyphChannel::Tint => a.tint += v,
                GlyphChannel::Hue => a.hue += v,
                GlyphChannel::Glow => a.glow *= v,
            }
        }
        a
    }
}

/// A unit's pose computed by the caller (the timeline's preset evaluator,
/// through an adapter). Equal only to itself (pointer identity).
#[derive(Clone)]
pub struct UnitFn(pub Arc<dyn Fn(&UnitSample) -> GlyphAnim + Send + Sync>);

impl UnitFn {
    pub fn new(f: impl Fn(&UnitSample) -> GlyphAnim + Send + Sync + 'static) -> UnitFn {
        UnitFn(Arc::new(f))
    }
}

impl fmt::Debug for UnitFn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("UnitFn(..)")
    }
}

impl PartialEq for UnitFn {
    fn eq(&self, other: &UnitFn) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// Where a unit's channel values come from.
#[derive(Debug, Clone, PartialEq)]
pub enum UnitChannels {
    Table(ChannelTable),
    Callback(UnitFn),
}

impl UnitChannels {
    fn eval(&self, s: &UnitSample) -> GlyphAnim {
        match self {
            UnitChannels::Table(t) => t.eval(s),
            UnitChannels::Callback(f) => (f.0)(s),
        }
        .sanitized()
    }
}

/// Decode (TX-056): while a letter has not landed (progress below 1) it
/// shows a seeded stand-in taken from the text's own letters, a new one
/// every `step`, drawn centred in the advance of the letter it replaces
/// (the layout never changes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Decode {
    pub seed: u32,
    /// How long each stand-in shows (TX-056: two frames).
    pub step: Tick,
}

/// Typewriter's caret (TX-056): a bar in the fill colour, 0.08 em wide and
/// one line tall, after the last revealed letter (on its left in
/// right-to-left text). Steady while typing; from the start of the last
/// unit's window it blinks, on for the first half of each `blink` period,
/// until the slot ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Caret {
    /// One on-and-off period (TX-056: 1 s).
    pub blink: Tick,
}

/// Width of the caret bar in em (TX-056).
pub const CARET_WIDTH_EM: f32 = 0.08;

/// A per-unit animation request for one text layer at one time.
#[derive(Debug, Clone, PartialEq)]
pub struct TextMotion {
    /// The unit asked for ([`UnitMap::unit`] says which one is used).
    pub unit: AnimUnit,
    pub order: UnitOrder,
    pub timing: UnitTiming,
    /// Slot-local time.
    pub at: Tick,
    pub channels: UnitChannels,
    /// The Reveal keyframe's value at `t` (TX-021): the fraction of letters
    /// (non-blank graphemes, in reading order) shown. `None` shows all.
    pub reveal: Option<PerMille>,
    pub decode: Option<Decode>,
    pub caret: Option<Caret>,
    /// The colour [`GlyphAnim::tint`] moves fills toward (straight sRGB;
    /// white by default: a band of brightness).
    pub tint_color: [u8; 3],
}

/// The default [`TextMotion::tint_color`].
pub const TINT_WHITE: [u8; 3] = [255, 255, 255];

impl TextMotion {
    /// A request with no Reveal keyframe, Decode or caret, tinting toward
    /// white.
    pub fn new(
        unit: AnimUnit,
        order: UnitOrder,
        timing: UnitTiming,
        at: Tick,
        channels: UnitChannels,
    ) -> TextMotion {
        TextMotion {
            unit,
            order,
            timing,
            at,
            channels,
            reveal: None,
            decode: None,
            caret: None,
            tint_color: TINT_WHITE,
        }
    }

    /// Every glyph's state at [`TextMotion::at`], for `layout` laid out
    /// from `text`.
    pub fn evaluate(&self, layout: &TextLayout, text: &str) -> MotionState {
        let map = UnitMap::new(layout, text, self.unit);
        self.evaluate_with(&map, layout, text)
    }

    /// As [`TextMotion::evaluate`], with the units already segmented.
    pub fn evaluate_with(&self, map: &UnitMap, layout: &TextLayout, text: &str) -> MotionState {
        let n = map.count;
        let sequence = play_order(n, self.order);
        let mut rank = vec![0u32; n as usize];
        for (r, &u) in sequence.iter().enumerate() {
            rank[u as usize] = r as u32;
        }
        let windows = self.timing.windows(n);
        let geometry = UnitGeometry::of_units(map, layout);
        // Each timed unit's sample and pose.
        let samples: Vec<UnitSample> = (0..n)
            .map(|u| {
                let r = rank[u as usize];
                let (progress, window, idle) = match (&windows, self.timing) {
                    (Some(w), timing) => {
                        let window = w[r as usize];
                        let p = progress(self.at, window);
                        if matches!(timing, UnitTiming::Out { .. }) {
                            (1.0 - p, window, self.at >= window.1)
                        } else {
                            (p, window, self.at < window.0)
                        }
                    }
                    (None, UnitTiming::Loop { cycle, overlap }) => {
                        let delay = loop_delay(cycle, n, r, overlap);
                        let end = Tick::from_raw(delay.raw().saturating_add(cycle.raw().max(0)));
                        (loop_phase(self.at, delay, cycle), (delay, end), false)
                    }
                    (None, _) => (1.0, (Tick::ZERO, Tick::ZERO), false),
                };
                UnitSample {
                    unit: u,
                    rank: r,
                    count: n,
                    progress,
                    window,
                    idle,
                    geometry: geometry.get(u as usize).copied().unwrap_or_default(),
                }
            })
            .collect();
        let poses: Vec<GlyphAnim> = samples.iter().map(|s| self.channels.eval(s)).collect();

        let reading_rtl = layout.rtl;
        let letters = self.reveal.map(|r| LetterReveal::new(layout, text, r));
        let mut glyphs: Vec<GlyphState> = layout
            .glyphs
            .iter()
            .enumerate()
            .map(|(i, g)| {
                let unit = map.glyph_unit.get(i).copied().unwrap_or(0);
                let anim = poses.get(unit as usize).copied().unwrap_or(GlyphAnim::REST);
                // The unit's reveal, clipped across its rest box.
                let mut shown = Shown::All;
                if anim.reveal <= 0.0 {
                    shown = Shown::None;
                } else if anim.reveal < 1.0
                    && let Some(Some(b)) = map.boxes.get(unit as usize)
                {
                    let w = b.width() * anim.reveal;
                    shown = if reading_rtl {
                        Shown::Range(b.x1 - w, f32::INFINITY)
                    } else {
                        Shown::Range(f32::NEG_INFINITY, b.x0 + w)
                    };
                }
                if let Some(letters) = &letters {
                    shown = shown.and(letters.shown(i, g.origin.0, g.advance, reading_rtl));
                }
                GlyphState {
                    unit,
                    anim,
                    shown,
                    stand_in: None,
                }
            })
            .collect();

        if let Some(decode) = self.decode
            && !matches!(self.timing, UnitTiming::Loop { .. })
        {
            // Where no letter can stand in for another (joined letters
            // move as words; one distinct letter), Decode plays as Letter
            // fade (TX-056): each unit fades in with its progress.
            let substituted = map.unit == AnimUnit::Grapheme
                && apply_decode(&mut glyphs, map, layout, &samples, decode, self.at);
            if !substituted {
                for g in &mut glyphs {
                    if let Some(s) = samples.get(g.unit as usize) {
                        g.anim.opacity *= s.progress.clamp(0.0, 1.0);
                    }
                }
            }
        }

        let caret = self.caret.and_then(|c| {
            let w = windows.as_ref()?;
            let span = match self.timing {
                UnitTiming::In { span, .. } | UnitTiming::Out { span, .. } => span,
                UnitTiming::Loop { .. } => return None,
            };
            caret_bar(c, w, span, self.at, layout, map, &glyphs)
        });

        MotionState {
            unit: map.unit,
            glyphs,
            caret,
            tint_color: self.tint_color,
        }
    }
}

/// Which part of a glyph is revealed, in rest layout px along x.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shown {
    All,
    /// Ink with rest x in `lo..hi` shows.
    Range(f32, f32),
    None,
}

impl Shown {
    /// Both reveals: a glyph shows only where both let it (TX-022).
    pub fn and(self, other: Shown) -> Shown {
        match (self, other) {
            (Shown::None, _) | (_, Shown::None) => Shown::None,
            (Shown::All, s) | (s, Shown::All) => s,
            (Shown::Range(a, b), Shown::Range(c, d)) => {
                let (lo, hi) = (a.max(c), b.min(d));
                if lo < hi {
                    Shown::Range(lo, hi)
                } else {
                    Shown::None
                }
            }
        }
    }

    pub fn is_none(&self) -> bool {
        *self == Shown::None
    }
}

/// One glyph at `t`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphState {
    /// Its timed unit.
    pub unit: u32,
    /// Its unit's pose.
    pub anim: GlyphAnim,
    /// The revealed part (both reveals combined).
    pub shown: Shown,
    /// Decode: the index of the layout glyph drawn in its place.
    pub stand_in: Option<u32>,
}

/// The caret's bar, in rest layout px: `x..x + width` over its line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaretBar {
    pub line: u32,
    pub x: f32,
    pub width: f32,
}

/// Every glyph's state at one time (see [`TextMotion::evaluate`]).
#[derive(Debug, Clone, PartialEq)]
pub struct MotionState {
    /// The unit used.
    pub unit: AnimUnit,
    /// One per layout glyph.
    pub glyphs: Vec<GlyphState>,
    pub caret: Option<CaretBar>,
    /// What tinted glyphs move toward ([`TextMotion::tint_color`]).
    pub tint_color: [u8; 3],
}

impl MotionState {
    /// Whether the text looks exactly as it does at rest.
    pub fn is_rest(&self) -> bool {
        self.caret.is_none()
            && self
                .glyphs
                .iter()
                .all(|g| g.anim == GlyphAnim::REST && g.shown == Shown::All && g.stand_in.is_none())
    }

    /// A canonical digest of the state: equal states give equal digests
    /// (−0 and 0 alike), so a frame whose glyphs stand as in another frame
    /// reuses its raster (the layer cache key, C46). The unit kind and each
    /// glyph's unit are part of it: units are what scales, turns and boxes
    /// pivot about.
    pub fn digest(&self) -> Vec<u32> {
        let mut out = Vec::with_capacity(self.glyphs.len() * 18 + 5);
        let f = |out: &mut Vec<u32>, v: f32| out.push(if v == 0.0 { 0 } else { v.to_bits() });
        out.push(match self.unit {
            AnimUnit::Grapheme => 0,
            AnimUnit::Word => 1,
            AnimUnit::Line => 2,
        });
        for g in &self.glyphs {
            let a = &g.anim;
            out.push(g.unit);
            for v in [
                a.offset.0,
                a.offset.1,
                a.scale,
                a.scale_xy.0,
                a.scale_xy.1,
                a.rotation,
                a.turn.0,
                a.turn.1,
                a.opacity,
                a.blur,
                a.tint,
                a.hue,
                a.glow,
            ] {
                f(&mut out, v);
            }
            match g.shown {
                Shown::All => out.push(0),
                Shown::None => out.push(1),
                Shown::Range(lo, hi) => {
                    out.push(2);
                    f(&mut out, lo);
                    f(&mut out, hi);
                }
            }
            out.push(g.stand_in.unwrap_or(u32::MAX));
            out.push(u32::from(a.line_clip));
        }
        match self.caret {
            None => out.push(0),
            Some(c) => {
                out.push(1);
                out.push(c.line);
                f(&mut out, c.x);
                f(&mut out, c.width);
            }
        }
        if self.glyphs.iter().any(|g| g.anim.tint > 0.0) {
            let [r, g, b] = self.tint_color.map(u32::from);
            out.push(1 << 24 | r << 16 | g << 8 | b);
        }
        out
    }
}

/// The Reveal keyframe (TX-021): the first ⌊n × reveal⌋ letters in reading
/// order show; a glyph holding several letters (a ligature) is clipped at
/// the proportional x position.
struct LetterReveal {
    /// Per layout glyph: (letters before it, letters in it).
    spans: Vec<(u32, u32)>,
    shown: u32,
}

impl LetterReveal {
    fn new(layout: &TextLayout, text: &str, reveal: PerMille) -> LetterReveal {
        use unicode_segmentation::UnicodeSegmentation;
        // Non-blank grapheme starts, ascending.
        let letters: Vec<usize> = text
            .grapheme_indices(true)
            .filter(|(_, g)| !units::is_blank(g))
            .map(|(i, _)| i)
            .collect();
        let total = letters.len() as u64;
        let shown = (total * u64::from(reveal.get().min(1000)) / 1000) as u32;
        let mut starts: Vec<usize> = layout.glyphs.iter().map(|g| g.cluster as usize).collect();
        starts.sort_unstable();
        starts.dedup();
        let spans = layout
            .glyphs
            .iter()
            .map(|g| {
                let start = g.cluster as usize;
                let next = starts.partition_point(|&s| s <= start);
                let end = starts.get(next).copied().unwrap_or(text.len());
                let before = letters.partition_point(|&l| l < start) as u32;
                let upto = letters.partition_point(|&l| l < end) as u32;
                (before, upto - before)
            })
            .collect();
        LetterReveal { spans, shown }
    }

    fn shown(&self, glyph: usize, x: f32, advance: f32, rtl: bool) -> Shown {
        let Some(&(before, count)) = self.spans.get(glyph) else {
            return Shown::All;
        };
        if count == 0 {
            // Whitespace shows with the letter before it (leading
            // whitespace with the first letter).
            let with = if before == 0 {
                self.shown > 0
            } else {
                self.shown >= before
            };
            return if with { Shown::All } else { Shown::None };
        }
        let visible = self.shown.saturating_sub(before).min(count);
        if visible == count {
            Shown::All
        } else if visible == 0 {
            Shown::None
        } else {
            let w = advance.max(0.0) * visible as f32 / count as f32;
            if rtl {
                Shown::Range(x + advance - w, f32::INFINITY)
            } else {
                Shown::Range(f32::NEG_INFINITY, x + w)
            }
        }
    }
}

/// Unit progress at `t` in `window`: 0 before it, 1 from its end, the
/// exact tick ratio in between (turned into `f32` only here).
pub fn progress(t: Tick, window: (Tick, Tick)) -> f32 {
    let (s, e) = (window.0.raw(), window.1.raw());
    let t = t.raw();
    if t >= e {
        1.0
    } else if t <= s {
        0.0
    } else {
        let num = i128::from(t) - i128::from(s);
        let den = i128::from(e) - i128::from(s);
        (num as f64 / den as f64) as f32
    }
}

/// The loop phase (0..1) at `t` of a unit delayed by `delay`.
pub fn loop_phase(t: Tick, delay: Tick, cycle: Tick) -> f32 {
    let c = i128::from(cycle.raw());
    if c <= 0 {
        return 0.0;
    }
    let local = (i128::from(t.raw()) - i128::from(delay.raw())).rem_euclid(c);
    (local as f64 / c as f64) as f32
}

/// Splits an In or Out span among `n` timed units (TX-023). Overlap 0 plays
/// them one after another, 1000 all together. Window `i` is
/// `[s·i·k / d, s·(i·k + 1000) / d)` with `k = 1000 − overlap` and
/// `d = 1000 + (n − 1)·k`, in exact integer arithmetic, so the last window
/// ends exactly at `span`. A negative span counts as 0.
pub fn unit_windows(span: Tick, n: u32, overlap: PerMille) -> Vec<(Tick, Tick)> {
    if n == 0 {
        return Vec::new();
    }
    let k = 1000 - i128::from(overlap.get().min(1000));
    let d = 1000 + i128::from(n - 1) * k;
    let s = i128::from(span.raw().max(0));
    (0..i128::from(n))
        // Both bounds lie in [0, span] (i·k ≤ (n−1)·k and i·k + 1000 ≤ d),
        // so the narrowing cannot overflow.
        .map(|i| {
            (
                Tick::from_raw((s * i * k / d) as i64),
                Tick::from_raw((s * (i * k + 1000) / d) as i64),
            )
        })
        .collect()
}

/// The delay of the unit at play position `rank` in a Loop (TX-023):
/// `rank × cycle × (1 − overlap) / n`, rounded down to a tick, so a wave
/// travels across the text and wraps seamlessly.
pub fn loop_delay(cycle: Tick, n: u32, rank: u32, overlap: PerMille) -> Tick {
    if n == 0 {
        return Tick::ZERO;
    }
    let k = 1000 - i128::from(overlap.get().min(1000));
    let v = i128::from(rank) * i128::from(cycle.raw().max(0)) * k / (1000 * i128::from(n));
    // rank < n, so v < cycle.
    Tick::from_raw(v as i64)
}

/// The units `0..n` in play order.
pub fn play_order(n: u32, order: UnitOrder) -> Vec<u32> {
    let mut seq: Vec<u32> = (0..n).collect();
    match order {
        UnitOrder::Forward => {}
        UnitOrder::Backward => seq.reverse(),
        UnitOrder::CenterOut => {
            // Distance from the middle, doubled to stay integer; ties take
            // the left unit first (the stable sort keeps index order).
            let mid2 = i64::from(n) - 1;
            seq.sort_by_key(|&i| (2 * i64::from(i) - mid2).abs());
        }
        UnitOrder::EdgesIn => {
            let (mut lo, mut hi) = (0u32, n);
            seq.clear();
            while lo < hi {
                seq.push(lo);
                lo += 1;
                if lo < hi {
                    hi -= 1;
                    seq.push(hi);
                }
            }
        }
        UnitOrder::Random { seed } => {
            let mut rng = SplitMix64::new(u64::from(seed));
            shuffle(&mut seq, &mut rng);
        }
    }
    seq
}

/// SplitMix64 (Steele, Lea and Flood), FreeCut's frozen generator for
/// seeded animation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    pub fn new(seed: u64) -> SplitMix64 {
        SplitMix64 { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A uniform integer in `0..bound` (Lemire's multiply-and-reject);
    /// 0 when `bound` is 0.
    pub fn below(&mut self, bound: u64) -> u64 {
        if bound == 0 {
            return 0;
        }
        let mut m = u128::from(self.next_u64()) * u128::from(bound);
        if (m as u64) < bound {
            let threshold = bound.wrapping_neg() % bound;
            while (m as u64) < threshold {
                m = u128::from(self.next_u64()) * u128::from(bound);
            }
        }
        (m >> 64) as u64
    }
}

/// Fisher–Yates shuffle, from the last element down.
pub fn shuffle<T>(items: &mut [T], rng: &mut SplitMix64) {
    for i in (1..items.len()).rev() {
        let j = rng.below(i as u64 + 1) as usize;
        items.swap(i, j);
    }
}

/// The SplitMix64 seed for `(seed, unit, salt)`.
fn mix(seed: u32, unit: u32, salt: u64) -> u64 {
    (u64::from(seed) | u64::from(unit) << 32) ^ salt.wrapping_mul(0xD1B5_4A32_D192_ED03)
}

/// A seeded value in `[-1, 1)` for one unit and one salt: the per-unit
/// variation of Scatter, Fall away and similar presets.
pub fn unit_noise(seed: u32, unit: u32, salt: u32) -> f32 {
    let mut rng = SplitMix64::new(mix(seed, unit, u64::from(salt) + 1));
    // 24 random bits: exact in f32.
    let v = (rng.next_u64() >> 40) as f32 / (1u64 << 24) as f32;
    2.0 * v - 1.0
}

/// Decode's stand-ins: every letter whose unit is still moving shows
/// another single-glyph letter of the text, chosen per unit and per step.
/// `false` when the text has fewer than two distinct such letters, so
/// nothing can stand in.
fn apply_decode(
    glyphs: &mut [GlyphState],
    map: &UnitMap,
    layout: &TextLayout,
    samples: &[UnitSample],
    decode: Decode,
    at: Tick,
) -> bool {
    // Letters drawn by one glyph each, distinct by face and glyph, in text
    // order.
    let mut per_unit = vec![0u32; map.count as usize];
    for (i, &u) in map.glyph_unit.iter().enumerate() {
        if !map.blank[i]
            && let Some(c) = per_unit.get_mut(u as usize)
        {
            *c += 1;
        }
    }
    let single =
        |i: usize| !map.blank[i] && per_unit.get(map.glyph_unit[i] as usize).copied() == Some(1);
    let mut candidates: Vec<u32> = Vec::new();
    for (i, g) in layout.glyphs.iter().enumerate() {
        if single(i)
            && !candidates.iter().any(|&c| {
                let o = &layout.glyphs[c as usize];
                o.font == g.font && o.glyph_id == g.glyph_id
            })
        {
            candidates.push(i as u32);
        }
    }
    if candidates.len() < 2 {
        return false;
    }
    let step = at.raw().max(0) / decode.step.raw().max(1);
    for (i, state) in glyphs.iter_mut().enumerate() {
        if !single(i) {
            continue;
        }
        let Some(s) = samples.get(state.unit as usize) else {
            continue;
        };
        if s.progress >= 1.0 {
            continue;
        }
        let own = &layout.glyphs[i];
        let others: Vec<u32> = candidates
            .iter()
            .copied()
            .filter(|&c| {
                let o = &layout.glyphs[c as usize];
                !(o.font == own.font && o.glyph_id == own.glyph_id)
            })
            .collect();
        if others.is_empty() {
            continue;
        }
        let mut rng = SplitMix64::new(mix(decode.seed, state.unit, step as u64));
        state.stand_in = Some(others[rng.below(others.len() as u64) as usize]);
    }
    true
}

/// The caret at `at`, `None` while it blinks off or after the slot.
fn caret_bar(
    caret: Caret,
    windows: &[(Tick, Tick)],
    span: Tick,
    at: Tick,
    layout: &TextLayout,
    map: &UnitMap,
    glyphs: &[GlyphState],
) -> Option<CaretBar> {
    if at >= span || at < Tick::ZERO {
        return None;
    }
    let typed = windows.last().map_or(Tick::ZERO, |w| w.0);
    if at >= typed && caret.blink.raw() > 0 {
        let half = (caret.blink.raw() / 2).max(1);
        let k = (i128::from(at.raw()) - i128::from(typed.raw())) / i128::from(half);
        if k % 2 == 1 {
            return None;
        }
    }
    let em = layout.glyphs.first()?.size;
    let width = CARET_WIDTH_EM * em;
    // The last typed letter in reading order: revealed and visible (the
    // bundled Typewriter steps opacity, not the reveal).
    let last = layout
        .glyphs
        .iter()
        .enumerate()
        .filter(|(i, _)| {
            let g = &glyphs[*i];
            !map.blank[*i] && !g.shown.is_none() && g.anim.opacity > 0.0
        })
        .max_by_key(|(_, g)| g.cluster);
    Some(match last {
        Some((_, g)) => CaretBar {
            line: g.unit.line,
            x: if layout.rtl {
                g.origin.0 - width
            } else {
                g.origin.0 + g.advance
            },
            width,
        },
        None => {
            let line = layout.lines.first()?;
            CaretBar {
                line: 0,
                x: if layout.rtl {
                    line.left + line.width - width
                } else {
                    line.left
                },
                width,
            }
        }
    })
}

#[cfg(test)]
mod tests;
