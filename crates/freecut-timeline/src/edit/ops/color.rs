//! Colour adjustments (CL-007, CL-008, CL-013, CL-014, CL-015, CL-017,
//! CL-019, CL-020, CL-023, CL-024, CL-030, CL-035): one
//! [`ColorEdit`] on every listed clip that takes a colour stack (video,
//! images and adjustment clips). Others are skipped; the edit is refused
//! when none takes one, and clips on locked tracks refuse it.
//!
//! Values are clamped into their [`crate::model::color_param_spec`] ranges.
//! A stack left at its defaults is stored as `None`, so an edit that
//! changes nothing records nothing. Setting a value writes the static
//! value; a keyed slider is edited through its keys (`EditKeys`, which the
//! app sends for a slider that has keys, TR-027).

use std::sync::Arc;

use super::props::update_clips;
use crate::edit::tx::Tx;
use crate::edit::{EditError, OpKind, Plan};
use crate::ids::{ClipId, SeqId};
use crate::model::{
    AutoCorrection, AutoOffsets, Clip, ColorMask, ColorMaskParam, ColorParamId, ColorStack, Curve,
    CurveChannel, HueBand, Keyframe, LutInterp, LutSlot, Mask, MaskCombine, MatchTransform,
    PropGroup, PropPath, PropValue, Section, SectionSnapshot, SkinScope, WheelId,
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
    /// The LUT slot (CL-017). `Some` puts the slot's file in place; a clip
    /// that already has a LUT keeps its Intensity, interpolation and skin
    /// protection (CL-017: applying a LUT replaces it and keeps the
    /// intensity), one without takes the slot as given. `None` clears the
    /// slot and removes the LUT section's keys in the same entry.
    SetLut(Option<LutSlot>),
    /// How the LUT is sampled (CL-017's More); nothing without a LUT.
    SetLutInterp(LutInterp),
    /// Skin protection's switch in one scope (CL-030); the LUT scope does
    /// nothing without a LUT.
    SetSkin { scope: SkinScope, on: bool },
    /// The snapshot's sections replace the clip's (CL-025's replace,
    /// CL-026's apply of a saved look): their values and switches, with
    /// the keys of those sections removed in the same entry. Other
    /// sections, Amount and bypass stay.
    ApplySections(Arc<SectionSnapshot>),
    /// The auto correction (CL-019): `Some` stores a run's offsets (a
    /// clip that already has a correction keeps its Strength and its
    /// Strength keys, CL-019's Re-run); `None` removes the correction and
    /// its Strength keys in the same entry.
    SetAuto(Option<AutoCorrection>),
    /// **Apply to sliders** (CL-019): the correction's offsets at its
    /// static Strength are added into the Light and Color sliders, each
    /// clamped into its range (a keyed slider's keys all move by the
    /// offset), and the correction and its Strength keys are removed, as
    /// one entry. Nothing on a clip without a correction.
    ApplyAutoToSliders,
    /// The colour match (CL-020): `Some` stores a transform (a clip that
    /// already has one keeps its Intensity, CL-020's Re-match); `None`
    /// removes it and its Intensity keys in the same entry.
    SetMatch(Option<MatchTransform>),
    // K6
    /// A colour mask added at the end of the list (CL-031), under the
    /// smallest id the clip's stack doesn't use, combined by Add with
    /// Expand 0. Nothing on a stack that already holds
    /// [`crate::model::MAX_COLOR_MASKS`].
    AddMask(Mask),
    /// Mask `id` removed, with its keys, in the same entry; undo puts both
    /// back in place.
    RemoveMask(u16),
    /// Mask `id` moved to position `to` of the list (clamped), the other
    /// masks keeping their order.
    ReorderMask { id: u16, to: usize },
    /// Mask `mask.id` replaced (its shape, values, invert, link, Expand
    /// and mode): a shape switch, a field or a canvas drag. A switch to
    /// another shape removes the keys of the values the new shape lacks
    /// (roundness, band width) in the same entry (TR-017). A keyed value
    /// is edited through its keys, like a keyed slider.
    SetMask(ColorMask),
    /// Mask `id`'s combine mode (TR-020).
    SetMaskMode { id: u16, mode: MaskCombine },
    /// One keyable value of mask `id` (CL-031's `SetColorMaskParam`, one
    /// entry per drag), clamped; nothing on a clip without the mask or a
    /// mask without the value. A keyed value is edited through its keys.
    SetMaskParam {
        id: u16,
        param: ColorMaskParam,
        value: PropValue,
    },
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
    /// The LUT slot: its file or its sampling.
    Lut,
    /// Sections copied from a saved look or another clip.
    Look,
    /// An auto correction stored, applied to the sliders or removed.
    Auto,
    /// A colour match stored or removed.
    Match,
    /// A colour mask added, removed, moved or changed.
    Mask,
}

impl ColorEdit {
    pub fn op(&self) -> ColorOp {
        match self {
            ColorEdit::SetParam(..) | ColorEdit::SetParams(_) => ColorOp::Set,
            ColorEdit::SetSectionEnabled(..) | ColorEdit::SetSkin { .. } => ColorOp::Switch,
            ColorEdit::SetLut(_) | ColorEdit::SetLutInterp(_) => ColorOp::Lut,
            ColorEdit::SetBypass(_) => ColorOp::Bypass,
            ColorEdit::ResetSection(_)
            | ColorEdit::ResetAll
            | ColorEdit::ResetBand(_)
            | ColorEdit::ResetWheel(_) => ColorOp::Reset,
            ColorEdit::SetCurve { .. } => ColorOp::Curve,
            ColorEdit::SetPuck { .. } => ColorOp::Puck,
            ColorEdit::ApplySections(_) => ColorOp::Look,
            ColorEdit::SetAuto(_) | ColorEdit::ApplyAutoToSliders => ColorOp::Auto,
            ColorEdit::SetMatch(_) => ColorOp::Match,
            ColorEdit::AddMask(_)
            | ColorEdit::RemoveMask(_)
            | ColorEdit::ReorderMask { .. }
            | ColorEdit::SetMask(_)
            | ColorEdit::SetMaskMode { .. }
            | ColorEdit::SetMaskParam { .. } => ColorOp::Mask,
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
                    .without(|p| p.group() == Some(PropGroup::Color(section)));
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
            ColorEdit::SetLut(Some(slot)) => {
                let slot = match &stack.lut {
                    Some(old) => LutSlot {
                        lut: slot.lut.clone(),
                        ..old.clone()
                    },
                    None => slot.clone(),
                };
                stack.with_lut(Some(slot))
            }
            ColorEdit::SetLut(None) => {
                out.keys = out
                    .keys
                    .without(|p| p.group() == Some(PropGroup::Color(Section::Lut)));
                stack.with_lut(None)
            }
            ColorEdit::SetLutInterp(interp) => {
                let lut = stack.lut.clone().map(|l| LutSlot {
                    interp: *interp,
                    ..l
                });
                stack.with_lut(lut)
            }
            ColorEdit::SetSkin { scope, on } => stack.with_skin(*scope, *on),
            ColorEdit::ApplySections(snapshot) => {
                let held = snapshot.sections();
                out.keys = out
                    .keys
                    .without(|p| matches!(p.group(), Some(PropGroup::Color(s)) if held.is_on(s)));
                snapshot.applied_to(&stack)
            }
            ColorEdit::SetAuto(Some(auto)) => {
                let strength = stack.auto.map_or(auto.strength, |old| old.strength);
                stack.with_auto(Some(AutoCorrection { strength, ..*auto }))
            }
            ColorEdit::SetAuto(None) => {
                out.keys = out.keys.without(|p| p == strength_key());
                stack.with_auto(None)
            }
            ColorEdit::ApplyAutoToSliders => {
                let Some(auto) = stack.auto else {
                    return Some(out);
                };
                for id in AutoOffsets::PARAMS {
                    let added = auto.added(id);
                    let prop = PropPath::Color(id);
                    let keys = out.keys.keys(prop);
                    if added == 0 || keys.is_empty() {
                        continue;
                    }
                    let moved: Arc<[Keyframe<PropValue>]> = keys
                        .iter()
                        .map(|k| Keyframe {
                            value: match k.value {
                                PropValue::Level(v) => {
                                    PropValue::Level(id.spec().clamp(v.saturating_add(added)))
                                }
                                other => other,
                            },
                            ..*k
                        })
                        .collect();
                    out.keys = out.keys.with_track(prop, moved);
                }
                out.keys = out.keys.without(|p| p == strength_key());
                stack.with_auto_applied()
            }
            ColorEdit::SetMatch(Some(xf)) => {
                let intensity = stack
                    .match_xf
                    .as_ref()
                    .map_or(xf.intensity, |m| m.intensity);
                stack.with_match(Some(xf.with_intensity(intensity)))
            }
            ColorEdit::SetMatch(None) => {
                out.keys = out
                    .keys
                    .without(|p| p == PropPath::Color(ColorParamId::MatchIntensity));
                stack.with_match(None)
            }
            ColorEdit::AddMask(mask) => match stack.with_mask_added(*mask) {
                Some((s, _)) => s,
                None => stack,
            },
            ColorEdit::RemoveMask(id) => {
                let id = *id;
                out.keys = out
                    .keys
                    .without(|p| matches!(p, PropPath::ColorMask { id: k, .. } if k == id));
                stack.without_mask(id)
            }
            ColorEdit::ReorderMask { id, to } => stack.with_mask_moved(*id, *to),
            ColorEdit::SetMask(mask) => {
                let after = stack.with_mask(*mask);
                // TR-017: values the new shape lacks lose their keys.
                if let Some(m) = after.mask(mask.id) {
                    let m = *m;
                    out.keys = out.keys.without(|p| {
                        matches!(p, PropPath::ColorMask { id, param } if id == m.id && m.get(param).is_none())
                    });
                }
                after
            }
            ColorEdit::SetMaskMode { id, mode } => stack.with_mask_mode(*id, *mode),
            ColorEdit::SetMaskParam { id, param, value } => {
                stack.with_mask_param(*id, *param, *value)
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

/// Auto's Strength key path.
fn strength_key() -> PropPath {
    PropPath::Color(ColorParamId::AutoStrength)
}

/// Whether `prop` is a colour control's (a slider or a wheel's puck).
pub fn is_color_prop(prop: PropPath) -> bool {
    matches!(
        prop,
        PropPath::Color(_) | PropPath::ColorPuck(_) | PropPath::ColorMask { .. }
    )
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
    fn choosing_a_lut_keeps_the_settings_of_the_one_it_replaces() {
        use crate::model::{LutRef, SkinProtect};
        let v = video();
        let a = LutSlot::new(LutRef::new([1; 16], "a.cube"));
        let with_a = ColorEdit::SetLut(Some(a.clone())).applied_to(&v).unwrap();
        let tuned = ColorEdit::SetParam(ColorParamId::LutIntensity, 40)
            .applied_to(&with_a)
            .unwrap();
        let tuned = ColorEdit::SetSkin {
            scope: SkinScope::Lut,
            on: true,
        }
        .applied_to(&tuned)
        .unwrap();
        let tuned = ColorEdit::SetLutInterp(LutInterp::Trilinear)
            .applied_to(&tuned)
            .unwrap();
        let b = LutRef::new([2; 16], "b.3dl");
        let with_b = ColorEdit::SetLut(Some(LutSlot::new(b.clone())))
            .applied_to(&tuned)
            .unwrap();
        let slot = with_b.color.unwrap().lut.clone().unwrap();
        assert_eq!(
            slot,
            LutSlot {
                lut: b,
                intensity: 40,
                interp: LutInterp::Trilinear,
                skin: SkinProtect {
                    on: true,
                    amount: 70
                },
            }
        );
        let cleared = ColorEdit::SetLut(None).applied_to(&with_a).unwrap();
        assert_eq!(cleared, v, "no LUT is the default stack again");
        // The LUT's settings do nothing without a LUT.
        assert_eq!(
            ColorEdit::SetLutInterp(LutInterp::Trilinear).applied_to(&v),
            Some(v.clone())
        );
        assert_eq!(ColorEdit::SetLut(None).op(), ColorOp::Lut);
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
