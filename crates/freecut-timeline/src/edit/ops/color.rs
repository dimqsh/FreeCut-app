//! Colour adjustments (CL-007, CL-008, CL-013, CL-014, CL-015, CL-023,
//! CL-024, CL-035): one
//! [`ColorEdit`] on every listed clip that takes a colour stack (video,
//! images and adjustment clips). Others are skipped; the edit is refused
//! when none takes one, and clips on locked tracks refuse it.
//!
//! Values are clamped into their [`crate::model::color_param_spec`] ranges.
//! A stack left at its defaults is stored as `None`, so an edit that
//! changes nothing records nothing. Setting a value writes the static
//! value; a keyed slider is edited through its keys (`EditKeys`, which the
//! app sends for a slider that has keys, TR-027).

use super::props::update_clips;
use crate::edit::tx::Tx;
use crate::edit::{EditError, OpKind, Plan};
use crate::ids::{ClipId, SeqId};
use crate::model::{
    Clip, ColorParamId, ColorStack, Curve, CurveChannel, HueBand, PropPath, Section, WheelId,
};

/// One change to the colour stacks of clips.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ColorEdit {
    /// One slider's static value (CL-007, CL-008, CL-035).
    SetParam(ColorParamId, i16),
    /// Several sliders at once, as one entry (in order; a later value of
    /// the same slider wins).
    SetParams(Vec<(ColorParamId, i16)>),
    /// A section's switch (CL-023): off keeps its values and keys.
    SetSectionEnabled(Section, bool),
    /// The master bypass (CL-023).
    SetBypass(bool),
    /// **Reset section** (CL-024): its values back to defaults, its switch
    /// on and its keys removed, in the same entry. Amount stays.
    ResetSection(Section),
    /// **Reset all color** (CL-024): the default stack, Amount 100 and no
    /// colour keys.
    ResetAll,
    /// One curve's points (CL-014): a drag, an added or deleted point, or
    /// a curve reset. Curves take no keys.
    SetCurve { channel: CurveChannel, curve: Curve },
    /// A wheel's puck (CL-015), pulled onto the disk. A keyed puck is
    /// edited through its keys, like a keyed slider.
    SetPuck { wheel: WheelId, x: i16, y: i16 },
    /// One HSL band back to its defaults (CL-013's swatch reset), its keys
    /// removed in the same entry.
    ResetBand(HueBand),
    /// One wheel's puck centred and its luminance at 0 (CL-015's reset
    /// button), its keys removed in the same entry.
    ResetWheel(WheelId),
}

/// What a colour edit did, for its undo label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorOp {
    /// Slider values.
    Set,
    /// A section switched on or off.
    Switch,
    /// The master bypass.
    Bypass,
    /// A section or the whole stack reset.
    Reset,
    /// A curve's points.
    Curve,
    /// A wheel's puck.
    Puck,
}

impl ColorEdit {
    pub fn op(&self) -> ColorOp {
        match self {
            ColorEdit::SetParam(..) | ColorEdit::SetParams(_) => ColorOp::Set,
            ColorEdit::SetSectionEnabled(..) => ColorOp::Switch,
            ColorEdit::SetBypass(_) => ColorOp::Bypass,
            ColorEdit::ResetSection(_)
            | ColorEdit::ResetAll
            | ColorEdit::ResetBand(_)
            | ColorEdit::ResetWheel(_) => ColorOp::Reset,
            ColorEdit::SetCurve { .. } => ColorOp::Curve,
            ColorEdit::SetPuck { .. } => ColorOp::Puck,
        }
    }

    /// `clip` after the edit; `None` when it takes no colour stack.
    pub fn applied_to(&self, clip: &Clip) -> Option<Clip> {
        if !clip.content.takes_color() {
            return None;
        }
        let stack = ColorStack::of(clip.color.as_ref());
        let mut out = clip.clone();
        let after = match self {
            ColorEdit::SetParam(id, v) => stack.with(*id, *v),
            ColorEdit::SetParams(values) => values.iter().fold(stack, |s, (id, v)| s.with(*id, *v)),
            ColorEdit::SetSectionEnabled(section, on) => ColorStack {
                sections_enabled: stack.sections_enabled.with(*section, *on),
                ..stack
            },
            ColorEdit::SetBypass(bypass) => ColorStack {
                bypass: *bypass,
                ..stack
            },
            ColorEdit::ResetSection(section) => {
                let section = *section;
                out.keys = out
                    .keys
                    .without(|p| p.group() == Some(crate::model::PropGroup::Color(section)));
                stack.with_section_reset(section)
            }
            ColorEdit::ResetAll => {
                out.keys = out.keys.without(is_color_prop);
                ColorStack::default()
            }
            ColorEdit::SetCurve { channel, curve } => stack.with_curve(*channel, curve.clone()),
            ColorEdit::SetPuck { wheel, x, y } => stack.with_puck(*wheel, *x, *y),
            ColorEdit::ResetBand(band) => {
                let band = *band;
                out.keys = out.keys.without(|p| {
                    matches!(p, PropPath::Color(ColorParamId::Hsl { band: b, .. }) if b == band)
                });
                stack.with_band_reset(band)
            }
            ColorEdit::ResetWheel(wheel) => {
                let wheel = *wheel;
                out.keys = out.keys.without(|p| {
                    p == PropPath::ColorPuck(wheel)
                        || p == PropPath::Color(ColorParamId::WheelLuma(wheel))
                });
                stack.with_wheel_reset(wheel)
            }
        };
        out.color = ColorStack::stored(after);
        Some(out)
    }
}

pub(crate) fn edit_color(
    tx: Tx<'_>,
    seq: SeqId,
    clips: &[ClipId],
    edit: &ColorEdit,
) -> Result<Plan, EditError> {
    update_clips(
        tx,
        seq,
        clips,
        OpKind::EditColor(edit.op()),
        EditError::NoColorClip,
        |clip| edit.applied_to(clip),
    )
}

/// Whether `prop` is a colour control's (a slider or a wheel's puck).
pub fn is_color_prop(prop: PropPath) -> bool {
    matches!(prop, PropPath::Color(_) | PropPath::ColorPuck(_))
}

/// Whether `clip`'s colour shows any change from its footage at some time:
/// a stored stack or a colour key (the timeline's colour badge, CL-001).
pub fn has_color(clip: &Clip) -> bool {
    clip.color.is_some() || clip.keys.tracks().iter().any(|t| is_color_prop(t.prop()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{ClipId, MediaId};
    use crate::model::{ClipContent, MediaKind, TextClipData};
    use crate::time::{Tick, TimeRange};
    use core::num::NonZeroU64;
    use std::sync::Arc;

    fn clip(content: ClipContent) -> Clip {
        Clip::new(
            ClipId::from_raw(NonZeroU64::new(1).unwrap()),
            TimeRange::new(Tick::ZERO, Tick::from_raw(100)).unwrap(),
            content,
        )
    }

    fn video() -> Clip {
        clip(ClipContent::Media {
            asset: MediaId::from_raw(NonZeroU64::new(9).unwrap()),
            kind: MediaKind::Video,
            source: TimeRange::new(Tick::ZERO, Tick::from_raw(100)).unwrap(),
            stream: None,
        })
    }

    #[test]
    fn edits_store_none_at_the_defaults() {
        let v = video();
        let warm = ColorEdit::SetParam(ColorParamId::Temperature, 40)
            .applied_to(&v)
            .unwrap();
        assert_eq!(warm.color.as_ref().unwrap().basic.temperature, 40);
        let back = ColorEdit::SetParam(ColorParamId::Temperature, 0)
            .applied_to(&warm)
            .unwrap();
        assert_eq!(back, v, "back to defaults stores nothing");
        let clamped = ColorEdit::SetParams(vec![
            (ColorParamId::Exposure, 900),
            (ColorParamId::Amount, 40),
        ])
        .applied_to(&v)
        .unwrap();
        let s = clamped.color.unwrap();
        assert_eq!((s.basic.exposure, s.amount), (100, 40));
    }

    #[test]
    fn text_and_audio_take_no_stack() {
        let text = clip(ClipContent::Text(Arc::new(TextClipData::with_content(
            "Hi",
        ))));
        assert_eq!(
            ColorEdit::SetParam(ColorParamId::Hue, 1).applied_to(&text),
            None
        );
        let audio = clip(ClipContent::Media {
            asset: MediaId::from_raw(NonZeroU64::new(9).unwrap()),
            kind: MediaKind::Audio,
            source: TimeRange::new(Tick::ZERO, Tick::from_raw(100)).unwrap(),
            stream: None,
        });
        assert_eq!(ColorEdit::ResetAll.applied_to(&audio), None);
        assert!(
            clip(ClipContent::Generated {
                kind: crate::model::GeneratedKind::Adjustment
            })
            .content
            .takes_color()
        );
    }
}
