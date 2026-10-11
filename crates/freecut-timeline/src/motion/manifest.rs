//! Preset manifests (FX-030, TX-056): what a preset moves, as plain data.
//! Parsing the JSON files is `freecut-project`'s job; this crate holds the
//! typed form, a lookup trait, and a few built-in samples for tests (the
//! full catalogue lives in `resources/`).
//!
//! Every preset is FreeCut's own motion, defined in TX-056 and FX-030.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use super::flash::{Photosensitivity, min_safe_cycle, photosensitivity};

use crate::model::{
    AnimOrder, AnimParams, AnimUnit, AnimationSpec, Direction, EaseCurve, EaseDir, Interp,
    PerMille, PresetId, Slot,
};
use crate::time::{TICKS_PER_SECOND, Tick};

/// The style values a preset modulates: which per-glyph style work the
/// renderer has to do for a per-unit preset (TX-022, TX-056).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct StyleUse {
    /// Hue turn ([`StyleChannel::Hue`]).
    pub hue: bool,
    /// Glow intensity ([`StyleChannel::GlowIntensity`]).
    pub glow: bool,
    /// Fill lightened toward white ([`StyleChannel::Lightness`]).
    pub lightness: bool,
    /// White tint ([`ChannelProp::Tint`]).
    pub tint: bool,
}

impl StyleUse {
    /// Whether any style value moves.
    pub const fn any(self) -> bool {
        self.hue || self.glow || self.lightness || self.tint
    }
}

/// When and how a preset plays (FX-030).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PresetKind {
    /// In: from the start pose to rest.
    Entrance,
    /// Out with a motion of its own, from rest to its end pose. Exits
    /// without a manifest play their entrance backwards.
    Exit,
    /// Repeats every cycle (the spec's duration).
    Loop,
    /// One pass across the Continuous window.
    Drift,
    /// An entrance, a through-motion and an exit across the window (FX-019;
    /// evaluated like a drift until its portions are specified).
    Combined,
}

/// The point rotations and scales turn about (FX-030).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pivot {
    /// The transform anchor (the layer's centre).
    Anchor,
    /// The top centre of the layer's bounds (a pendulum).
    TopCenter,
    /// The bottom centre (text: about the baseline).
    BottomCenter,
    /// The middle of the edge on the effective direction's side (a swing).
    Edge,
}

/// A style value a Loop may modulate (TX-022 step 1, TX-056).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StyleChannel {
    /// Turns fill and outline hues (degrees), keeping lightness.
    Hue,
    /// Multiplies the glow intensity (`1 + v`).
    GlowIntensity,
    /// Lightens the fill (`v` of the way to white).
    Lightness,
}

/// What a channel moves (FX-030, TX-056).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChannelProp {
    TranslateX,
    TranslateY,
    /// Uniform scale factor: multiplied by `1 + v`.
    Scale,
    ScaleX,
    ScaleY,
    /// Degrees, clockwise.
    Rotate,
    /// A 3D turn about the horizontal axis, degrees.
    TurnX,
    /// A 3D turn about the vertical axis, degrees.
    TurnY,
    /// Multiplied by `1 + v`.
    Opacity,
    Blur,
    /// The shown share, multiplied by `1 + v`: a wipe from the effective
    /// direction's side on a block, a per-unit reveal on text.
    Reveal,
    /// White tint, added.
    Tint,
    /// Decode's glyph substitution (TX-056), added; drawn by the renderer.
    Decode,
    Style(StyleChannel),
    /// A seeded per-unit offset (TX-056's Scatter): each unit moves a
    /// seeded distance of 0.5–1 × the amplitude in a seeded direction,
    /// times the curve.
    RandomOffset,
    /// A seeded per-unit turn (Scatter, Fall away): within ±amplitude,
    /// times the curve, in degrees.
    RandomRotate,
    /// A pull toward the unit's line centre (Spread): the unit moves this
    /// share of the way from its own centre to its line's centre, times the
    /// curve. Needs [`super::UnitContext::to_line_center`].
    TowardLineCenter,
    /// Down by this many of the unit's line heights, times the curve,
    /// clipped to the line's box (Word rise). Uses
    /// [`super::UnitContext::line_height`] (1.2 em without it).
    LineRise,
    /// Smooth seeded jitter of ±amplitude (FX-030's Jitter shake, TX-056's
    /// Shake and Wiggle), changing direction 12 times a second; the curve
    /// is its envelope. Horizontal, in the channel's length unit.
    JitterX,
    /// As [`ChannelProp::JitterX`], vertical.
    JitterY,
    /// As [`ChannelProp::JitterX`], a turn in degrees.
    JitterRotate,
    /// Opacity dips to `1 − amplitude` (times the curve) for two frames at
    /// seeded moments, at least a second apart, also across a loop's seam
    /// (TX-056's Flicker): a cycle of `c` seconds holds `⌊c⌋` dips.
    FlickerOpacity,
    /// The unit swells to `1 + amplitude` (times the curve) for 0.2 s once
    /// per cycle, at a seeded moment (TX-056's Sparkle).
    SparkleScale,
    /// A 3D turn whose axis follows the direction (FX-030's 3D flip in):
    /// about the vertical axis from the left (positive) or right
    /// (negative), about the horizontal axis from the top (positive) or
    /// bottom (negative). Degrees.
    TurnFromSide,
}

impl ChannelProp {
    /// Every channel, for tables and tests.
    pub const ALL: [ChannelProp; 26] = [
        ChannelProp::TranslateX,
        ChannelProp::TranslateY,
        ChannelProp::Scale,
        ChannelProp::ScaleX,
        ChannelProp::ScaleY,
        ChannelProp::Rotate,
        ChannelProp::TurnX,
        ChannelProp::TurnY,
        ChannelProp::Opacity,
        ChannelProp::Blur,
        ChannelProp::Reveal,
        ChannelProp::Tint,
        ChannelProp::Decode,
        ChannelProp::Style(StyleChannel::Hue),
        ChannelProp::Style(StyleChannel::GlowIntensity),
        ChannelProp::Style(StyleChannel::Lightness),
        ChannelProp::RandomOffset,
        ChannelProp::RandomRotate,
        ChannelProp::TowardLineCenter,
        ChannelProp::LineRise,
        ChannelProp::JitterX,
        ChannelProp::JitterY,
        ChannelProp::JitterRotate,
        ChannelProp::FlickerOpacity,
        ChannelProp::SparkleScale,
        ChannelProp::TurnFromSide,
    ];

    /// The stable name in manifest files.
    pub const fn as_str(self) -> &'static str {
        match self {
            ChannelProp::TranslateX => "translate_x",
            ChannelProp::TranslateY => "translate_y",
            ChannelProp::Scale => "scale",
            ChannelProp::ScaleX => "scale_x",
            ChannelProp::ScaleY => "scale_y",
            ChannelProp::Rotate => "rotate",
            ChannelProp::TurnX => "turn_x",
            ChannelProp::TurnY => "turn_y",
            ChannelProp::Opacity => "opacity",
            ChannelProp::Blur => "blur",
            ChannelProp::Reveal => "reveal",
            ChannelProp::Tint => "tint",
            ChannelProp::Decode => "decode",
            ChannelProp::Style(StyleChannel::Hue) => "style.hue",
            ChannelProp::Style(StyleChannel::GlowIntensity) => "style.glow_intensity",
            ChannelProp::Style(StyleChannel::Lightness) => "style.lightness",
            ChannelProp::RandomOffset => "random.offset",
            ChannelProp::RandomRotate => "random.rotate",
            ChannelProp::TowardLineCenter => "toward_line_center",
            ChannelProp::LineRise => "line_rise",
            ChannelProp::JitterX => "jitter.translate_x",
            ChannelProp::JitterY => "jitter.translate_y",
            ChannelProp::JitterRotate => "jitter.rotate",
            ChannelProp::FlickerOpacity => "flicker.opacity",
            ChannelProp::SparkleScale => "sparkle.scale",
            ChannelProp::TurnFromSide => "turn_from_side",
        }
    }

    /// Whether the channel draws seeded numbers (TX-056: from
    /// [`AnimParams::seed`] and the unit's index), so a new seed changes
    /// the motion.
    pub const fn is_seeded(self) -> bool {
        matches!(
            self,
            ChannelProp::RandomOffset
                | ChannelProp::RandomRotate
                | ChannelProp::JitterX
                | ChannelProp::JitterY
                | ChannelProp::JitterRotate
                | ChannelProp::FlickerOpacity
                | ChannelProp::SparkleScale
        )
    }

    /// Whether the channel needs the unit's place in its line
    /// ([`super::UnitContext`]'s line geometry), which only the layout
    /// knows.
    pub const fn needs_line_geometry(self) -> bool {
        matches!(self, ChannelProp::TowardLineCenter | ChannelProp::LineRise)
    }

    /// The channel named `name`; `None` for a channel this build doesn't
    /// know, which makes the whole manifest unavailable (FX-030: shown as
    /// **Unavailable animation**, FX-018).
    pub fn parse(name: &str) -> Option<ChannelProp> {
        Self::ALL.iter().copied().find(|c| c.as_str() == name)
    }
}

/// The unit of a channel's amplitude (FX-030).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChannelUnit {
    /// Fractions of the canvas short edge.
    Se,
    /// The clip's font size (text; other layers use 1/16 of the short
    /// edge).
    Em,
    Deg,
    /// A plain factor or share.
    Ratio,
    /// The slide distance set by Travel (FX-030): just past the canvas
    /// edge on the effective side (**From edge**), or 0.15·k short edges
    /// (3·k em on text, **Short**). Amplitudes are written for a slide
    /// from the left (−1 = one distance to the left).
    Travel,
}

/// One point of a sampled curve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurvePoint {
    pub t: f64,
    pub v: f64,
}

/// A channel's shape over slot progress `p` in `[0, 1]` (FX-030). For
/// entrances (and loops and drifts) the weight is written so that rest is
/// 0 at `p = 1`; for exit-kind manifests rest is 0 at `p = 0`.
#[derive(Debug, Clone, PartialEq)]
pub enum Curve {
    /// Follows the slot's easing `E` (the preset's, or the spec's
    /// override): `1 − E(p)` on entrances, `E(p)` on exit-kind manifests.
    Eased,
    /// As [`Curve::Eased`] with a fixed TR-031 easing.
    Ease(Interp),
    /// A monotone cubic through up to 16 points (`t` strictly increasing
    /// in `[0, 1]`), held flat outside them; evaluated at `E(p)` when
    /// `eased`, else at `p`.
    Sampled {
        points: Arc<[CurvePoint]>,
        eased: bool,
    },
    /// `sin(2π·p)`: one full cycle, for loops.
    Sine,
    /// A jump: 1 until the slot starts and 0 from then on (entrances); 0
    /// until it starts and 1 from then on (exit-kind manifests).
    Step,
}

impl Curve {
    /// A sampled curve; `None` for 0 or more than 16 points, or times that
    /// don't strictly increase inside `[0, 1]`.
    pub fn sampled(points: Vec<CurvePoint>, eased: bool) -> Option<Curve> {
        let ok = (1..=16).contains(&points.len())
            && points
                .iter()
                .all(|p| (0.0..=1.0).contains(&p.t) && p.v.is_finite())
            && points.windows(2).all(|w| w[0].t < w[1].t);
        ok.then(|| Curve::Sampled {
            points: Arc::from(points),
            eased,
        })
    }
}

/// One moving part of a preset (FX-030).
#[derive(Debug, Clone, PartialEq)]
pub struct Channel {
    pub property: ChannelProp,
    pub curve: Curve,
    /// In `unit`s: the channel contributes `amplitude × curve(p)` (times
    /// `k` when it scales with intensity).
    pub amplitude: f64,
    pub unit: ChannelUnit,
    pub scales_with_intensity: bool,
}

/// The settings a preset starts with when chosen (TX-056's defaults).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ParamDefaults {
    pub overlap: PerMille,
    pub order: AnimOrder,
    /// Pace for typewriter-like presets.
    pub pace: Option<Tick>,
}

impl Default for ParamDefaults {
    fn default() -> ParamDefaults {
        ParamDefaults {
            overlap: PerMille::new(500),
            order: AnimOrder::Forward,
            pace: None,
        }
    }
}

/// A preset (FX-030's manifest format).
#[derive(Debug, Clone, PartialEq)]
pub struct PresetManifest {
    pub id: PresetId,
    pub kind: PresetKind,
    /// The units a per-unit preset moves, the first being its default;
    /// `None` for whole-block presets (TX-023).
    pub units: Option<Vec<AnimUnit>>,
    /// The directions offered, the first being the default; `None` when
    /// the preset has none.
    pub directions: Option<Vec<Direction>>,
    /// Slot length, or cycle length for loops.
    pub default_duration: Tick,
    /// The built-in time curve (FX-018's easing override replaces it).
    pub easing: Interp,
    pub pivot: Pivot,
    pub channels: Vec<Channel>,
    pub defaults: ParamDefaults,
}

/// Why a manifest is unusable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ManifestError {
    #[error("unknown channel {0:?}")]
    UnknownChannel(String),
    #[error("the default duration must be positive")]
    Duration,
    #[error("a channel amplitude is not a finite number")]
    Amplitude,
    #[error("a preset needs at least one channel")]
    NoChannels,
    #[error("an empty unit or direction list")]
    EmptyList,
}

impl PresetManifest {
    /// Checks what the types can't: a positive default duration, finite
    /// amplitudes, at least one channel, and non-empty unit and direction
    /// lists.
    pub fn check(&self) -> Result<(), ManifestError> {
        if !self.default_duration.is_positive() {
            return Err(ManifestError::Duration);
        }
        if self.channels.is_empty() {
            return Err(ManifestError::NoChannels);
        }
        if self.channels.iter().any(|c| !c.amplitude.is_finite()) {
            return Err(ManifestError::Amplitude);
        }
        if self.units.as_ref().is_some_and(Vec::is_empty)
            || self.directions.as_ref().is_some_and(Vec::is_empty)
        {
            return Err(ManifestError::EmptyList);
        }
        Ok(())
    }

    /// Whether the preset moves units of text rather than the whole block.
    pub fn is_per_unit(&self) -> bool {
        self.units
            .as_ref()
            .is_some_and(|u| u.iter().any(|u| *u != AnimUnit::Block))
    }

    /// Whether the motion depends on [`AnimParams::seed`]: a random order
    /// by default, a seeded channel, or Decode's seeded stand-ins (TX-023's
    /// dice button, TX-056).
    pub fn uses_seed(&self) -> bool {
        self.defaults.order == AnimOrder::Random
            || self
                .channels
                .iter()
                .any(|c| c.property.is_seeded() || c.property == ChannelProp::Decode)
    }

    /// Whether posing a unit needs its line's geometry
    /// ([`super::UnitContext::to_line_center`] and `line_height`), which
    /// the renderer reads from its layout.
    pub fn needs_line_geometry(&self) -> bool {
        self.channels
            .iter()
            .any(|c| c.property.needs_line_geometry())
    }

    /// The style values the preset modulates (TX-022 step 1 for whole
    /// text; per glyph for per-unit presets such as Shimmer's band).
    pub fn style_channels(&self) -> StyleUse {
        let mut out = StyleUse::default();
        for c in &self.channels {
            match c.property {
                ChannelProp::Style(StyleChannel::Hue) => out.hue = true,
                ChannelProp::Style(StyleChannel::GlowIntensity) => out.glow = true,
                ChannelProp::Style(StyleChannel::Lightness) => out.lightness = true,
                ChannelProp::Tint => out.tint = true,
                _ => {}
            }
        }
        out
    }

    /// The manifest with every amplitude times `amount` (TX-022's loop
    /// seam envelope): every channel is linear in its amplitude, so this
    /// plays the same motion at `amount` of its strength.
    pub fn scaled(&self, amount: f64) -> PresetManifest {
        let mut out = self.clone();
        for c in &mut out.channels {
            c.amplitude *= amount;
        }
        out
    }

    /// A spec choosing this preset with its defaults (TX-056): its length
    /// or cycle, its default unit, overlap, order and Pace.
    pub fn default_spec(&self) -> AnimationSpec {
        let mut spec = AnimationSpec::new(self.id.clone(), self.default_duration);
        spec.params = AnimParams {
            unit: self.units.as_ref().and_then(|u| u.first().copied()),
            overlap: self.defaults.overlap,
            order: self.defaults.order,
            pace: self.defaults.pace,
            ..AnimParams::default()
        };
        spec
    }
}

/// Where manifests come from (the app's loaded catalogue).
pub trait PresetCatalog {
    /// The manifest with this exact id.
    fn get(&self, id: &PresetId) -> Option<Arc<PresetManifest>>;

    /// The manifest a spec in `slot` plays: its own, or for an exit
    /// without a manifest of its own, its entrance twin played backwards
    /// (FX-030). `None` is FX-018's **Unavailable animation**: rendered as
    /// no animation.
    fn resolve(&self, slot: Slot, id: &PresetId) -> Option<Arc<PresetManifest>> {
        if let Some(m) = self.get(id) {
            return Some(m);
        }
        if slot != Slot::Exit {
            return None;
        }
        self.get(&id.entrance_twin()?)
            .filter(|m| m.kind == PresetKind::Entrance)
    }

    /// The photosensitivity of the manifest with this id (FX-030). Computed
    /// on every call here; [`PresetTable`] caches it.
    fn photosensitivity(&self, id: &PresetId) -> Option<Photosensitivity> {
        self.get(id).map(|m| photosensitivity(&m))
    }

    /// Whether the manifest with this id exists and passes FX-030's
    /// photosensitivity rule, so a grid may offer it.
    fn is_safe(&self, id: &PresetId) -> bool {
        self.photosensitivity(id).is_some_and(|p| p.is_safe())
    }

    /// The shortest cycle the loop with this id plays at
    /// ([`min_safe_cycle`], FX-019, FX-030): the Cycle control's lower
    /// bound, and the cycle a shorter stored one plays at. FX-019's 0.1 s
    /// for any other id. Computed on every call here; [`PresetTable`]
    /// caches it.
    fn min_cycle(&self, id: &PresetId) -> Tick {
        self.get(id)
            .map_or(AnimationSpec::MIN_DURATION, |m| min_safe_cycle(&m))
    }
}

/// No presets: every animation is unavailable; keyframes still play.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoPresets;

impl PresetCatalog for NoPresets {
    fn get(&self, _: &PresetId) -> Option<Arc<PresetManifest>> {
        None
    }
}

/// Whether a grid can offer a preset (FX-018, FX-030), as
/// [`PresetTable::availability`] tells it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    /// Loaded and within the photosensitivity limits: offered.
    Ready,
    /// Loaded, but over FX-030's limits: never offered.
    Unsafe(Photosensitivity),
    /// Its file needs something this build lacks (a channel, a newer
    /// schema): FX-018's **Unavailable animation**, with the loader's
    /// reason.
    Unavailable(Arc<str>),
    /// No preset with this id.
    Unknown,
}

/// A loaded manifest, its photosensitivity and (loops) its shortest safe
/// cycle, each computed once on first use and shared by every clone of
/// the table.
#[derive(Debug, Clone)]
struct Entry {
    manifest: Arc<PresetManifest>,
    safety: Arc<OnceLock<Photosensitivity>>,
    min_cycle: Arc<OnceLock<Tick>>,
}

impl Entry {
    fn safety(&self) -> Photosensitivity {
        *self.safety.get_or_init(|| photosensitivity(&self.manifest))
    }

    fn min_cycle(&self) -> Tick {
        *self
            .min_cycle
            .get_or_init(|| min_safe_cycle(&self.manifest))
    }
}

/// Manifests by id, with each one's photosensitivity cached (computed on
/// first use, or all at once by [`PresetTable::precompute`] on a worker),
/// and the ids the loader had to leave out, so grids can show them as
/// **Unavailable animation** instead of hiding them.
#[derive(Debug, Clone, Default)]
pub struct PresetTable {
    entries: BTreeMap<PresetId, Entry>,
    unavailable: BTreeMap<PresetId, Arc<str>>,
}

impl PresetTable {
    pub fn new(manifests: impl IntoIterator<Item = PresetManifest>) -> PresetTable {
        PresetTable {
            entries: manifests
                .into_iter()
                .map(|m| {
                    let entry = Entry {
                        manifest: Arc::new(m),
                        safety: Arc::default(),
                        min_cycle: Arc::default(),
                    };
                    (entry.manifest.id.clone(), entry)
                })
                .collect(),
            unavailable: BTreeMap::new(),
        }
    }

    /// The built-in samples ([`samples`]).
    pub fn samples() -> PresetTable {
        PresetTable::new(samples())
    }

    /// The table that also knows the presets the loader left out, by id,
    /// with why (FX-018). An id that also loaded stays loaded.
    pub fn with_unavailable(
        mut self,
        ids: impl IntoIterator<Item = (PresetId, Arc<str>)>,
    ) -> PresetTable {
        for (id, why) in ids {
            if !self.entries.contains_key(&id) {
                self.unavailable.insert(id, why);
            }
        }
        self
    }

    /// Every loaded manifest, by id.
    pub fn manifests(&self) -> impl Iterator<Item = &Arc<PresetManifest>> {
        self.entries.values().map(|e| &e.manifest)
    }

    /// The loaded manifests a grid may offer: within FX-030's
    /// photosensitivity limits (cached).
    pub fn safe_manifests(&self) -> impl Iterator<Item = &Arc<PresetManifest>> {
        self.entries
            .values()
            .filter(|e| e.safety().is_safe())
            .map(|e| &e.manifest)
    }

    /// The presets the loader left out, with why.
    pub fn unavailable(&self) -> impl Iterator<Item = (&PresetId, &str)> {
        self.unavailable.iter().map(|(id, why)| (id, &**why))
    }

    /// Whether a grid can offer `id`: cheap after the first call for that
    /// id (or after [`PresetTable::precompute`]).
    pub fn availability(&self, id: &PresetId) -> Availability {
        if let Some(e) = self.entries.get(id) {
            let safety = e.safety();
            return if safety.is_safe() {
                Availability::Ready
            } else {
                Availability::Unsafe(safety)
            };
        }
        match self.unavailable.get(id) {
            Some(why) => Availability::Unavailable(Arc::clone(why)),
            None => Availability::Unknown,
        }
    }

    /// Computes every manifest's photosensitivity and every loop's
    /// shortest safe cycle now (milliseconds in release builds), so later
    /// questions never wait. Meant for a worker thread after loading;
    /// clones made before or after share the result.
    pub fn precompute(&self) {
        for e in self.entries.values() {
            e.safety();
            e.min_cycle();
        }
    }
}

impl PresetCatalog for PresetTable {
    fn get(&self, id: &PresetId) -> Option<Arc<PresetManifest>> {
        self.entries.get(id).map(|e| Arc::clone(&e.manifest))
    }

    fn photosensitivity(&self, id: &PresetId) -> Option<Photosensitivity> {
        self.entries.get(id).map(Entry::safety)
    }

    fn min_cycle(&self, id: &PresetId) -> Tick {
        self.entries
            .get(id)
            .map_or(AnimationSpec::MIN_DURATION, Entry::min_cycle)
    }
}

const fn secs_milli(ms: i64) -> Tick {
    Tick::from_raw(TICKS_PER_SECOND / 1000 * ms)
}

fn ease(curve: EaseCurve, dir: EaseDir) -> Interp {
    Interp::Ease { curve, dir }
}

fn points(p: &[(f64, f64)], eased: bool) -> Curve {
    Curve::Sampled {
        points: p.iter().map(|&(t, v)| CurvePoint { t, v }).collect(),
        eased,
    }
}

fn channel(
    property: ChannelProp,
    curve: Curve,
    amplitude: f64,
    unit: ChannelUnit,
    k: bool,
) -> Channel {
    Channel {
        property,
        curve,
        amplitude,
        unit,
        scales_with_intensity: k,
    }
}

/// Six FreeCut presets from TX-056, as built-in data for tests and as the
/// reference for the manifest files: Fade, Rise and Slide (whole text),
/// Pop (whole text), Typewriter (letters) and Wave (a per-letter loop).
pub fn samples() -> Vec<PresetManifest> {
    use ChannelProp as P;
    use ChannelUnit as U;
    let manifest = |id: &str, kind, duration, easing, channels| PresetManifest {
        id: PresetId::new(id),
        kind,
        units: None,
        directions: None,
        default_duration: secs_milli(duration),
        easing,
        pivot: Pivot::Anchor,
        channels,
        defaults: ParamDefaults::default(),
    };
    vec![
        // Opacity 0 → 1; 0.5 s, ease-out.
        manifest(
            "fc.in.fade",
            PresetKind::Entrance,
            500,
            ease(EaseCurve::Quad, EaseDir::Out),
            vec![channel(P::Opacity, Curve::Eased, -1.0, U::Ratio, false)],
        ),
        // 1·k em below and transparent, up to rest; opaque at 40 %.
        manifest(
            "fc.in.rise",
            PresetKind::Entrance,
            500,
            ease(EaseCurve::Cubic, EaseDir::Out),
            vec![
                channel(P::TranslateY, Curve::Eased, 1.0, U::Em, true),
                channel(
                    P::Opacity,
                    points(&[(0.0, 1.0), (0.4, 0.0), (1.0, 0.0)], false),
                    -1.0,
                    U::Ratio,
                    false,
                ),
            ],
        ),
        // In from the chosen side, from just past the edge (Travel).
        PresetManifest {
            directions: Some(vec![
                Direction::Left,
                Direction::Right,
                Direction::Up,
                Direction::Down,
            ]),
            ..manifest(
                "fc.in.slide",
                PresetKind::Entrance,
                600,
                ease(EaseCurve::Cubic, EaseDir::Out),
                vec![channel(P::TranslateX, Curve::Eased, -1.0, U::Travel, false)],
            )
        },
        // Scale 0 → 1 + 0.15·k at 65 % → 1.
        manifest(
            "fc.in.pop",
            PresetKind::Entrance,
            400,
            Interp::Linear,
            vec![
                channel(
                    P::Scale,
                    points(&[(0.0, -1.0), (0.65, 0.0), (1.0, 0.0)], true),
                    1.0,
                    U::Ratio,
                    false,
                ),
                channel(
                    P::Scale,
                    points(&[(0.0, 0.0), (0.65, 0.15), (1.0, 0.0)], true),
                    1.0,
                    U::Ratio,
                    true,
                ),
            ],
        ),
        // Each letter appears at once at the start of its window.
        PresetManifest {
            units: Some(vec![AnimUnit::Letter]),
            defaults: ParamDefaults {
                overlap: PerMille::ZERO,
                order: AnimOrder::Forward,
                pace: Some(secs_milli(60)),
            },
            ..manifest(
                "fc.in.typewriter",
                PresetKind::Entrance,
                500,
                Interp::Linear,
                vec![channel(P::Opacity, Curve::Step, -1.0, U::Ratio, false)],
            )
        },
        // Each unit bobs ±0.25·k em, up first, with TX-023's phase delay.
        PresetManifest {
            units: Some(vec![AnimUnit::Letter, AnimUnit::Word]),
            defaults: ParamDefaults {
                overlap: PerMille::ZERO,
                ..ParamDefaults::default()
            },
            ..manifest(
                "fc.loop.wave",
                PresetKind::Loop,
                1200,
                Interp::Linear,
                vec![channel(P::TranslateY, Curve::Sine, -0.25, U::Em, true)],
            )
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_names_round_trip_and_unknown_ones_are_refused() {
        for c in ChannelProp::ALL {
            assert_eq!(ChannelProp::parse(c.as_str()), Some(c));
        }
        assert_eq!(ChannelProp::parse("glitch"), None);
        assert_eq!(ChannelProp::parse(""), None);
    }

    #[test]
    fn samples_are_well_formed_and_resolve() {
        let table = PresetTable::samples();
        assert_eq!(table.manifests().count(), 6);
        for m in table.manifests() {
            assert_eq!(m.check(), Ok(()), "{}", m.id);
            assert_eq!(m.default_spec().preset, m.id);
        }
        // An exit without its own manifest plays its entrance backwards.
        let out = PresetId::new("fc.out.fade");
        assert_eq!(
            table.resolve(Slot::Exit, &out).unwrap().id.as_str(),
            "fc.in.fade"
        );
        assert!(table.resolve(Slot::Entrance, &out).is_none());
        assert!(
            table
                .resolve(Slot::Exit, &PresetId::new("fc.out.nope"))
                .is_none()
        );
        assert!(
            NoPresets
                .resolve(Slot::Entrance, &PresetId::new("fc.in.fade"))
                .is_none()
        );
        let tw = table.get(&PresetId::new("fc.in.typewriter")).unwrap();
        assert!(tw.is_per_unit());
        assert_eq!(tw.default_spec().params.pace, Some(secs_milli(60)));
        assert_eq!(tw.default_spec().params.unit, Some(AnimUnit::Letter));
        assert!(
            !table
                .get(&PresetId::new("fc.in.fade"))
                .unwrap()
                .is_per_unit()
        );
    }

    #[test]
    fn checks_catch_bad_data() {
        let mut m = samples().remove(0);
        m.channels[0].amplitude = f64::NAN;
        assert_eq!(m.check(), Err(ManifestError::Amplitude));
        m.channels.clear();
        assert_eq!(m.check(), Err(ManifestError::NoChannels));
        m.default_duration = Tick::ZERO;
        assert_eq!(m.check(), Err(ManifestError::Duration));
        assert!(Curve::sampled(vec![], false).is_none());
        assert!(
            Curve::sampled(
                vec![CurvePoint { t: 0.5, v: 0.0 }, CurvePoint { t: 0.5, v: 1.0 }],
                false
            )
            .is_none()
        );
        assert!(Curve::sampled(vec![CurvePoint { t: 0.0, v: 1.0 }], true).is_some());
    }

    #[test]
    fn helpers_describe_what_a_manifest_needs() {
        let table = PresetTable::samples();
        let wave = table.get(&PresetId::new("fc.loop.wave")).unwrap();
        assert!(!wave.uses_seed() && !wave.needs_line_geometry());
        assert!(!wave.style_channels().any());
        let half = wave.scaled(0.5);
        assert_eq!(half.channels[0].amplitude, -0.125);
        assert_eq!(half.id, wave.id);
        let mut m = (*wave).clone();
        m.channels[0].property = ChannelProp::JitterRotate;
        assert!(m.uses_seed());
        m.channels[0].property = ChannelProp::Decode;
        assert!(m.uses_seed(), "Decode's stand-ins are seeded");
        m.channels[0].property = ChannelProp::LineRise;
        assert!(m.needs_line_geometry() && !m.uses_seed());
        m.channels[0].property = ChannelProp::Style(StyleChannel::Lightness);
        let style = m.style_channels();
        assert!(style.lightness && style.any() && !style.hue);
        m.defaults.order = AnimOrder::Random;
        assert!(m.uses_seed());
        // Catalogues answer safety by id; nothing is safe in no catalogue.
        assert!(table.is_safe(&PresetId::new("fc.in.fade")));
        assert!(!NoPresets.is_safe(&PresetId::new("fc.in.fade")));
        assert_eq!(
            NoPresets.photosensitivity(&PresetId::new("fc.in.fade")),
            None
        );
    }
}
