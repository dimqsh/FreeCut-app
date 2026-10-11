//! Text round 2 model (TX-008 to TX-016, TX-054, TX-055): every style and
//! layout edit, optional layers and the Background/Bubble exclusivity, card
//! and lens reset, and **Change case**. Every edit goes through
//! `Fixture::apply`, which checks plan purity, validity and the
//! apply/undo/redo round trip.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use std::sync::Arc;

use common::*;
use freecut_timeline::Committed;
use freecut_timeline::edit::{
    EditError, EditRequest, InsertReq, InsertTarget, OpKind, ResetScope, TextLayoutEdit,
    TextStyleEdit, TransformEdit, change_case,
};
use freecut_timeline::ids::ClipId;
use freecut_timeline::model::{
    BoxHeight, BoxShape, BubbleKind, CaseTransform, DeciDegrees, Fill, FillCenter, FillExtent,
    FillKind, GlowDirection, GradientStop, GradientStops, HAlign, LangTag, PerMille, RefPx, Rgba8,
    Scale, Side, SignedPerMille, StrokePosition, TailTip, TextCard, TextClipData, TextLayerKind,
    TextLayout, TextLens, TextPropPath, TextStyle, TrackFlags, TrackHeader, Transform, VAnchor,
};
use pretty_assertions::assert_eq;

use TextPropPath as P;

fn insert_text(fx: &mut Fixture, at: i64) -> ClipId {
    let req = EditRequest::Insert(InsertReq {
        seq: fx.seq_id(),
        target: InsertTarget::FirstFree,
        at: f(at),
        clips: vec![freecut_timeline::edit::NewClip::text(
            TextClipData::default(),
            f(10),
        )],
    });
    match fx.apply(req).unwrap() {
        Committed::Applied { selection, .. } => selection.unwrap()[0],
        Committed::NoOp => panic!("insert was a no-op"),
    }
}

fn text_of(fx: &Fixture, clip: ClipId) -> Arc<TextClipData> {
    Arc::clone(fx.doc().clip(clip).unwrap().content.text().unwrap())
}

fn style(fx: &mut Fixture, clips: &[ClipId], edit: TextStyleEdit) -> Result<Committed, EditError> {
    fx.apply(EditRequest::SetTextStyle {
        seq: fx.seq_id(),
        clips: clips.to_vec(),
        edit,
    })
}

fn layout(
    fx: &mut Fixture,
    clips: &[ClipId],
    edit: TextLayoutEdit,
) -> Result<Committed, EditError> {
    fx.apply(EditRequest::SetTextLayout {
        seq: fx.seq_id(),
        clips: clips.to_vec(),
        edit,
    })
}

fn reset(fx: &mut Fixture, clips: &[ClipId], scope: ResetScope) -> Result<Committed, EditError> {
    fx.apply(EditRequest::ResetText {
        seq: fx.seq_id(),
        clips: clips.to_vec(),
        scope,
    })
}

/// `after` differs from `before` exactly in `paths`: copying those paths
/// back from `before` gives `before`, and each of them changed.
fn assert_changed_exactly(before: &TextClipData, after: &TextClipData, paths: &[TextPropPath]) {
    let mut back = after.clone();
    for p in paths {
        p.copy(before, &mut back);
    }
    assert_eq!(&back, before, "changed more than {paths:?}");
    for p in paths {
        let mut only = before.clone();
        p.copy(after, &mut only);
        assert_ne!(&only, before, "{p} did not change");
    }
}

fn red() -> Rgba8 {
    Rgba8::new(255, 0, 0, 255)
}

fn three_stops() -> GradientStops {
    GradientStops::new(vec![
        GradientStop {
            at: PerMille::ZERO,
            color: red(),
        },
        GradientStop {
            at: PerMille::new(400),
            color: Rgba8::new(0, 255, 0, 200),
        },
        GradientStop {
            at: PerMille::ONE,
            color: Rgba8::new(0, 0, 255, 255),
        },
    ])
    .unwrap()
}

/// Every style edit of round 2 with a value off the default, in an order
/// where each one changes something, and the paths it changes.
fn style_edits() -> Vec<(TextStyleEdit, Vec<TextPropPath>)> {
    use TextStyleEdit as E;
    let pm = PerMille::new;
    vec![
        (E::Underline(true), vec![P::Underline]),
        (E::Strike(true), vec![P::Strike]),
        (E::Case(CaseTransform::Title), vec![P::Case]),
        (
            E::LetterSpacing(SignedPerMille::new(-150)),
            vec![P::LetterSpacing],
        ),
        (E::FillKind(FillKind::LinearGradient), vec![P::FillKind]),
        (
            E::FillAngle(DeciDegrees::from_degrees(30)),
            vec![P::FillAngle],
        ),
        (E::FillStops(three_stops()), vec![P::FillStops]),
        (
            E::FillStopColor {
                index: 1,
                color: Rgba8::BLACK,
            },
            vec![P::FillStops],
        ),
        (E::FillExtent(FillExtent::Line), vec![P::FillExtent]),
        (E::FillKind(FillKind::RadialGradient), vec![P::FillKind]),
        (
            E::FillCenter(FillCenter {
                x: pm(100),
                y: pm(900),
            }),
            vec![P::FillCenter],
        ),
        (E::FillSpread(pm(1500)), vec![P::FillSpread]),
        (E::Fill(Fill::Solid(red())), vec![P::FillKind]),
        (
            E::SetLayer {
                layer: TextLayerKind::Outline,
                on: true,
            },
            vec![P::OutlineOn],
        ),
        (E::OutlineColor(red()), vec![P::OutlineColor]),
        (E::OutlineWidth(pm(120)), vec![P::OutlineWidth]),
        (E::OutlineOpacity(pm(400)), vec![P::OutlineOpacity]),
        (
            E::OutlinePosition(StrokePosition::Inside),
            vec![P::OutlinePosition],
        ),
        (
            E::SetLayer {
                layer: TextLayerKind::Glow,
                on: true,
            },
            vec![P::GlowOn],
        ),
        (E::GlowColor(red()), vec![P::GlowColor]),
        (E::GlowIntensity(pm(900)), vec![P::GlowIntensity]),
        (E::GlowRadius(pm(500)), vec![P::GlowRadius]),
        (E::GlowOffset(pm(100)), vec![P::GlowOffset]),
        (
            E::GlowAngle(DeciDegrees::from_degrees(200)),
            vec![P::GlowAngle],
        ),
        (
            E::GlowDirection(GlowDirection::Both),
            vec![P::GlowDirection],
        ),
        (
            E::SetLayer {
                layer: TextLayerKind::Shadow,
                on: true,
            },
            vec![P::ShadowOn],
        ),
        (E::ShadowColor(red()), vec![P::ShadowColor]),
        (E::ShadowOpacity(pm(800)), vec![P::ShadowOpacity]),
        (E::ShadowBlur(pm(300)), vec![P::ShadowBlur]),
        (E::ShadowDistance(pm(600)), vec![P::ShadowDistance]),
        (
            E::ShadowAngle(DeciDegrees::from_degrees(135)),
            vec![P::ShadowAngle],
        ),
        (E::ShadowIncludeBox(true), vec![P::ShadowIncludeBox]),
        (
            E::SetLayer {
                layer: TextLayerKind::Background,
                on: true,
            },
            vec![P::BackgroundOn],
        ),
        (E::BackgroundColor(red()), vec![P::BackgroundColor]),
        (E::BackgroundOpacity(pm(300)), vec![P::BackgroundOpacity]),
        (
            E::BackgroundShape(BoxShape::PerChar),
            vec![P::BackgroundShape],
        ),
        (E::BackgroundPadX(pm(10)), vec![P::BackgroundPadX]),
        (E::BackgroundPadY(pm(20)), vec![P::BackgroundPadY]),
        (
            E::BackgroundRoundness(pm(1000)),
            vec![P::BackgroundRoundness],
        ),
        (
            E::BackgroundHeight(BoxHeight::LowerHalf),
            vec![P::BackgroundHeight],
        ),
        (E::BackgroundBorder(true), vec![P::BackgroundBorderOn]),
        (
            E::BackgroundBorderColor(red()),
            vec![P::BackgroundBorderColor],
        ),
        (
            E::BackgroundBorderWidth(pm(150)),
            vec![P::BackgroundBorderWidth],
        ),
        // Bubble on switches Background off in the same edit, and the fill
        // is red on white: readable, so the bubble keeps its white.
        (
            E::SetLayer {
                layer: TextLayerKind::Bubble,
                on: true,
            },
            vec![P::BubbleOn, P::BackgroundOn],
        ),
        (
            E::BubbleKind(BubbleKind::ArrowTag { point: Side::Left }),
            vec![P::BubbleKind, P::BubbleTailOn],
        ),
        (E::BubbleTail(true), vec![P::BubbleTailOn]),
        (E::BubbleColor(Rgba8::BLACK), vec![P::BubbleColor]),
        (E::BubbleOpacity(pm(500)), vec![P::BubbleOpacity]),
        (
            E::BubbleTailTip(TailTip {
                x: RefPx::from_px(300),
                y: RefPx::from_px(-50),
            }),
            vec![P::BubbleTailTip],
        ),
        (E::BubbleTailBase(pm(500)), vec![P::BubbleTailBase]),
        (
            E::BubbleTailBend(SignedPerMille::new(-700)),
            vec![P::BubbleTailBend],
        ),
        (E::BubblePadX(pm(100)), vec![P::BubblePadX]),
        (E::BubblePadY(pm(100)), vec![P::BubblePadY]),
        (E::BubbleRoundness(pm(0)), vec![P::BubbleRoundness]),
        (E::BubbleBorder(false), vec![P::BubbleBorderOn]),
        (E::BubbleBorderColor(red()), vec![P::BubbleBorderColor]),
        (E::BubbleBorderWidth(pm(10)), vec![P::BubbleBorderWidth]),
        (E::BubbleSpikes(20), vec![P::BubbleSpikes]),
        (E::BubbleSpikeDepth(pm(300)), vec![P::BubbleSpikeDepth]),
        (E::BubbleJitter(pm(0)), vec![P::BubbleJitter]),
        (E::BubbleSeed(42), vec![P::BubbleSeed]),
        (E::BubbleLobe(pm(350)), vec![P::BubbleLobe]),
        (E::BubbleTrail(4), vec![P::BubbleTrail]),
        (
            E::SetLayer {
                layer: TextLayerKind::Curve,
                on: true,
            },
            vec![P::CurveOn],
        ),
        (E::Curve(SignedPerMille::new(-1000)), vec![P::CurveAmount]),
    ]
}

#[test]
fn every_style_edit_sets_exactly_its_property_and_undoes() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let v = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)])[0];
    let a = insert_text(&mut fx, 0);
    let b = insert_text(&mut fx, 20);
    for (edit, paths) in style_edits() {
        let before = text_of(&fx, a);
        let committed = style(&mut fx, &[v, a, b], edit.clone()).unwrap();
        assert!(matches!(committed, Committed::Applied { .. }), "{edit:?}");
        let label = fx.editor.history().undo_label().unwrap();
        assert_eq!(
            (label.op, label.count),
            (OpKind::SetTextStyle, 2),
            "{edit:?}"
        );
        let after = text_of(&fx, a);
        assert_changed_exactly(&before, &after, &paths);
        assert_eq!(text_of(&fx, b), after, "both clips get the edit");
        // Repeating the edit changes nothing.
        assert_eq!(
            style(&mut fx, &[a, b], edit.clone()).unwrap(),
            Committed::NoOp,
            "{edit:?}"
        );
    }
    // Every Style and Box & bubble property was reached (Font, Size, Bold
    // and Italic are covered by tests/text.rs).
    let end = text_of(&fx, a);
    assert!(end.style.is_curved());
    assert!(end.style.bubble.on && !end.style.background.on);
}

#[test]
fn every_layout_edit_sets_exactly_its_property_and_undoes() {
    let mut fx = Fixture::new();
    let a = insert_text(&mut fx, 0);
    for (edit, paths) in [
        (TextLayoutEdit::Align(HAlign::Left), vec![P::Align]),
        (TextLayoutEdit::Justify(true), vec![P::Justify]),
        (TextLayoutEdit::VAnchor(VAnchor::Bottom), vec![P::VAnchor]),
        (TextLayoutEdit::VAnchor(VAnchor::Top), vec![P::VAnchor]),
        // Picking an alignment ends justification.
        (
            TextLayoutEdit::Align(HAlign::Right),
            vec![P::Align, P::Justify],
        ),
    ] {
        let before = text_of(&fx, a);
        layout(&mut fx, &[a], edit).unwrap();
        assert_eq!(
            fx.editor.history().undo_label().unwrap().op,
            OpKind::SetTextLayout
        );
        assert_changed_exactly(&before, &text_of(&fx, a), &paths);
    }
    assert_eq!(
        layout(&mut fx, &[a], TextLayoutEdit::Justify(false)).unwrap(),
        Committed::NoOp
    );
    assert_eq!(
        text_of(&fx, a).layout,
        TextLayout {
            align: HAlign::Right,
            v_anchor: VAnchor::Top,
            ..TextLayout::default()
        }
    );
}

#[test]
fn background_and_bubble_switch_each_other_off_in_one_entry() {
    let mut fx = Fixture::new();
    let a = insert_text(&mut fx, 0);
    let bg = |on| TextStyleEdit::SetLayer {
        layer: TextLayerKind::Background,
        on,
    };
    let bubble = |on| TextStyleEdit::SetLayer {
        layer: TextLayerKind::Bubble,
        on,
    };
    style(&mut fx, &[a], TextStyleEdit::BackgroundColor(red())).unwrap();
    style(&mut fx, &[a], bg(true)).unwrap();
    let with_box = text_of(&fx, a);
    style(&mut fx, &[a], bubble(true)).unwrap();
    let s = text_of(&fx, a).style.clone();
    assert!(s.bubble.on && !s.background.on);
    // White text on the white default bubble: the bubble turns dark.
    assert_eq!(s.bubble.value.fill, Rgba8::DARK);
    assert_eq!(s.fill, Fill::Solid(Rgba8::WHITE));
    // Background kept its settings while off.
    assert_eq!(s.background.value.color, red());
    // One undo brings the box back and the bubble off.
    fx.editor.undo().unwrap();
    assert_eq!(text_of(&fx, a), with_box);
    fx.editor.redo().unwrap();
    style(&mut fx, &[a], bg(true)).unwrap();
    let s = text_of(&fx, a).style.clone();
    assert!(s.background.on && !s.bubble.on);
    assert_eq!(
        s.bubble.value.fill,
        Rgba8::DARK,
        "the bubble keeps its settings"
    );
    assert_eq!(style(&mut fx, &[a], bg(true)).unwrap(), Committed::NoOp);
}

#[test]
fn switching_a_layer_off_keeps_its_value() {
    let mut fx = Fixture::new();
    let a = insert_text(&mut fx, 0);
    for layer in [
        TextLayerKind::Outline,
        TextLayerKind::Shadow,
        TextLayerKind::Glow,
        TextLayerKind::Background,
        TextLayerKind::Bubble,
        TextLayerKind::Curve,
    ] {
        style(&mut fx, &[a], TextStyleEdit::SetLayer { layer, on: true }).unwrap();
        let on = text_of(&fx, a).style.clone();
        assert!(on.layer_on(layer));
        style(&mut fx, &[a], TextStyleEdit::SetLayer { layer, on: false }).unwrap();
        let off = text_of(&fx, a).style.clone();
        assert!(!off.layer_on(layer));
        assert_eq!(off.with_layer(layer, true), on, "{layer:?}");
    }
    // Editing a layer that is off keeps it off.
    style(
        &mut fx,
        &[a],
        TextStyleEdit::OutlineWidth(PerMille::new(200)),
    )
    .unwrap();
    let s = text_of(&fx, a).style.clone();
    assert!(!s.stroke.on);
    assert_eq!(s.stroke.value.width, PerMille::new(200));
}

#[test]
fn new_style_edits_skip_non_text_clips_and_respect_locks() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let v = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)])[0];
    let curve = TextStyleEdit::SetLayer {
        layer: TextLayerKind::Curve,
        on: true,
    };
    assert_eq!(
        style(&mut fx, &[v], curve.clone()).unwrap_err(),
        EditError::NoTextClip
    );
    assert_eq!(
        reset(&mut fx, &[v], ResetScope::Card(TextCard::Type)).unwrap_err(),
        EditError::NoTextClip
    );
    let a = insert_text(&mut fx, 0);
    let track = fx.doc().find_clip(a).unwrap().track;
    fx.apply(EditRequest::SetTrackHeader {
        seq: fx.seq_id(),
        track,
        header: TrackHeader {
            name: None,
            flags: TrackFlags {
                locked: true,
                ..TrackFlags::default()
            },
        },
    })
    .unwrap();
    assert_eq!(
        style(&mut fx, &[v, a], curve).unwrap_err(),
        EditError::TrackLocked { track }
    );
    assert_eq!(
        reset(&mut fx, &[a], ResetScope::Lens(TextLens::Style)).unwrap_err(),
        EditError::TrackLocked { track }
    );
}

// ---------------------------------------------------------------- reset

#[test]
fn reset_card_restores_defaults_in_one_entry_and_keeps_switches() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let v = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)])[0];
    let a = insert_text(&mut fx, 0);
    let b = insert_text(&mut fx, 20);
    for (edit, _) in style_edits() {
        style(&mut fx, &[a, b], edit).unwrap();
    }
    let edited = text_of(&fx, a);
    let entries = fx.editor.history().undo_label();
    reset(&mut fx, &[v, a, b], ResetScope::Card(TextCard::Bubble)).unwrap();
    let label = fx.editor.history().undo_label().unwrap();
    assert_eq!((label.op, label.count), (OpKind::ResetText, 2));
    let after = text_of(&fx, a);
    assert!(after.style.bubble.on, "the switch is kept");
    assert_eq!(after.style.bubble.value, TextStyle::default().bubble.value);
    // Only the card's properties changed.
    let mut back = (*after).clone();
    for p in freecut_timeline::model::card_props(TextCard::Bubble) {
        p.copy(&edited, &mut back);
    }
    assert_eq!(back, *edited);
    // One undo restores every clip.
    fx.editor.undo().unwrap();
    assert_eq!(text_of(&fx, a), edited);
    assert_eq!(text_of(&fx, b), edited);
    assert_eq!(fx.editor.history().undo_label(), entries);
    fx.editor.redo().unwrap();
    // Nothing left to reset.
    assert_eq!(
        reset(&mut fx, &[a, b], ResetScope::Card(TextCard::Bubble)).unwrap(),
        Committed::NoOp
    );
    // The Text card never touches the content.
    fx.apply(EditRequest::SetTextContent {
        seq: fx.seq_id(),
        clip: a,
        content: "mine".into(),
    })
    .unwrap();
    assert_eq!(
        reset(&mut fx, &[a], ResetScope::Card(TextCard::Text)).unwrap(),
        Committed::NoOp
    );
}

#[test]
fn reset_lens_covers_every_card_of_the_lens() {
    let mut fx = Fixture::new();
    let a = insert_text(&mut fx, 0);
    for (edit, _) in style_edits() {
        style(&mut fx, &[a], edit).unwrap();
    }
    style(&mut fx, &[a], TextStyleEdit::Size(RefPx::from_px(200))).unwrap();
    layout(&mut fx, &[a], TextLayoutEdit::Justify(true)).unwrap();
    layout(
        &mut fx,
        &[a],
        TextLayoutEdit::LineSpacing(PerMille::new(2000)),
    )
    .unwrap();
    fx.apply(EditRequest::SetTransform {
        seq: fx.seq_id(),
        clips: vec![a],
        edit: TransformEdit::Scale(Scale::from_percent(50)),
    })
    .unwrap();
    fx.apply(EditRequest::SetTransform {
        seq: fx.seq_id(),
        clips: vec![a],
        edit: TransformEdit::Opacity(PerMille::new(500)),
    })
    .unwrap();

    // Style: Type, Fill, Spacing (with line spacing), Outline, Glow, Shadow.
    reset(&mut fx, &[a], ResetScope::Lens(TextLens::Style)).unwrap();
    let s = text_of(&fx, a);
    let d = TextStyle::default();
    assert_eq!(s.style.size, d.size);
    assert_eq!(s.style.fill, d.fill);
    assert_eq!(s.style.case, d.case);
    assert_eq!(s.style.letter_spacing, d.letter_spacing);
    assert_eq!(s.layout.line_spacing, PerMille::ONE);
    assert_eq!(s.style.stroke.value, d.stroke.value);
    assert!(s.style.stroke.on && s.style.glow.on && s.style.shadow.on);
    assert!(s.layout.justify, "paragraph is the Content lens's");
    assert_ne!(s.style.bubble.value, d.bubble.value, "not this lens");

    // Box & bubble: Background, Bubble, Curve and the Placement card.
    reset(&mut fx, &[a], ResetScope::Lens(TextLens::BoxAndBubble)).unwrap();
    let s = text_of(&fx, a);
    // The bubble stays on behind the reset white text, so its reset colour
    // is TX-055's dark default rather than the factory white.
    let mut readable = d.bubble.value;
    readable.fill = Rgba8::DARK;
    assert_eq!(s.style.bubble.value, readable);
    assert_eq!(s.style.background.value, d.background.value);
    assert_eq!(s.style.curve.value, d.curve.value);
    assert!(s.style.curve.on && s.style.bubble.on);
    assert_eq!(fx.doc().clip(a).unwrap().transform, Transform::IDENTITY);

    // Content: the paragraph only, never the text.
    reset(&mut fx, &[a], ResetScope::Lens(TextLens::Content)).unwrap();
    let s = text_of(&fx, a);
    assert_eq!(s.layout, TextLayout::default());
    assert_eq!(s.content.as_str(), "Your text");
}

// ---------------------------------------------------------------- change case

#[test]
fn change_case_rewrites_the_selection_as_one_content_edit() {
    let mut fx = Fixture::new();
    let a = insert_text(&mut fx, 0);
    fx.apply(EditRequest::SetTextContent {
        seq: fx.seq_id(),
        clip: a,
        content: "straße in izmir".into(),
    })
    .unwrap();
    let seq = fx.seq_id();
    let req = change_case(fx.doc(), seq, a, 0..7, CaseTransform::Upper, None).unwrap();
    fx.apply(req).unwrap();
    assert_eq!(
        fx.editor.history().undo_label().unwrap().op,
        OpKind::SetTextContent
    );
    assert_eq!(text_of(&fx, a).content.as_str(), "STRASSE in izmir");
    let tr = LangTag::new("tr");
    let text = text_of(&fx, a).content.as_str().to_owned();
    let start = text.find("izmir").unwrap();
    let req = change_case(
        fx.doc(),
        seq,
        a,
        start..text.len(),
        CaseTransform::Title,
        Some(&tr),
    )
    .unwrap();
    fx.apply(req).unwrap();
    assert_eq!(text_of(&fx, a).content.as_str(), "STRASSE in İzmir");
    // The display transform still applies on top, as a style value.
    assert_eq!(text_of(&fx, a).style.case, CaseTransform::AsTyped);
    // Unchanged text records nothing.
    let req = change_case(fx.doc(), seq, a, 0..3, CaseTransform::Upper, None).unwrap();
    assert_eq!(fx.apply(req).unwrap(), Committed::NoOp);
    // Only text clips.
    let main = fx.main();
    let v = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)])[0];
    assert_eq!(
        change_case(fx.doc(), seq, v, 0..1, CaseTransform::Upper, None).unwrap_err(),
        EditError::NoTextClip
    );
}

#[test]
fn edited_text_data_keeps_through_split() {
    let mut fx = Fixture::new();
    let a = insert_text(&mut fx, 0);
    for (edit, _) in style_edits() {
        style(&mut fx, &[a], edit).unwrap();
    }
    layout(&mut fx, &[a], TextLayoutEdit::VAnchor(VAnchor::Bottom)).unwrap();
    let data = text_of(&fx, a);
    let Committed::Applied { selection, .. } = fx
        .apply(EditRequest::Split(freecut_timeline::edit::SplitReq {
            seq: fx.seq_id(),
            at: f(4),
            targets: freecut_timeline::edit::SplitTargets::Clips(vec![a]),
        }))
        .unwrap()
    else {
        panic!("no-op");
    };
    assert_eq!(text_of(&fx, selection.unwrap()[0]), data);
}
