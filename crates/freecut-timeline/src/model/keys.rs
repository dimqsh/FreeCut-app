//! Keyframed clip properties (TR-027, TR-028, TX-021): which properties
//! take keys, the value types they hold, and the per-clip key store.
//!
//! *FreeCut decision (S2 model):* a property's [`Animatable`] is its static
//! value, kept where it always lives (the clip's [`Transform`], the text
//! clip's style and layout), plus its keys, kept in the clip's
//! [`ClipKeys`] under the property's [`PropPath`]. [`Clip::animatable`]
//! joins the two without copying. Clips without keys therefore carry
//! nothing extra, every reader of static values stays as it is, and the
//! rest pose (what a renderer without keyframe support would show) is the
//! static value.
//!
//! Key times are the clip's content time (TR-027): source time for media;
//! for images, text and generated items, clip-local time plus
//! [`ClipKeys::origin`], which a split advances on the right piece and a
//! head trim moves with the in-point, so keys stick to the content.

use std::sync::Arc;

use super::animate::{Animatable, Interpolate, Keyframe, Progress};
use super::clip::{Clip, ClipContent};
use super::color::{ColorParamId, ColorStack, Section, WheelId};
use super::text::{
    DeciDegrees, Fill, RefPx, Rgba8, SignedPerMille, TailTip, TextCard, TextClipData, TextPropPath,
    card_props,
};
use super::transform::{Angle, CanvasPos, PerMille, Scale, Transform};
use crate::time::Tick;

/// A transform property that takes keys (TR-027).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TransformProp {
    Position,
    Scale,
    Rotation,
    Opacity,
}

impl TransformProp {
    pub const ALL: [TransformProp; 4] = [
        TransformProp::Position,
        TransformProp::Scale,
        TransformProp::Rotation,
        TransformProp::Opacity,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            TransformProp::Position => "transform.position",
            TransformProp::Scale => "transform.scale",
            TransformProp::Rotation => "transform.rotation",
            TransformProp::Opacity => "transform.opacity",
        }
    }
}

/// A keyable property of a clip, with a stable string form (preset files,
/// the command bar, project files): `transform.position`, any animatable
/// [`TextPropPath`], `text.fill.stops.N.color` and `text.reveal`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PropPath {
    Transform(TransformProp),
    Text(TextPropPath),
    /// The colour of gradient stop `N` (TX-014, TX-021); stop positions
    /// and count are not keyable.
    FillStopColor(u8),
    /// TX-021's **Reveal**: the share of characters shown, for custom
    /// typewriter effects. It has no static control: without keys every
    /// character shows, and removing the last key returns to that.
    Reveal,
    /// A colour slider of a video, image or adjustment clip (CL-028):
    /// `color.<name>`, valued [`PropValue::Level`] in its
    /// [`super::color::color_param_spec`] range.
    Color(ColorParamId),
    /// A colour wheel's puck (CL-015, CL-028): `color.wheels.<wheel>.puck`,
    /// valued [`PropValue::Puck`] on the disk.
    ColorPuck(WheelId),
}

/// The value type of a keyable property.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValueKind {
    Position,
    Scale,
    Angle,
    Ratio,
    Length,
    SignedRatio,
    Direction,
    Color,
    TailTip,
    /// A colour slider's level (CL-028).
    Level,
    /// A colour wheel's puck (CL-015).
    Puck,
}

/// A keyframe value. Its variant must match its property's [`ValueKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PropValue {
    Position(CanvasPos),
    Scale(Scale),
    Angle(Angle),
    Ratio(PerMille),
    Length(RefPx),
    SignedRatio(SignedPerMille),
    Direction(DeciDegrees),
    Color(Rgba8),
    TailTip(TailTip),
    /// A colour slider's level (CL-028), in its control's range.
    Level(i16),
    /// A colour wheel's puck (CL-015), on the disk of radius 1000.
    Puck {
        x: i16,
        y: i16,
    },
}

impl PropValue {
    pub const fn kind(self) -> ValueKind {
        match self {
            PropValue::Position(_) => ValueKind::Position,
            PropValue::Scale(_) => ValueKind::Scale,
            PropValue::Angle(_) => ValueKind::Angle,
            PropValue::Ratio(_) => ValueKind::Ratio,
            PropValue::Length(_) => ValueKind::Length,
            PropValue::SignedRatio(_) => ValueKind::SignedRatio,
            PropValue::Direction(_) => ValueKind::Direction,
            PropValue::Color(_) => ValueKind::Color,
            PropValue::TailTip(_) => ValueKind::TailTip,
            PropValue::Level(_) => ValueKind::Level,
            PropValue::Puck { .. } => ValueKind::Puck,
        }
    }
}

/// Each variant interpolates as its type does. Mismatched variants (never
/// in a valid document) hold the earlier value until the midpoint.
impl Interpolate for PropValue {
    fn interpolate(a: &PropValue, b: &PropValue, p: Progress) -> PropValue {
        use PropValue as V;
        match (a, b) {
            (V::Position(x), V::Position(y)) => V::Position(Interpolate::interpolate(x, y, p)),
            (V::Scale(x), V::Scale(y)) => V::Scale(Interpolate::interpolate(x, y, p)),
            (V::Angle(x), V::Angle(y)) => V::Angle(Interpolate::interpolate(x, y, p)),
            (V::Ratio(x), V::Ratio(y)) => V::Ratio(Interpolate::interpolate(x, y, p)),
            (V::Length(x), V::Length(y)) => V::Length(Interpolate::interpolate(x, y, p)),
            (V::SignedRatio(x), V::SignedRatio(y)) => {
                V::SignedRatio(Interpolate::interpolate(x, y, p))
            }
            (V::Direction(x), V::Direction(y)) => V::Direction(Interpolate::interpolate(x, y, p)),
            (V::Color(x), V::Color(y)) => V::Color(Interpolate::interpolate(x, y, p)),
            (V::TailTip(x), V::TailTip(y)) => V::TailTip(Interpolate::interpolate(x, y, p)),
            (V::Level(x), V::Level(y)) => V::Level(lerp_level(*x, *y, p)),
            // Per coordinate, so the puck travels in a straight line; the
            // stack pulls it back onto the disk (CL-015).
            (V::Puck { x: ax, y: ay }, V::Puck { x: bx, y: by }) => V::Puck {
                x: lerp_level(*ax, *bx, p),
                y: lerp_level(*ay, *by, p),
            },
            _ => {
                if p.is_past_half() {
                    *b
                } else {
                    *a
                }
            }
        }
    }
}

/// A colour level between two keys (CL-028): interpolated in `f64` from
/// the exact progress, rounded half away from zero to the step when
/// evaluated, saturated at the `i16` range (eased progress may overshoot;
/// the stack clamps into the control's range).
fn lerp_level(a: i16, b: i16, p: Progress) -> i16 {
    let (a, b) = (f64::from(a), f64::from(b));
    let v = (a + (b - a) * p.as_f64()).round();
    // Saturated into i16 first, so the cast is exact.
    v.clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16
}

/// A set of properties keyed together: a text card's **Key this card**
/// (TX-021), the transform's "Add keyframe for all transform
/// properties" (TR-028) or a colour section's sliders (CL-028).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PropGroup {
    Text(TextCard),
    Transform,
    Color(Section),
}

impl PropGroup {
    /// The keyable properties of this group that `clip` has, in row order
    /// (the Fill card lists the colour of every stop of the current
    /// gradient).
    pub fn props(self, clip: &Clip) -> Vec<PropPath> {
        let candidates: Vec<PropPath> = match self {
            PropGroup::Transform => TransformProp::ALL.map(PropPath::Transform).to_vec(),
            PropGroup::Text(card) => {
                let mut out: Vec<PropPath> = card_props(card)
                    .iter()
                    .filter(|p| p.is_animatable())
                    .map(|p| PropPath::Text(*p))
                    .collect();
                if card == TextCard::Fill {
                    out.extend((0..8u8).map(PropPath::FillStopColor));
                }
                out
            }
            PropGroup::Color(section) => {
                let mut out: Vec<PropPath> = section
                    .params()
                    .iter()
                    .map(|p| PropPath::Color(*p))
                    .collect();
                if section == Section::Wheels {
                    out.extend(WheelId::ALL.map(PropPath::ColorPuck));
                }
                out
            }
        };
        candidates
            .into_iter()
            .filter(|p| p.get(clip).is_some())
            .collect()
    }

    /// Whether `prop` belongs to this group.
    pub fn contains(self, prop: PropPath) -> bool {
        prop.group() == Some(self)
    }
}

impl PropPath {
    /// Every keyable text property, transform property and Reveal (stop
    /// colours are listed per stop by [`PropGroup::props`]).
    pub fn all_fixed() -> Vec<PropPath> {
        let mut out: Vec<PropPath> = TransformProp::ALL.map(PropPath::Transform).to_vec();
        out.extend(
            TextPropPath::ALL
                .iter()
                .filter(|p| p.is_animatable())
                .map(|p| PropPath::Text(*p)),
        );
        out.push(PropPath::Reveal);
        out.extend(ColorParamId::ALL.map(PropPath::Color));
        out.extend(WheelId::ALL.map(PropPath::ColorPuck));
        out
    }

    /// The value type, for keyable properties; `None` for text properties
    /// TX-021 doesn't let take keys.
    pub fn kind(self) -> Option<ValueKind> {
        use TextPropPath as T;
        use ValueKind as K;
        Some(match self {
            PropPath::Transform(TransformProp::Position) => K::Position,
            PropPath::Transform(TransformProp::Scale) => K::Scale,
            PropPath::Transform(TransformProp::Rotation) => K::Angle,
            PropPath::Transform(TransformProp::Opacity) => K::Ratio,
            PropPath::FillStopColor(_) => K::Color,
            PropPath::Reveal => K::Ratio,
            PropPath::Color(_) => K::Level,
            PropPath::ColorPuck(_) => K::Puck,
            PropPath::Text(p) => match p {
                T::Size => K::Length,
                T::FillColor
                | T::OutlineColor
                | T::GlowColor
                | T::ShadowColor
                | T::BackgroundColor => K::Color,
                T::FillAngle | T::ShadowAngle => K::Direction,
                T::LetterSpacing | T::BubbleTailBend | T::CurveAmount => K::SignedRatio,
                T::LineSpacing
                | T::OutlineWidth
                | T::OutlineOpacity
                | T::GlowIntensity
                | T::GlowRadius
                | T::GlowOffset
                | T::ShadowOpacity
                | T::ShadowBlur
                | T::ShadowDistance
                | T::BackgroundOpacity
                | T::BackgroundRoundness
                | T::BackgroundBorderWidth
                | T::BubbleTailBase => K::Ratio,
                T::BubbleTailTip => K::TailTip,
                _ => return None,
            },
        })
    }

    /// Whether the property takes keys (TR-027, TX-021).
    pub fn is_animatable(self) -> bool {
        self.kind().is_some()
    }

    /// The card or section holding the property; Reveal sits in the Motion
    /// lens's Keyframes card, which is no text card, and the colour Amount
    /// in the Color tab's header, which is no section (CL-035).
    pub fn group(self) -> Option<PropGroup> {
        match self {
            PropPath::Transform(_) => Some(PropGroup::Transform),
            PropPath::Text(p) => Some(PropGroup::Text(p.card())),
            PropPath::FillStopColor(_) => Some(PropGroup::Text(TextCard::Fill)),
            PropPath::Reveal => None,
            PropPath::Color(p) => p.section().map(PropGroup::Color),
            PropPath::ColorPuck(_) => Some(PropGroup::Color(Section::Wheels)),
        }
    }

    /// The stable string form.
    pub fn to_path_string(self) -> String {
        match self {
            PropPath::Transform(t) => t.as_str().to_string(),
            PropPath::Text(t) => t.as_str().to_string(),
            PropPath::FillStopColor(i) => format!("text.fill.stops.{i}.color"),
            PropPath::Reveal => "text.reveal".to_string(),
            PropPath::Color(p) => format!("color.{}", p.as_str()),
            PropPath::ColorPuck(w) => format!("color.wheels.{}.puck", w.as_str()),
        }
    }

    /// The keyable property named `path`, if it is one.
    pub fn parse(path: &str) -> Option<PropPath> {
        if path == "text.reveal" {
            return Some(PropPath::Reveal);
        }
        if let Some(name) = path.strip_prefix("color.") {
            if let Some(wheel) = name
                .strip_prefix("wheels.")
                .and_then(|rest| rest.strip_suffix(".puck"))
            {
                return WheelId::parse(wheel).map(PropPath::ColorPuck);
            }
            return ColorParamId::parse(name).map(PropPath::Color);
        }
        if let Some(t) = TransformProp::ALL.iter().find(|t| t.as_str() == path) {
            return Some(PropPath::Transform(*t));
        }
        if let Some(index) = path
            .strip_prefix("text.fill.stops.")
            .and_then(|rest| rest.strip_suffix(".color"))
        {
            let i: u8 = index.parse().ok()?;
            // Only the canonical decimal form round-trips.
            return (i.to_string() == index).then_some(PropPath::FillStopColor(i));
        }
        TextPropPath::parse(path)
            .filter(|p| p.is_animatable())
            .map(PropPath::Text)
    }

    /// The property's static value on `clip`; `None` when the clip doesn't
    /// have it: text properties on other clips, transform properties on
    /// audio clips, the solid colour of a gradient fill, the angle of a
    /// fill that isn't linear, stops the gradient doesn't have, colour
    /// sliders on clips without a colour stack (CL-001), the LUT's sliders
    /// on stacks without a LUT (CL-017).
    pub fn get(self, clip: &Clip) -> Option<PropValue> {
        match self {
            PropPath::Transform(p) => clip
                .content
                .is_visual()
                .then(|| transform_value(&clip.transform, p)),
            PropPath::Color(p) => {
                if !clip.content.takes_color() {
                    return None;
                }
                let stack = ColorStack::of(clip.color.as_ref());
                // The LUT section's sliders exist only while there is a LUT.
                if p.section() == Some(Section::Lut) && stack.lut.is_none() {
                    return None;
                }
                Some(PropValue::Level(stack.get(p)))
            }
            PropPath::ColorPuck(w) => clip.content.takes_color().then(|| {
                let (x, y) = ColorStack::of(clip.color.as_ref()).puck(w);
                PropValue::Puck { x, y }
            }),
            _ => text_value(clip.content.text()?, self),
        }
    }

    /// Writes `value` as the static value on `clip`, clamped into the
    /// property's range. False when the clip lacks the property or the
    /// value is of the wrong kind. Reveal has no static value (see
    /// [`PropPath::Reveal`]): writing it changes nothing.
    pub fn set(self, clip: &mut Clip, value: PropValue) -> bool {
        if self.kind() != Some(value.kind()) || self.get(clip).is_none() {
            return false;
        }
        match self {
            PropPath::Transform(p) => {
                set_transform(&mut clip.transform, p, value);
                clip.transform = clip.transform.clamped();
                true
            }
            PropPath::Color(p) => {
                let PropValue::Level(v) = value else {
                    return false;
                };
                clip.color = ColorStack::stored(ColorStack::of(clip.color.as_ref()).with(p, v));
                true
            }
            PropPath::ColorPuck(w) => {
                let PropValue::Puck { x, y } = value else {
                    return false;
                };
                clip.color =
                    ColorStack::stored(ColorStack::of(clip.color.as_ref()).with_puck(w, x, y));
                true
            }
            _ => {
                let ClipContent::Text(data) = &mut clip.content else {
                    return false;
                };
                let data = Arc::make_mut(data);
                let ok = set_text(data, self, value);
                *data = data.clamped();
                ok
            }
        }
    }

    /// `value` clamped into this property's range on `clip`, as a key
    /// would hold it; `None` when the clip lacks the property or the kind
    /// is wrong.
    pub fn clamp_value(self, clip: &Clip, value: PropValue) -> Option<PropValue> {
        if self == PropPath::Reveal {
            return match value {
                PropValue::Ratio(r) => Some(PropValue::Ratio(PerMille::new(r.get().min(1000)))),
                _ => None,
            };
        }
        let mut probe = clip.clone();
        if !self.set(&mut probe, value) {
            return None;
        }
        self.get(&probe)
    }
}

impl core::fmt::Display for PropPath {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.to_path_string())
    }
}

fn transform_value(t: &Transform, p: TransformProp) -> PropValue {
    match p {
        TransformProp::Position => PropValue::Position(t.position),
        TransformProp::Scale => PropValue::Scale(t.scale),
        TransformProp::Rotation => PropValue::Angle(t.rotation),
        TransformProp::Opacity => PropValue::Ratio(t.opacity),
    }
}

fn set_transform(t: &mut Transform, p: TransformProp, v: PropValue) {
    match (p, v) {
        (TransformProp::Position, PropValue::Position(x)) => t.position = x,
        (TransformProp::Scale, PropValue::Scale(x)) => t.scale = x,
        (TransformProp::Rotation, PropValue::Angle(x)) => t.rotation = x,
        (TransformProp::Opacity, PropValue::Ratio(x)) => t.opacity = x,
        _ => {}
    }
}

fn text_value(d: &TextClipData, prop: PropPath) -> Option<PropValue> {
    use PropValue as V;
    use TextPropPath as T;
    let s = &d.style;
    Some(match prop {
        PropPath::Reveal => V::Ratio(PerMille::ONE),
        PropPath::FillStopColor(i) => {
            V::Color(s.fill.stops()?.as_slice().get(usize::from(i))?.color)
        }
        PropPath::Transform(_) | PropPath::Color(_) | PropPath::ColorPuck(_) => return None,
        PropPath::Text(p) => match p {
            T::Size => V::Length(s.size),
            T::FillColor => V::Color(s.fill.solid()?),
            T::FillAngle => match &s.fill {
                Fill::LinearGradient(g) => V::Direction(g.angle),
                _ => return None,
            },
            T::LetterSpacing => V::SignedRatio(s.letter_spacing),
            T::LineSpacing => V::Ratio(d.layout.line_spacing),
            T::OutlineColor => V::Color(s.stroke.value.color),
            T::OutlineWidth => V::Ratio(s.stroke.value.width),
            T::OutlineOpacity => V::Ratio(s.stroke.value.opacity),
            T::GlowColor => V::Color(s.glow.value.color),
            T::GlowIntensity => V::Ratio(s.glow.value.intensity),
            T::GlowRadius => V::Ratio(s.glow.value.radius),
            T::GlowOffset => V::Ratio(s.glow.value.offset),
            T::ShadowColor => V::Color(s.shadow.value.color),
            T::ShadowOpacity => V::Ratio(s.shadow.value.opacity),
            T::ShadowBlur => V::Ratio(s.shadow.value.blur),
            T::ShadowDistance => V::Ratio(s.shadow.value.distance),
            T::ShadowAngle => V::Direction(s.shadow.value.angle),
            T::BackgroundColor => V::Color(s.background.value.color),
            T::BackgroundOpacity => V::Ratio(s.background.value.opacity),
            T::BackgroundRoundness => V::Ratio(s.background.value.roundness),
            T::BackgroundBorderWidth => V::Ratio(s.background.value.border.value.width),
            T::BubbleTailTip => V::TailTip(s.bubble.value.spec.tail.value.tip),
            T::BubbleTailBase => V::Ratio(s.bubble.value.spec.tail.value.base),
            T::BubbleTailBend => V::SignedRatio(s.bubble.value.spec.tail.value.bend),
            T::CurveAmount => V::SignedRatio(s.curve.value),
            _ => return None,
        },
    })
}

/// Writes one text value (unclamped; the caller clamps the whole data).
fn set_text(d: &mut TextClipData, prop: PropPath, v: PropValue) -> bool {
    use PropValue as V;
    use TextPropPath as T;
    let s = &mut d.style;
    match (prop, v) {
        (PropPath::Reveal, V::Ratio(_)) => {}
        (PropPath::FillStopColor(i), V::Color(c)) => {
            let Some(stops) = s
                .fill
                .stops()
                .and_then(|st| st.with_color(usize::from(i), c))
            else {
                return false;
            };
            match &mut s.fill {
                Fill::LinearGradient(g) => g.stops = stops,
                Fill::RadialGradient(g) => g.stops = stops,
                Fill::Solid(_) => return false,
            }
        }
        (PropPath::Text(p), v) => match (p, v) {
            (T::Size, V::Length(x)) => s.size = x,
            (T::FillColor, V::Color(x)) => match &mut s.fill {
                Fill::Solid(c) => *c = x,
                _ => return false,
            },
            (T::FillAngle, V::Direction(x)) => match &mut s.fill {
                Fill::LinearGradient(g) => g.angle = x,
                _ => return false,
            },
            (T::LetterSpacing, V::SignedRatio(x)) => s.letter_spacing = x,
            (T::LineSpacing, V::Ratio(x)) => d.layout.line_spacing = x,
            (T::OutlineColor, V::Color(x)) => s.stroke.value.color = x,
            (T::OutlineWidth, V::Ratio(x)) => s.stroke.value.width = x,
            (T::OutlineOpacity, V::Ratio(x)) => s.stroke.value.opacity = x,
            (T::GlowColor, V::Color(x)) => s.glow.value.color = x,
            (T::GlowIntensity, V::Ratio(x)) => s.glow.value.intensity = x,
            (T::GlowRadius, V::Ratio(x)) => s.glow.value.radius = x,
            (T::GlowOffset, V::Ratio(x)) => s.glow.value.offset = x,
            (T::ShadowColor, V::Color(x)) => s.shadow.value.color = x,
            (T::ShadowOpacity, V::Ratio(x)) => s.shadow.value.opacity = x,
            (T::ShadowBlur, V::Ratio(x)) => s.shadow.value.blur = x,
            (T::ShadowDistance, V::Ratio(x)) => s.shadow.value.distance = x,
            (T::ShadowAngle, V::Direction(x)) => s.shadow.value.angle = x,
            (T::BackgroundColor, V::Color(x)) => s.background.value.color = x,
            (T::BackgroundOpacity, V::Ratio(x)) => s.background.value.opacity = x,
            (T::BackgroundRoundness, V::Ratio(x)) => s.background.value.roundness = x,
            (T::BackgroundBorderWidth, V::Ratio(x)) => s.background.value.border.value.width = x,
            (T::BubbleTailTip, V::TailTip(x)) => s.bubble.value.spec.tail.value.tip = x,
            (T::BubbleTailBase, V::Ratio(x)) => s.bubble.value.spec.tail.value.base = x,
            (T::BubbleTailBend, V::SignedRatio(x)) => s.bubble.value.spec.tail.value.bend = x,
            (T::CurveAmount, V::SignedRatio(x)) => s.curve.value = x,
            _ => return false,
        },
        _ => return false,
    }
    true
}

/// The keys of one property: never empty, strictly increasing in time.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyTrack {
    prop: PropPath,
    keys: Arc<[Keyframe<PropValue>]>,
}

impl KeyTrack {
    pub fn prop(&self) -> PropPath {
        self.prop
    }

    pub fn keys(&self) -> &[Keyframe<PropValue>] {
        &self.keys
    }
}

/// Why stored keys were refused (project loading).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeysError {
    #[error("{0} doesn't take keyframes")]
    NotAnimatable(PropPath),
    #[error("{0} is listed twice")]
    Repeated(PropPath),
    #[error("{0} has no keys")]
    Empty(PropPath),
    #[error("{0}: keyframe times must be strictly increasing")]
    Unordered(PropPath),
    #[error("{0}: a keyframe holds the wrong kind of value")]
    WrongKind(PropPath),
}

/// A clip's keyframes, by property (TR-027), and the content time of the
/// clip's start for clips without a source range. Sorted by property;
/// shared, so clip clones stay cheap.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct ClipKeys {
    origin: Tick,
    tracks: Option<Arc<[KeyTrack]>>,
}

impl ClipKeys {
    /// Builds from stored parts (project loading): every property keyable,
    /// listed once, with keys in strictly increasing time order and values
    /// of its kind. Properties are sorted; empty lists are refused.
    pub fn from_parts(
        origin: Tick,
        tracks: Vec<(PropPath, Vec<Keyframe<PropValue>>)>,
    ) -> Result<ClipKeys, KeysError> {
        let mut out: Vec<KeyTrack> = Vec::with_capacity(tracks.len());
        for (prop, keys) in tracks {
            let kind = prop.kind().ok_or(KeysError::NotAnimatable(prop))?;
            if keys.is_empty() {
                return Err(KeysError::Empty(prop));
            }
            if keys.windows(2).any(|w| w[0].at >= w[1].at) {
                return Err(KeysError::Unordered(prop));
            }
            if keys.iter().any(|k| k.value.kind() != kind) {
                return Err(KeysError::WrongKind(prop));
            }
            out.push(KeyTrack {
                prop,
                keys: Arc::from(keys),
            });
        }
        out.sort_by_key(|t| t.prop);
        if let Some(w) = out.windows(2).find(|w| w[0].prop == w[1].prop) {
            return Err(KeysError::Repeated(w[0].prop));
        }
        Ok(ClipKeys {
            origin,
            tracks: (!out.is_empty()).then(|| Arc::from(out)),
        })
    }

    /// The content time of the clip's start, for clips without a source
    /// range (TR-027). 0 for a new clip.
    pub fn origin(&self) -> Tick {
        self.origin
    }

    /// The keyed properties, sorted by property.
    pub fn tracks(&self) -> &[KeyTrack] {
        self.tracks.as_deref().unwrap_or(&[])
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_none()
    }

    /// The keys of `prop` (empty when it has none).
    pub fn keys(&self, prop: PropPath) -> &[Keyframe<PropValue>] {
        self.track(prop).map_or(&[], |t| &t.keys)
    }

    pub fn is_animated(&self, prop: PropPath) -> bool {
        self.track(prop).is_some()
    }

    fn track(&self, prop: PropPath) -> Option<&KeyTrack> {
        let tracks = self.tracks();
        tracks
            .binary_search_by_key(&prop, |t| t.prop)
            .ok()
            .map(|i| &tracks[i])
    }

    /// The same keys with another origin.
    pub(crate) fn with_origin(&self, origin: Tick) -> ClipKeys {
        ClipKeys {
            origin,
            tracks: self.tracks.clone(),
        }
    }

    /// `prop`'s keys replaced by `keys` (an empty list removes them).
    pub(crate) fn with_track(&self, prop: PropPath, keys: Arc<[Keyframe<PropValue>]>) -> ClipKeys {
        let mut tracks: Vec<KeyTrack> = self
            .tracks()
            .iter()
            .filter(|t| t.prop != prop)
            .cloned()
            .collect();
        if !keys.is_empty() {
            tracks.push(KeyTrack { prop, keys });
            tracks.sort_by_key(|t| t.prop);
        }
        ClipKeys {
            origin: self.origin,
            tracks: (!tracks.is_empty()).then(|| Arc::from(tracks)),
        }
    }

    /// Without the keys of every property `drop` names.
    pub(crate) fn without(&self, drop: impl Fn(PropPath) -> bool) -> ClipKeys {
        let tracks: Vec<KeyTrack> = self
            .tracks()
            .iter()
            .filter(|t| !drop(t.prop))
            .cloned()
            .collect();
        ClipKeys {
            origin: self.origin,
            tracks: (!tracks.is_empty()).then(|| Arc::from(tracks)),
        }
    }
}

impl Clip {
    /// The content time shown at sequence time `at` (TR-027): where keys
    /// are anchored. Media clips use their source time; other clips their
    /// local time plus [`ClipKeys::origin`]. `None` only on overflow.
    pub fn content_time_at(&self, at: Tick) -> Option<Tick> {
        if self.content.source().is_some() {
            return self.source_time_at(at);
        }
        let local = at.checked_sub(self.start()).ok()?;
        local.checked_add(self.keys.origin()).ok()
    }

    /// The sequence time at which content time `content` shows: the
    /// inverse of [`Clip::content_time_at`].
    pub fn sequence_time_of(&self, content: Tick) -> Option<Tick> {
        let base = match self.content.source() {
            Some(source) => source.start(),
            None => self.keys.origin(),
        };
        let local = content.checked_sub(base).ok()?;
        self.start().checked_add(local).ok()
    }

    /// `prop` as an [`Animatable`]: its static value and its keys (shared,
    /// not copied). `None` when the clip doesn't have the property.
    pub fn animatable(&self, prop: PropPath) -> Option<Animatable<PropValue>> {
        let value = prop.get(self)?;
        Some(match self.keys.track(prop) {
            Some(track) => Animatable::from_shared(value, Arc::clone(&track.keys)),
            None => Animatable::new(value),
        })
    }

    /// The value of `prop` at content time `at`.
    pub fn value_at(&self, prop: PropPath, at: Tick) -> Option<PropValue> {
        let value = prop.get(self)?;
        Some(match self.keys.track(prop) {
            Some(track) => Animatable::from_shared(value, Arc::clone(&track.keys)).value_at(at),
            None => value,
        })
    }

    /// The transform at content time `at`: keyed values evaluated, every
    /// value clamped into its range (TR-031's clamps at evaluation).
    pub fn transform_at(&self, at: Tick) -> Transform {
        let mut out = self.transform;
        let mut keyed = false;
        for track in self.keys.tracks() {
            if let PropPath::Transform(p) = track.prop {
                let a = Animatable::from_shared(transform_value(&out, p), Arc::clone(&track.keys));
                set_transform(&mut out, p, a.value_at(at));
                keyed = true;
            }
        }
        if keyed { out.clamped() } else { out }
    }

    /// The text at content time `at` (TX-022 step 1, keys only): keyed
    /// style values evaluated and clamped into range. The same `Arc` when
    /// no text property is keyed. `None` for other clips.
    pub fn text_at(&self, at: Tick) -> Option<Arc<TextClipData>> {
        let data = self.content.text()?;
        let mut tracks = self
            .keys
            .tracks()
            .iter()
            .filter(|t| matches!(t.prop, PropPath::Text(_) | PropPath::FillStopColor(_)))
            .peekable();
        if tracks.peek().is_none() {
            return Some(Arc::clone(data));
        }
        let mut out = TextClipData::clone(data);
        for track in tracks {
            // Properties the data lacks (an inactive fill part) are skipped.
            let Some(value) = text_value(&out, track.prop) else {
                continue;
            };
            let a = Animatable::from_shared(value, Arc::clone(&track.keys));
            set_text(&mut out, track.prop, a.value_at(at));
        }
        Some(Arc::new(out.clamped()))
    }

    /// The colour stack at content time `at` (CL-028): keyed sliders
    /// evaluated and clamped into their ranges over the static values. The
    /// same `Arc` (or `None`) when no colour control is keyed.
    pub fn color_at(&self, at: Tick) -> Option<Arc<ColorStack>> {
        let mut tracks = self
            .keys
            .tracks()
            .iter()
            .filter(|t| matches!(t.prop, PropPath::Color(_) | PropPath::ColorPuck(_)))
            .peekable();
        if tracks.peek().is_none() || !self.content.takes_color() {
            return self.color.clone();
        }
        let mut out = ColorStack::of(self.color.as_ref());
        for track in tracks {
            match track.prop {
                PropPath::Color(p) => {
                    let rest = PropValue::Level(out.get(p));
                    let a = Animatable::from_shared(rest, Arc::clone(&track.keys));
                    if let PropValue::Level(v) = a.value_at(at) {
                        out = out.with(p, v);
                    }
                }
                PropPath::ColorPuck(w) => {
                    let (x, y) = out.puck(w);
                    let a =
                        Animatable::from_shared(PropValue::Puck { x, y }, Arc::clone(&track.keys));
                    if let PropValue::Puck { x, y } = a.value_at(at) {
                        out = out.with_puck(w, x, y);
                    }
                }
                _ => {}
            }
        }
        Some(Arc::new(out))
    }

    /// TX-021's Reveal at content time `at`: the share of characters shown
    /// (1000 without keys).
    pub fn reveal_at(&self, at: Tick) -> PerMille {
        match self.value_at(PropPath::Reveal, at) {
            Some(PropValue::Ratio(r)) => PerMille::new(r.get().min(1000)),
            _ => PerMille::ONE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::text::{FillKind, GradientStops};
    use crate::time::TimeRange;
    use core::num::NonZeroU64;

    fn t(v: i64) -> Tick {
        Tick::from_raw(v)
    }

    fn text_clip() -> Clip {
        Clip::new(
            crate::ids::ClipId::from_raw(NonZeroU64::new(1).unwrap()),
            TimeRange::new(t(100), t(1000)).unwrap(),
            ClipContent::Text(Arc::new(TextClipData::with_content("Hi"))),
        )
    }

    #[test]
    fn paths_round_trip_and_name_their_kind() {
        let mut all = PropPath::all_fixed();
        all.extend((0..8).map(PropPath::FillStopColor));
        for p in &all {
            assert_eq!(PropPath::parse(&p.to_path_string()), Some(*p), "{p}");
            assert!(p.is_animatable());
        }
        assert_eq!(PropPath::parse("text.font"), None, "not keyable");
        assert_eq!(PropPath::parse("text.fill.stops.01.color"), None);
        assert_eq!(PropPath::parse("transform.flip_h"), None);
        assert_eq!(
            PropPath::Transform(TransformProp::Position).to_path_string(),
            "transform.position"
        );
        // Every animatable text path has a kind, and only those.
        for p in TextPropPath::ALL {
            assert_eq!(PropPath::Text(*p).is_animatable(), p.is_animatable(), "{p}");
        }
    }

    #[test]
    fn values_live_where_they_always_did() {
        let mut clip = text_clip();
        let size = PropPath::Text(TextPropPath::Size);
        assert_eq!(size.get(&clip), Some(PropValue::Length(RefPx::from_px(64))));
        assert!(size.set(&mut clip, PropValue::Length(RefPx::from_px(1))));
        assert_eq!(
            clip.content.text().unwrap().style.size,
            RefPx::from_px(4),
            "clamped"
        );
        assert!(
            !size.set(&mut clip, PropValue::Ratio(PerMille::ONE)),
            "wrong kind"
        );
        // Fill parts exist only on their fill kind.
        assert!(PropPath::Text(TextPropPath::FillAngle).get(&clip).is_none());
        assert!(PropPath::FillStopColor(0).get(&clip).is_none());
        let data = Arc::make_mut(match &mut clip.content {
            ClipContent::Text(d) => d,
            _ => unreachable!(),
        });
        data.style.fill = data.style.fill.with_kind(FillKind::LinearGradient);
        assert!(PropPath::Text(TextPropPath::FillColor).get(&clip).is_none());
        assert!(PropPath::FillStopColor(1).get(&clip).is_some());
        assert!(PropPath::FillStopColor(2).get(&clip).is_none());
        assert_eq!(
            PropGroup::Text(TextCard::Fill).props(&clip),
            vec![
                PropPath::Text(TextPropPath::FillAngle),
                PropPath::FillStopColor(0),
                PropPath::FillStopColor(1)
            ]
        );
        let _ = GradientStops::two(Rgba8::BLACK, Rgba8::WHITE);
    }

    #[test]
    fn evaluation_reads_keys_in_content_time() {
        let mut clip = text_clip();
        let pos = PropPath::Transform(TransformProp::Position);
        let a = clip
            .animatable(pos)
            .unwrap()
            .with_key(t(0), PropValue::Position(CanvasPos::new(0, 0)))
            .with_key(t(100), PropValue::Position(CanvasPos::new(1000, 0)));
        clip.keys = clip.keys.with_track(pos, Arc::clone(a.shared_keys()));
        assert_eq!(clip.content_time_at(t(150)), Some(t(50)));
        assert_eq!(clip.sequence_time_of(t(50)), Some(t(150)));
        assert_eq!(clip.transform_at(t(50)).position, CanvasPos::new(500, 0));
        assert_eq!(clip.transform.position, CanvasPos::CENTER, "static kept");
        clip.keys = clip.keys.with_origin(t(40));
        assert_eq!(clip.content_time_at(t(150)), Some(t(90)));
        // Text without text keys keeps its Arc.
        let same = clip.text_at(t(0)).unwrap();
        assert!(Arc::ptr_eq(&same, clip.content.text().unwrap()));
        assert_eq!(clip.reveal_at(t(0)), PerMille::ONE);
    }

    #[test]
    fn stored_keys_are_checked() {
        let key = |at, v| Keyframe {
            at: t(at),
            value: v,
            interp: crate::model::Interp::Linear,
        };
        let r = |v| PropValue::Ratio(PerMille::new(v));
        let op = PropPath::Transform(TransformProp::Opacity);
        assert!(ClipKeys::from_parts(t(0), vec![(op, vec![key(0, r(1)), key(5, r(2))])]).is_ok());
        assert_eq!(
            ClipKeys::from_parts(t(0), vec![(op, vec![key(5, r(1)), key(5, r(2))])]),
            Err(KeysError::Unordered(op))
        );
        assert_eq!(
            ClipKeys::from_parts(t(0), vec![(op, vec![])]),
            Err(KeysError::Empty(op))
        );
        assert_eq!(
            ClipKeys::from_parts(t(0), vec![(op, vec![key(0, PropValue::Scale(Scale::ONE))])]),
            Err(KeysError::WrongKind(op))
        );
        let font = PropPath::Text(TextPropPath::Font);
        assert_eq!(
            ClipKeys::from_parts(t(0), vec![(font, vec![key(0, r(1))])]),
            Err(KeysError::NotAnimatable(font))
        );
        assert_eq!(
            ClipKeys::from_parts(
                t(0),
                vec![(op, vec![key(0, r(1))]), (op, vec![key(1, r(1))])]
            ),
            Err(KeysError::Repeated(op))
        );
        assert!(ClipKeys::from_parts(t(3), vec![]).unwrap().is_empty());
    }
}
