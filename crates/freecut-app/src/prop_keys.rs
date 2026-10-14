//! Keyframes of transform and text properties, seen from the app (TR-027
//! to TR-031, TX-021, ui-concept §13.6): what a row's navigator shows at
//! the playhead, where its arrows jump, which property an inspector edit
//! touches, and the edit behind the navigator's diamond. Pure: the
//! inspector commits the edits as `EditKeys` requests (`ui::props`).
//!
//! Keys are stored in the clip's content time ([`Clip::content_time_at`]);
//! everything here speaks sequence time (the playhead), converted per clip.
//! Keys outside the clip's trimmed range are kept by the model but not
//! reachable from the navigator, as for volume keys.

use freecut_timeline::edit::{ColorEdit, KeyEdit, TextLayoutEdit, TextStyleEdit, TransformEdit};
use freecut_timeline::model::{
    Clip, Fill, Interp, Keyframe, PropGroup, PropPath, PropValue, TextPropPath, TransformProp,
};
use freecut_timeline::time::Tick;

use crate::widgets::keyframe::KeyState;

/// A key of one property as the app shows it: where it sits in sequence
/// time, its value and the curve of the segment it starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShownKey {
    /// Sequence time.
    pub at: Tick,
    pub value: PropValue,
    pub interp: Interp,
}

/// `prop`'s keys inside `clip`'s range, in time order. The range is
/// half-open as the clip's frames are: a key exactly at the trimmed end
/// never plays and can't be edited there, so it is not reachable.
pub fn shown_keys(clip: &Clip, prop: PropPath) -> Vec<ShownKey> {
    clip.keys
        .keys(prop)
        .iter()
        .filter_map(|k| shown(clip, k))
        .collect()
}

fn shown(clip: &Clip, k: &Keyframe<PropValue>) -> Option<ShownKey> {
    let at = clip.sequence_time_of(k.at)?;
    clip.range.contains(at).then_some(ShownKey {
        at,
        value: k.value,
        interp: k.interp,
    })
}

/// What a row's navigator shows for `prop` at the playhead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyNav {
    pub state: KeyState,
    /// The nearest key before / after the playhead (sequence time).
    pub prev: Option<Tick>,
    pub next: Option<Tick>,
    /// Keys in the clip's range.
    pub count: usize,
    /// The playhead is on the clip: a key can be added there.
    pub under_playhead: bool,
}

/// The navigator of `prop` on `clip` with the playhead at `playhead`.
pub fn key_nav(clip: &Clip, prop: PropPath, playhead: Tick) -> KeyNav {
    let times: Vec<Tick> = shown_keys(clip, prop).iter().map(|k| k.at).collect();
    nav_of(&times, clip.keys.is_animated(prop), playhead, clip)
}

/// The navigator over every keyed property of `group` (a card's Previous /
/// Next key and its summary diamond).
pub fn group_nav(clip: &Clip, group: PropGroup, playhead: Tick) -> KeyNav {
    let times = group_times(clip, group);
    let animated = clip.keys.tracks().iter().any(|t| group.contains(t.prop()));
    nav_of(&times, animated, playhead, clip)
}

/// The times of every key of `group`'s properties in the clip's range,
/// sorted, each once.
pub fn group_times(clip: &Clip, group: PropGroup) -> Vec<Tick> {
    let mut times: Vec<Tick> = clip
        .keys
        .tracks()
        .iter()
        .filter(|t| group.contains(t.prop()))
        .flat_map(|t| t.keys().iter().filter_map(|k| shown(clip, k)))
        .map(|k| k.at)
        .collect();
    times.sort_unstable();
    times.dedup();
    times
}

fn nav_of(times: &[Tick], animated: bool, playhead: Tick, clip: &Clip) -> KeyNav {
    let prev = times.iter().rev().find(|t| **t < playhead).copied();
    let next = times.iter().find(|t| **t > playhead).copied();
    let state = if animated {
        KeyState::Animated {
            on_key: times.contains(&playhead),
            prev: prev.is_some(),
            next: next.is_some(),
        }
    } else {
        KeyState::NotAnimated
    };
    KeyNav {
        state,
        prev,
        next,
        count: times.len(),
        under_playhead: clip.range.contains(playhead),
    }
}

/// Why the diamond can't add a key now, if it can't (TR-028: keys are set
/// at the playhead, on the clip).
pub fn add_blocked(nav: &KeyNav) -> Option<&'static str> {
    let on_key = matches!(nav.state, KeyState::Animated { on_key: true, .. });
    (!nav.under_playhead && !on_key).then_some("Move the playhead onto this clip to add a keyframe")
}

/// The property a text style edit sets and the value it sets it to, when
/// the property takes keys (TX-021); `None` for every other edit.
pub fn style_prop(edit: &TextStyleEdit) -> Option<(PropPath, PropValue)> {
    use PropValue as V;
    use TextPropPath as P;
    use TextStyleEdit as E;
    let text = |p: P, v: PropValue| Some((PropPath::Text(p), v));
    match *edit {
        E::Size(v) => text(P::Size, V::Length(v)),
        E::Fill(Fill::Solid(c)) => text(P::FillColor, V::Color(c)),
        E::FillStopColor { index, color } => {
            Some((PropPath::FillStopColor(index), V::Color(color)))
        }
        E::FillAngle(a) => text(P::FillAngle, V::Direction(a)),
        E::LetterSpacing(v) => text(P::LetterSpacing, V::SignedRatio(v)),
        E::OutlineColor(c) => text(P::OutlineColor, V::Color(c)),
        E::OutlineWidth(v) => text(P::OutlineWidth, V::Ratio(v)),
        E::OutlineOpacity(v) => text(P::OutlineOpacity, V::Ratio(v)),
        E::GlowColor(c) => text(P::GlowColor, V::Color(c)),
        E::GlowIntensity(v) => text(P::GlowIntensity, V::Ratio(v)),
        E::GlowRadius(v) => text(P::GlowRadius, V::Ratio(v)),
        E::GlowOffset(v) => text(P::GlowOffset, V::Ratio(v)),
        E::ShadowColor(c) => text(P::ShadowColor, V::Color(c)),
        E::ShadowOpacity(v) => text(P::ShadowOpacity, V::Ratio(v)),
        E::ShadowBlur(v) => text(P::ShadowBlur, V::Ratio(v)),
        E::ShadowDistance(v) => text(P::ShadowDistance, V::Ratio(v)),
        E::ShadowAngle(a) => text(P::ShadowAngle, V::Direction(a)),
        E::BackgroundColor(c) => text(P::BackgroundColor, V::Color(c)),
        E::BackgroundOpacity(v) => text(P::BackgroundOpacity, V::Ratio(v)),
        E::BackgroundRoundness(v) => text(P::BackgroundRoundness, V::Ratio(v)),
        E::BackgroundBorderWidth(v) => text(P::BackgroundBorderWidth, V::Ratio(v)),
        E::BubbleTailTip(t) => text(P::BubbleTailTip, V::TailTip(t)),
        E::BubbleTailBase(v) => text(P::BubbleTailBase, V::Ratio(v)),
        E::BubbleTailBend(v) => text(P::BubbleTailBend, V::SignedRatio(v)),
        E::Curve(v) => text(P::CurveAmount, V::SignedRatio(v)),
        _ => None,
    }
}

/// As [`style_prop`], for the paragraph's line spacing (TX-009).
pub fn layout_prop(edit: &TextLayoutEdit) -> Option<(PropPath, PropValue)> {
    match *edit {
        TextLayoutEdit::LineSpacing(v) => Some((
            PropPath::Text(TextPropPath::LineSpacing),
            PropValue::Ratio(v),
        )),
        _ => None,
    }
}

/// As [`style_prop`], for the transform (TR-027).
pub fn transform_prop(edit: &TransformEdit) -> Option<(PropPath, PropValue)> {
    let t = |p: TransformProp, v: PropValue| Some((PropPath::Transform(p), v));
    match *edit {
        TransformEdit::Position(p) => t(TransformProp::Position, PropValue::Position(p)),
        TransformEdit::Scale(s) => t(TransformProp::Scale, PropValue::Scale(s)),
        TransformEdit::Rotation(a) => t(TransformProp::Rotation, PropValue::Angle(a)),
        TransformEdit::Opacity(o) => t(TransformProp::Opacity, PropValue::Ratio(o)),
        _ => None,
    }
}

/// The colour slider a Look-lens edit sets, with its value (CL-028).
pub fn color_prop(edit: &ColorEdit) -> Option<(PropPath, PropValue)> {
    match edit {
        ColorEdit::SetParam(id, v) => Some((PropPath::Color(*id), PropValue::Level(*v))),
        ColorEdit::SetPuck { wheel, x, y } => Some((
            PropPath::ColorPuck(*wheel),
            PropValue::Puck { x: *x, y: *y },
        )),
        ColorEdit::SetMaskParam { id, param, value } => Some((
            PropPath::ColorMask {
                id: *id,
                param: *param,
            },
            *value,
        )),
        _ => None,
    }
}

/// The name a property's row has, for the Keyframes card and tooltips.
pub fn prop_label(prop: PropPath) -> String {
    use TextPropPath as P;
    let name = match prop {
        PropPath::Transform(TransformProp::Position) => "Position",
        PropPath::Transform(TransformProp::Scale) => "Scale",
        PropPath::Transform(TransformProp::Rotation) => "Rotation",
        PropPath::Transform(TransformProp::Opacity) => "Opacity",
        PropPath::FillStopColor(i) => return format!("Stop {} colour", u16::from(i) + 1),
        PropPath::Reveal => "Reveal",
        PropPath::Color(id) => crate::ui::look::param_label(id),
        PropPath::ColorPuck(w) => crate::ui::look::puck_label(w),
        PropPath::ColorMask { id, param } => {
            return format!(
                "Mask {id} {}",
                crate::ui::look::mask_param_label(param).to_lowercase()
            );
        }
        PropPath::Text(p) => match p {
            P::Size => "Size",
            P::FillColor => "Fill colour",
            P::FillAngle => "Gradient angle",
            P::LetterSpacing => "Letter spacing",
            P::LineSpacing => "Line spacing",
            P::OutlineColor => "Outline colour",
            P::OutlineWidth => "Outline width",
            P::OutlineOpacity => "Outline opacity",
            P::GlowColor => "Glow colour",
            P::GlowIntensity => "Glow intensity",
            P::GlowRadius => "Glow radius",
            P::GlowOffset => "Glow offset",
            P::ShadowColor => "Shadow colour",
            P::ShadowOpacity => "Shadow opacity",
            P::ShadowBlur => "Shadow blur",
            P::ShadowDistance => "Shadow distance",
            P::ShadowAngle => "Shadow angle",
            P::BackgroundColor => "Background colour",
            P::BackgroundOpacity => "Background opacity",
            P::BackgroundRoundness => "Background roundness",
            P::BackgroundBorderWidth => "Border width",
            P::BubbleTailTip => "Tail tip",
            P::BubbleTailBase => "Tail width",
            P::BubbleTailBend => "Tail bend",
            P::CurveAmount => "Curve",
            other => other.as_str(),
        },
    };
    name.to_owned()
}

/// A key's value in a few words ("64 px", "#FF0000", "40 %").
pub fn value_label(value: PropValue) -> String {
    match value {
        // Millionths of the canvas: shown as percentages of it.
        PropValue::Position(p) => format!(
            "{:.1} % · {:.1} %",
            f64::from(p.x) / 10_000.0,
            f64::from(p.y) / 10_000.0
        ),
        PropValue::Scale(s) => format!("{:.0} %", s.factor() * 100.0),
        PropValue::Angle(a) => format!("{:.1}°", a.degrees()),
        PropValue::Ratio(r) => format!("{:.1} %", r.as_f64() * 100.0),
        PropValue::Length(l) => format!("{:.1} px", l.as_f64()),
        PropValue::SignedRatio(r) => format!("{:.1}", f64::from(r.get()) / 10.0),
        PropValue::Direction(d) => format!("{:.1}°", d.degrees()),
        PropValue::Color(c) => crate::ui::rows::hex(c),
        PropValue::TailTip(t) => format!("{:.0} · {:.0}", t.x.as_f64(), t.y.as_f64()),
        PropValue::Level(v) => format!("{v:+}"),
        // On the disk of radius 1000: shown as percentages of the rim.
        PropValue::Puck { x, y } => format!(
            "{:+.0} % · {:+.0} %",
            f64::from(x) / 10.0,
            f64::from(y) / 10.0
        ),
        // In shorter sides of the item: shown as percentages of it.
        PropValue::MaskPoint(p) => format!(
            "{:.1} % · {:.1} %",
            f64::from(p.x) / 100.0,
            f64::from(p.y) / 100.0
        ),
    }
}

/// A TR-031 easing's menu name.
pub fn interp_label(interp: Interp) -> &'static str {
    use freecut_timeline::model::{EaseCurve as C, EaseDir as D};
    match interp {
        Interp::Linear => "Linear",
        Interp::Hold => "Hold",
        Interp::Ease { curve, dir } => match (curve, dir) {
            (C::Quad, D::In) => "Ease in",
            (C::Quad, D::Out) => "Ease out",
            (C::Quad, D::InOut) => "Ease in-out",
            (C::Sine, D::In) => "Gentle in",
            (C::Sine, D::Out) => "Gentle out",
            (C::Sine, D::InOut) => "Gentle in-out",
            (C::Cubic, D::In) => "Strong in",
            (C::Cubic, D::Out) => "Strong out",
            (C::Cubic, D::InOut) => "Strong in-out",
            (C::Expo, D::In) => "Very strong in",
            (C::Expo, D::Out) => "Very strong out",
            (C::Expo, D::InOut) => "Very strong in-out",
        },
        Interp::OvershootOut => "Overshoot",
        Interp::OvershootIn => "Anticipate",
        Interp::ElasticOut => "Elastic",
        Interp::BounceOut => "Bounce",
    }
}

// ------------------------------------------------------------ edits

/// The navigator's diamond (ui-concept §13.6): removes the key at the
/// playhead, or adds one there holding the value shown now.
pub fn toggle_edit(nav: &KeyNav, playhead: Tick) -> KeyEdit {
    match nav.state {
        KeyState::Animated { on_key: true, .. } => KeyEdit::Remove { at: playhead },
        _ => KeyEdit::Add {
            at: playhead,
            value: None,
        },
    }
}

/// Whether `group` has a key at the playhead on `clip` (enables **Remove
/// this card's keys here**).
pub fn group_key_at(clip: &Clip, group: PropGroup, playhead: Tick) -> bool {
    group_times(clip, group).contains(&playhead)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use freecut_timeline::edit::{TextLayoutEdit, TextStyleEdit, TransformEdit};
    use freecut_timeline::ids::ClipId;
    use freecut_timeline::model::{
        CanvasPos, DeciDegrees, PerMille, ProjectSettings, RefPx, Rgba8, Scale, SignedPerMille,
        TailTip, TextCard,
    };

    use super::*;
    use crate::session::Project;
    use crate::ui::drag::{DragGesture, Held};
    use crate::ui::inspector::lens::{CardId, SelectionKind};
    use crate::ui::props::{Edit, Merge, Steps, commit};
    use crate::ui::rows::{At, KeyView};
    use crate::widgets::keyframe::KeyNavOutput;

    fn secs(s: i64) -> Tick {
        Tick::from_secs(s).unwrap()
    }

    fn frames(n: i64) -> Tick {
        // 30 fps projects: whole frames.
        Tick::from_raw(Tick::from_secs(1).unwrap().raw() / 30 * n)
    }

    fn project_with_text() -> (Project, ClipId) {
        let mut project = Project::new(ProjectSettings::new_default().unwrap()).unwrap();
        let clip = crate::ui::text::add_text_at_playhead(&mut project).unwrap();
        (project, clip)
    }

    fn clip(project: &Project, id: ClipId) -> std::sync::Arc<Clip> {
        std::sync::Arc::clone(project.doc().clip(id).unwrap())
    }

    const SIZE: PropPath = PropPath::Text(TextPropPath::Size);

    /// Commits `edits` as the inspector's cards do, each its own entry.
    fn apply(project: &mut Project, clips: &[ClipId], edits: Vec<Edit>) -> Option<String> {
        let mut steps = Steps::default();
        for e in edits {
            steps.edit(e, Merge::Own);
        }
        commit(project, &mut DragGesture::default(), clips, steps, Held::Up)
    }

    /// Clicks a row navigator of `prop` on `id` at the playhead, as a row
    /// does; returns where an arrow asked the playhead to go.
    fn click(project: &mut Project, id: ClipId, prop: PropPath, out: KeyNavOutput) -> Option<Tick> {
        let view = KeyView::new(clip(project, id), project.playhead);
        let at = At::new(SelectionKind::Text, CardId::Type).with_keys(Some(&view));
        let mut steps = Steps::default();
        at.keys_clicked(prop, out, &mut steps);
        let failed = commit(project, &mut DragGesture::default(), &[id], steps, Held::Up);
        assert_eq!(failed, None);
        view.seek.get()
    }

    const TOGGLE: KeyNavOutput = KeyNavOutput {
        toggle: true,
        prev: false,
        next: false,
    };

    #[test]
    fn every_mapped_edit_names_an_animatable_property_of_its_value_kind() {
        let styles = [
            TextStyleEdit::Size(RefPx::from_px(50)),
            TextStyleEdit::Fill(Fill::Solid(Rgba8::WHITE)),
            TextStyleEdit::FillStopColor {
                index: 1,
                color: Rgba8::WHITE,
            },
            TextStyleEdit::FillAngle(DeciDegrees::DOWN),
            TextStyleEdit::LetterSpacing(SignedPerMille::new(10)),
            TextStyleEdit::OutlineColor(Rgba8::WHITE),
            TextStyleEdit::OutlineWidth(PerMille::new(10)),
            TextStyleEdit::OutlineOpacity(PerMille::new(10)),
            TextStyleEdit::GlowColor(Rgba8::WHITE),
            TextStyleEdit::GlowIntensity(PerMille::new(10)),
            TextStyleEdit::GlowRadius(PerMille::new(10)),
            TextStyleEdit::GlowOffset(PerMille::new(10)),
            TextStyleEdit::ShadowColor(Rgba8::WHITE),
            TextStyleEdit::ShadowOpacity(PerMille::new(10)),
            TextStyleEdit::ShadowBlur(PerMille::new(10)),
            TextStyleEdit::ShadowDistance(PerMille::new(10)),
            TextStyleEdit::ShadowAngle(DeciDegrees::DOWN),
            TextStyleEdit::BackgroundColor(Rgba8::WHITE),
            TextStyleEdit::BackgroundOpacity(PerMille::new(10)),
            TextStyleEdit::BackgroundRoundness(PerMille::new(10)),
            TextStyleEdit::BackgroundBorderWidth(PerMille::new(10)),
            TextStyleEdit::BubbleTailTip(TailTip {
                x: RefPx::from_px(1),
                y: RefPx::from_px(2),
            }),
            TextStyleEdit::BubbleTailBase(PerMille::new(200)),
            TextStyleEdit::BubbleTailBend(SignedPerMille::new(10)),
            TextStyleEdit::Curve(SignedPerMille::new(10)),
        ];
        let mut props = Vec::new();
        for e in &styles {
            let (prop, value) = style_prop(e).unwrap_or_else(|| panic!("{e:?}"));
            assert_eq!(prop.kind(), Some(value.kind()), "{e:?}");
            props.push(prop);
        }
        let (prop, value) = layout_prop(&TextLayoutEdit::LineSpacing(PerMille::ONE)).unwrap();
        assert_eq!(prop.kind(), Some(value.kind()));
        props.push(prop);
        // Every animatable text property is reachable from some row.
        for p in TextPropPath::ALL.iter().filter(|p| p.is_animatable()) {
            assert!(props.contains(&PropPath::Text(*p)), "{p:?} has no row");
        }
        for e in [
            TransformEdit::Position(CanvasPos::new(1, 2)),
            TransformEdit::Scale(Scale::from_basis_points(5000)),
            TransformEdit::Rotation(freecut_timeline::model::Angle::from_hundredths(100)),
            TransformEdit::Opacity(PerMille::new(10)),
        ] {
            let (prop, value) = transform_prop(&e).unwrap();
            assert_eq!(prop.kind(), Some(value.kind()));
        }
        assert_eq!(style_prop(&TextStyleEdit::Bold(true)), None);
        assert_eq!(transform_prop(&TransformEdit::FlipH(true)), None);
        assert_eq!(layout_prop(&TextLayoutEdit::Justify(true)), None);
        for p in PropPath::all_fixed() {
            assert!(!prop_label(p).starts_with("text."), "{p:?}");
        }
        for i in Interp::PRESETS {
            assert!(!interp_label(i).is_empty());
        }
    }

    /// ui-concept §13.6: the navigator's state, arrows and diamond as the
    /// playhead moves over a property with keys, through the rows' path.
    #[test]
    fn the_navigator_follows_the_playhead_over_the_keys() {
        let (mut project, id) = project_with_text();
        let c = clip(&project, id);
        let nav = key_nav(&c, SIZE, frames(10));
        assert_eq!(nav.state, KeyState::NotAnimated);
        assert_eq!((nav.prev, nav.next, nav.count), (None, None, 0));
        assert!(nav.under_playhead);
        assert_eq!(add_blocked(&nav), None);
        assert_eq!(
            toggle_edit(&nav, frames(10)),
            KeyEdit::Add {
                at: frames(10),
                value: None
            }
        );

        let entries = project.editor.history().len();
        for f in [10, 40] {
            project.playhead = frames(f);
            assert_eq!(click(&mut project, id, SIZE, TOGGLE), None);
        }
        assert_eq!(
            project.editor.history().len(),
            entries + 2,
            "one entry each"
        );
        let c = clip(&project, id);
        let on = key_nav(&c, SIZE, frames(10));
        assert_eq!(
            on.state,
            KeyState::Animated {
                on_key: true,
                prev: false,
                next: true
            }
        );
        assert_eq!((on.prev, on.next, on.count), (None, Some(frames(40)), 2));
        assert_eq!(
            toggle_edit(&on, frames(10)),
            KeyEdit::Remove { at: frames(10) }
        );
        let between = key_nav(&c, SIZE, frames(20));
        assert_eq!(
            between.state,
            KeyState::Animated {
                on_key: false,
                prev: true,
                next: true
            }
        );
        assert_eq!(
            (between.prev, between.next),
            (Some(frames(10)), Some(frames(40)))
        );
        // The arrows move the playhead; they edit nothing.
        project.playhead = frames(20);
        let prev = KeyNavOutput {
            prev: true,
            ..KeyNavOutput::default()
        };
        let next = KeyNavOutput {
            next: true,
            ..KeyNavOutput::default()
        };
        assert_eq!(click(&mut project, id, SIZE, prev), Some(frames(10)));
        assert_eq!(click(&mut project, id, SIZE, next), Some(frames(40)));
        assert_eq!(project.editor.history().len(), entries + 2);
        // The diamond on a key removes it; undo brings it back.
        project.playhead = frames(40);
        click(&mut project, id, SIZE, TOGGLE);
        assert_eq!(shown_keys(&clip(&project, id), SIZE).len(), 1);
        project.undo().unwrap();
        assert_eq!(shown_keys(&clip(&project, id), SIZE).len(), 2);
        project.redo().unwrap();
        project.undo().unwrap();

        let c = clip(&project, id);
        let after = key_nav(&c, SIZE, frames(50));
        assert_eq!((after.prev, after.next), (Some(frames(40)), None));
        // Past the clip's end: arrows still work, adding doesn't.
        let past = key_nav(&c, SIZE, c.end().checked_add(secs(1)).unwrap());
        assert!(!past.under_playhead);
        assert!(add_blocked(&past).is_some());
        assert_eq!(past.prev, Some(frames(40)));
    }

    /// TR-027: once a property has keys, editing it in its row sets the key
    /// at the playhead (adding one there), and leaves the static value.
    #[test]
    fn editing_a_keyed_property_keys_it_at_the_playhead() {
        let (mut project, id) = project_with_text();
        let size = |v: i32| Edit::Style(TextStyleEdit::Size(RefPx::from_px(v)));
        // Without keys: the static value.
        assert_eq!(apply(&mut project, &[id], vec![size(70)]), None);
        assert!(clip(&project, id).keys.is_empty());
        project.playhead = frames(10);
        click(&mut project, id, SIZE, TOGGLE);
        let entries = project.editor.history().len();
        // On the key: its value changes.
        assert_eq!(apply(&mut project, &[id], vec![size(90)]), None);
        // Elsewhere: a new key.
        project.playhead = frames(50);
        assert_eq!(apply(&mut project, &[id], vec![size(120)]), None);
        assert_eq!(project.editor.history().len(), entries + 2);
        let keys = shown_keys(&clip(&project, id), SIZE);
        let values: Vec<(Tick, PropValue)> = keys.iter().map(|k| (k.at, k.value)).collect();
        assert_eq!(
            values,
            [
                (frames(10), PropValue::Length(RefPx::from_px(90))),
                (frames(50), PropValue::Length(RefPx::from_px(120))),
            ]
        );
        let c = clip(&project, id);
        assert_eq!(
            c.content.text().unwrap().style.size,
            RefPx::from_px(70),
            "the static value stays"
        );
        // The shown value follows the keys.
        let shown = crate::ui::props::shown_text(&c, frames(30)).unwrap();
        assert!(shown.style.size > RefPx::from_px(90) && shown.style.size < RefPx::from_px(120));
        project.undo().unwrap();
        assert_eq!(shown_keys(&clip(&project, id), SIZE).len(), 1);
    }

    /// TR-028 / TX-021 through the app path: Key this card, Remove this
    /// card's keys here, the card's Previous / Next, and easing and moving
    /// keys, each one undo entry.
    #[test]
    fn card_commands_and_key_edits_are_one_entry_each() {
        let (mut project, id) = project_with_text();
        let card = PropGroup::Text(TextCard::Type);
        let before = project.doc().clone();
        let key_card = |at| Edit::Keys {
            prop: PropPath::Reveal,
            edit: KeyEdit::KeyCard { card, at },
        };
        for f in [5, 25] {
            project.playhead = frames(f);
            assert_eq!(apply(&mut project, &[id], vec![key_card(frames(f))]), None);
        }
        let c = clip(&project, id);
        assert_eq!(group_times(&c, card), [frames(5), frames(25)]);
        assert!(group_key_at(&c, card, frames(25)));
        let nav = group_nav(&c, card, frames(10));
        assert_eq!((nav.prev, nav.next), (Some(frames(5)), Some(frames(25))));
        let clear = Edit::Keys {
            prop: PropPath::Reveal,
            edit: KeyEdit::ClearCard {
                card,
                at: Some(frames(25)),
            },
        };
        assert_eq!(apply(&mut project, &[id], vec![clear]), None);
        let c = clip(&project, id);
        assert_eq!(group_times(&c, card), [frames(5)]);
        for _ in 0..3 {
            project.undo().unwrap();
        }
        assert_eq!(project.doc(), &before, "three entries");

        // Easing and moving a key.
        for (f, px) in [(0, 40), (30, 80)] {
            let add = Edit::Keys {
                prop: SIZE,
                edit: KeyEdit::Add {
                    at: frames(f),
                    value: Some(PropValue::Length(RefPx::from_px(px))),
                },
            };
            assert_eq!(apply(&mut project, &[id], vec![add]), None);
        }
        let ease = Interp::PRESETS[2];
        let eased = Edit::Keys {
            prop: SIZE,
            edit: KeyEdit::SetInterp {
                at: frames(0),
                interp: ease,
            },
        };
        let moved = Edit::Keys {
            prop: SIZE,
            edit: KeyEdit::Move {
                from: frames(30),
                to: frames(45),
            },
        };
        assert_eq!(apply(&mut project, &[id], vec![eased, moved]), None);
        let keys = shown_keys(&clip(&project, id), SIZE);
        assert_eq!(keys[0].interp, ease);
        assert_eq!(keys[1].at, frames(45));
        assert_eq!(value_label(keys[1].value), "80.0 px");
        project.undo().unwrap();
        assert_eq!(shown_keys(&clip(&project, id), SIZE)[1].at, frames(30));

        // Off the clip: refused with a sentence, nothing recorded.
        let entries = project.editor.history().len();
        project.playhead = secs(60);
        let add = Edit::Keys {
            prop: SIZE,
            edit: KeyEdit::Add {
                at: secs(60),
                value: None,
            },
        };
        let err = apply(&mut project, &[id], vec![add]).unwrap();
        assert!(err.contains("playhead"), "{err}");
        assert_eq!(project.editor.history().len(), entries);
    }

    #[test]
    fn reset_placement_removes_position_scale_and_rotation_keys_in_one_entry() {
        let (mut project, id) = project_with_text();
        let position = PropPath::Transform(TransformProp::Position);
        let opacity = PropPath::Transform(TransformProp::Opacity);
        let away = CanvasPos::from_px(300.0, 0.0, 1920, 1080);
        let add = |prop, at, value| Edit::Keys {
            prop,
            edit: KeyEdit::Add {
                at,
                value: Some(value),
            },
        };
        let edits = vec![
            add(position, frames(0), PropValue::Position(away)),
            add(position, frames(30), PropValue::Position(away)),
            add(opacity, frames(0), PropValue::Ratio(PerMille::new(500))),
        ];
        assert_eq!(apply(&mut project, &[id], edits), None);
        project.playhead = frames(15);
        let before = project.doc().clone();
        let entries = project.editor.history().len();

        let toast =
            crate::ui::props::reset_placement(&mut project, &mut DragGesture::default(), &[id])
                .unwrap();
        assert!(
            toast.contains("2 keyframes") && toast.contains("Position 2"),
            "{toast}"
        );
        let c = clip(&project, id);
        assert!(shown_keys(&c, position).is_empty(), "position keys removed");
        assert_eq!(shown_keys(&c, opacity).len(), 1, "opacity keys stay");
        let shown = crate::ui::props::shown_transform(&c, project.playhead);
        assert_eq!(shown.position, CanvasPos::CENTER, "the reset shows");
        assert_eq!(project.editor.history().len(), entries + 1, "one entry");
        project.undo().unwrap();
        assert_eq!(project.doc(), &before);
    }
}
