//! Keyframes of transform and text properties (TR-027 to TR-031, TX-021):
//! every key edit through `Fixture::apply` (plan purity, validity and the
//! apply/undo/redo triple), the multi-clip rule, card commands, reset, the
//! split and trim rules, and evaluation in the frame plan.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use std::collections::BTreeSet;

use common::*;
use freecut_timeline::Committed;
use freecut_timeline::edit::{
    Edge, EditError, EditRequest, GainKeyEdit, InsertReq, InsertTarget, KeyEdit, NewClip, OpKind,
    ResetScope, SplitReq, SplitTargets, TextStyleEdit, TrimReq,
};
use freecut_timeline::ids::ClipId;
use freecut_timeline::model::{
    CanvasPos, EaseCurve, EaseDir, FillKind, GainDb, Interp, PerMille, PropGroup, PropPath,
    PropValue, RefPx, Rgba8, TextCard, TextClipData, TextPropPath, TrackFlags, TrackHeader,
    TransformProp,
};
use freecut_timeline::plan::{LayerPlan, PlanInputs, frame_plan};
use freecut_timeline::time::Tick;
use pretty_assertions::assert_eq;

const POSITION: PropPath = PropPath::Transform(TransformProp::Position);
const OPACITY: PropPath = PropPath::Transform(TransformProp::Opacity);
const SIZE: PropPath = PropPath::Text(TextPropPath::Size);

fn insert_text(fx: &mut Fixture, at: i64, len: i64) -> ClipId {
    let req = EditRequest::Insert(InsertReq {
        seq: fx.seq_id(),
        target: InsertTarget::FirstFree,
        at: f(at),
        clips: vec![NewClip::text(TextClipData::default(), f(len))],
    });
    match fx.apply(req).unwrap() {
        Committed::Applied { selection, .. } => selection.unwrap()[0],
        Committed::NoOp => panic!("insert was a no-op"),
    }
}

fn keys(
    fx: &mut Fixture,
    clips: &[ClipId],
    prop: PropPath,
    edit: KeyEdit,
) -> Result<Committed, EditError> {
    fx.apply(EditRequest::EditKeys {
        seq: fx.seq_id(),
        clips: clips.to_vec(),
        prop,
        edit,
    })
}

fn pos(x: i32, y: i32) -> PropValue {
    PropValue::Position(CanvasPos::new(x, y))
}

fn key_times(fx: &Fixture, clip: ClipId, prop: PropPath) -> Vec<Tick> {
    fx.doc()
        .clip(clip)
        .unwrap()
        .keys
        .keys(prop)
        .iter()
        .map(|k| k.at)
        .collect()
}

fn label(fx: &Fixture) -> (OpKind, u32) {
    let l = fx.editor.history().undo_label().unwrap();
    (l.op, l.count)
}

fn layer(fx: &Fixture, frame: i64) -> LayerPlan {
    let none = BTreeSet::new();
    let plan = frame_plan(
        fx.doc(),
        fx.seq_id(),
        f(frame),
        &PlanInputs { offline: &none },
    )
    .unwrap();
    plan.nodes.last().unwrap().layer().unwrap().clone()
}

#[test]
fn transform_keys_add_move_ease_and_remove() {
    let mut fx = Fixture::new();
    let clip = insert_text(&mut fx, 10, 40);
    // The first key captures the static value (TR-027).
    keys(
        &mut fx,
        &[clip],
        POSITION,
        KeyEdit::Add {
            at: f(10),
            value: None,
        },
    )
    .unwrap();
    assert_eq!(label(&fx), (OpKind::EditKeys, 1));
    keys(
        &mut fx,
        &[clip],
        POSITION,
        KeyEdit::Add {
            at: f(30),
            value: Some(pos(200_000, 0)),
        },
    )
    .unwrap();
    // Content time of a text clip: clip-local time.
    assert_eq!(key_times(&fx, clip, POSITION), vec![f(0), f(20)]);
    let c = fx.doc().clip(clip).unwrap();
    assert_eq!(c.transform_at(f(10)).position, CanvasPos::new(100_000, 0));
    assert_eq!(
        c.transform.position,
        CanvasPos::CENTER,
        "the static value stays"
    );
    // Ease the first segment (TR-031).
    let quad_in = Interp::Ease {
        curve: EaseCurve::Quad,
        dir: EaseDir::In,
    };
    keys(
        &mut fx,
        &[clip],
        POSITION,
        KeyEdit::SetInterp {
            at: f(10),
            interp: quad_in,
        },
    )
    .unwrap();
    let c = fx.doc().clip(clip).unwrap();
    assert_eq!(c.transform_at(f(10)).position, CanvasPos::new(50_000, 0));
    // Move the second key; it can't pass the first.
    keys(
        &mut fx,
        &[clip],
        POSITION,
        KeyEdit::Move {
            from: f(30),
            to: f(0),
        },
    )
    .unwrap();
    assert_eq!(
        key_times(&fx, clip, POSITION),
        vec![f(0), Tick::from_raw(1)]
    );
    // Nothing at the old time any more.
    assert_eq!(
        keys(
            &mut fx,
            &[clip],
            POSITION,
            KeyEdit::Move {
                from: f(30),
                to: f(35)
            }
        ),
        Err(EditError::NoKey { at: f(30) })
    );
    let moved = Tick::from_raw(f(10).raw() + 1);
    keys(
        &mut fx,
        &[clip],
        POSITION,
        KeyEdit::Move {
            from: moved,
            to: f(35),
        },
    )
    .unwrap();
    assert_eq!(key_times(&fx, clip, POSITION), vec![f(0), f(25)]);
    // Removing the last key writes its value as the static value.
    keys(&mut fx, &[clip], POSITION, KeyEdit::Remove { at: f(10) }).unwrap();
    keys(&mut fx, &[clip], POSITION, KeyEdit::Remove { at: f(35) }).unwrap();
    let c = fx.doc().clip(clip).unwrap();
    assert!(c.keys.is_empty());
    assert_eq!(c.transform.position, CanvasPos::new(200_000, 0));
}

#[test]
fn key_values_are_clamped_and_must_fit_the_property() {
    let mut fx = Fixture::new();
    let clip = insert_text(&mut fx, 0, 20);
    keys(
        &mut fx,
        &[clip],
        SIZE,
        KeyEdit::Add {
            at: f(0),
            value: Some(PropValue::Length(RefPx::from_px(1))),
        },
    )
    .unwrap();
    assert_eq!(
        fx.doc().clip(clip).unwrap().keys.keys(SIZE)[0].value,
        PropValue::Length(RefPx::from_px(4)),
        "clamped to TX-004's minimum"
    );
    assert_eq!(
        keys(
            &mut fx,
            &[clip],
            SIZE,
            KeyEdit::Add {
                at: f(0),
                value: Some(PropValue::Ratio(PerMille::ONE))
            }
        ),
        Err(EditError::WrongValueKind)
    );
    assert_eq!(
        keys(
            &mut fx,
            &[clip],
            PropPath::Text(TextPropPath::Font),
            KeyEdit::Add {
                at: f(0),
                value: None
            }
        ),
        Err(EditError::NotAnimatable)
    );
    assert_eq!(
        keys(
            &mut fx,
            &[clip],
            SIZE,
            KeyEdit::Add {
                at: f(50),
                value: None
            }
        ),
        Err(EditError::NotUnderPlayhead)
    );
    assert_eq!(
        keys(&mut fx, &[clip], SIZE, KeyEdit::Remove { at: f(5) }),
        Err(EditError::NoKey { at: f(5) })
    );
    // Text properties on a video clip, transform properties on audio.
    let video = fx.insert(InsertTarget::Track(fx.main()), 0, &[(VIDEO_50, 0, 10)])[0];
    assert_eq!(
        keys(
            &mut fx,
            &[video],
            SIZE,
            KeyEdit::Add {
                at: f(0),
                value: None
            }
        ),
        Err(EditError::NoTextClip)
    );
    let audio = fx.insert(InsertTarget::NewTrack, 0, &[(AUDIO_200, 0, 10)])[0];
    assert_eq!(
        keys(
            &mut fx,
            &[audio],
            OPACITY,
            KeyEdit::Add {
                at: f(0),
                value: None
            }
        ),
        Err(EditError::NoVisualClip)
    );
    // A video clip takes transform keys, in its source time.
    keys(
        &mut fx,
        &[video],
        OPACITY,
        KeyEdit::Add {
            at: f(4),
            value: Some(PropValue::Ratio(PerMille::new(300))),
        },
    )
    .unwrap();
    let v = fx.doc().clip(video).unwrap();
    assert_eq!(v.keys.keys(OPACITY)[0].at, v.source_time_at(f(4)).unwrap());
}

#[test]
fn a_key_on_several_clips_lands_on_those_under_the_playhead() {
    let mut fx = Fixture::new();
    let a = insert_text(&mut fx, 0, 20);
    let b = insert_text(&mut fx, 5, 20);
    let c = insert_text(&mut fx, 40, 10);
    keys(
        &mut fx,
        &[a, b, c],
        SIZE,
        KeyEdit::Add {
            at: f(10),
            value: Some(PropValue::Length(RefPx::from_px(90))),
        },
    )
    .unwrap();
    assert_eq!(label(&fx), (OpKind::EditKeys, 2));
    assert_eq!(key_times(&fx, a, SIZE), vec![f(10)]);
    assert_eq!(key_times(&fx, b, SIZE), vec![f(5)], "b's own content time");
    assert!(key_times(&fx, c, SIZE).is_empty(), "not under the playhead");
}

#[test]
fn key_this_card_is_one_entry_and_skips_keys_already_there() {
    let mut fx = Fixture::new();
    let clip = insert_text(&mut fx, 0, 30);
    let outline = PropGroup::Text(TextCard::Outline);
    keys(
        &mut fx,
        &[clip],
        OPACITY,
        KeyEdit::KeyCard {
            card: outline,
            at: f(3),
        },
    )
    .unwrap();
    assert_eq!(label(&fx), (OpKind::EditKeys, 1));
    let props: Vec<PropPath> = fx
        .doc()
        .clip(clip)
        .unwrap()
        .keys
        .tracks()
        .iter()
        .map(|t| t.prop())
        .collect();
    assert_eq!(
        props,
        vec![
            PropPath::Text(TextPropPath::OutlineColor),
            PropPath::Text(TextPropPath::OutlineWidth),
            PropPath::Text(TextPropPath::OutlineOpacity),
        ]
    );
    // Everything is keyed there already: nothing changes, no entry.
    let entries = fx.editor.history().len();
    assert_eq!(
        keys(
            &mut fx,
            &[clip],
            OPACITY,
            KeyEdit::KeyCard {
                card: outline,
                at: f(3)
            }
        )
        .unwrap(),
        Committed::NoOp
    );
    assert_eq!(fx.editor.history().len(), entries);
    // The Fill card keys the colour of every stop of a gradient.
    fx.apply(EditRequest::SetTextStyle {
        seq: fx.seq_id(),
        clips: vec![clip],
        edit: TextStyleEdit::FillKind(FillKind::LinearGradient),
    })
    .unwrap();
    keys(
        &mut fx,
        &[clip],
        OPACITY,
        KeyEdit::KeyCard {
            card: PropGroup::Text(TextCard::Fill),
            at: f(3),
        },
    )
    .unwrap();
    let c = fx.doc().clip(clip).unwrap();
    assert!(c.keys.is_animated(PropPath::FillStopColor(0)));
    assert!(c.keys.is_animated(PropPath::FillStopColor(1)));
    assert!(c.keys.is_animated(PropPath::Text(TextPropPath::FillAngle)));
    // Transform group: TR-028's "all transform properties".
    keys(
        &mut fx,
        &[clip],
        OPACITY,
        KeyEdit::KeyCard {
            card: PropGroup::Transform,
            at: f(3),
        },
    )
    .unwrap();
    assert!(TransformProp::ALL.iter().all(|p| {
        fx.doc()
            .clip(clip)
            .unwrap()
            .keys
            .is_animated(PropPath::Transform(*p))
    }));
    // Remove this card's keys here, then every key of a card.
    keys(
        &mut fx,
        &[clip],
        OPACITY,
        KeyEdit::ClearCard {
            card: outline,
            at: Some(f(3)),
        },
    )
    .unwrap();
    assert!(
        !fx.doc()
            .clip(clip)
            .unwrap()
            .keys
            .is_animated(PropPath::Text(TextPropPath::OutlineWidth))
    );
    keys(
        &mut fx,
        &[clip],
        OPACITY,
        KeyEdit::ClearCard {
            card: PropGroup::Transform,
            at: None,
        },
    )
    .unwrap();
    assert!(!fx.doc().clip(clip).unwrap().keys.is_animated(OPACITY));
    assert_eq!(
        keys(
            &mut fx,
            &[clip],
            OPACITY,
            KeyEdit::ClearCard {
                card: outline,
                at: Some(f(3))
            }
        ),
        Err(EditError::NoKey { at: f(3) })
    );
    // Clearing a card without keys changes nothing.
    assert_eq!(
        keys(
            &mut fx,
            &[clip],
            OPACITY,
            KeyEdit::ClearCard {
                card: PropGroup::Text(TextCard::Glow),
                at: None
            }
        )
        .unwrap(),
        Committed::NoOp
    );
}

#[test]
fn reset_text_removes_the_cards_keys_with_its_values() {
    let mut fx = Fixture::new();
    let clip = insert_text(&mut fx, 0, 30);
    keys(
        &mut fx,
        &[clip],
        SIZE,
        KeyEdit::Add {
            at: f(0),
            value: Some(PropValue::Length(RefPx::from_px(30))),
        },
    )
    .unwrap();
    keys(
        &mut fx,
        &[clip],
        PropPath::Text(TextPropPath::OutlineWidth),
        KeyEdit::Add {
            at: f(0),
            value: None,
        },
    )
    .unwrap();
    keys(
        &mut fx,
        &[clip],
        POSITION,
        KeyEdit::Add {
            at: f(0),
            value: None,
        },
    )
    .unwrap();
    let before = fx.doc().clone();
    fx.apply(EditRequest::ResetText {
        seq: fx.seq_id(),
        clips: vec![clip],
        scope: ResetScope::Card(TextCard::Type),
    })
    .unwrap();
    assert_eq!(label(&fx), (OpKind::ResetText, 1));
    let c = fx.doc().clip(clip).unwrap();
    assert!(!c.keys.is_animated(SIZE), "the Type card's keys go");
    assert!(
        c.keys
            .is_animated(PropPath::Text(TextPropPath::OutlineWidth)),
        "others stay"
    );
    assert!(c.keys.is_animated(POSITION));
    // The Box & bubble lens resets the placement, with its keys.
    fx.apply(EditRequest::ResetText {
        seq: fx.seq_id(),
        clips: vec![clip],
        scope: ResetScope::Lens(freecut_timeline::model::TextLens::BoxAndBubble),
    })
    .unwrap();
    assert!(!fx.doc().clip(clip).unwrap().keys.is_animated(POSITION));
    // One undo each brings values and keys back together.
    fx.editor.undo().unwrap();
    fx.editor.undo().unwrap();
    assert_eq!(fx.doc(), &before);
}

#[test]
fn volume_keys_keep_their_own_request_and_history_label() {
    let mut fx = Fixture::new();
    let a = fx.insert(InsertTarget::NewTrack, 0, &[(AUDIO_200, 0, 50)])[0];
    let src = fx.doc().clip(a).unwrap().source_time_at(f(5)).unwrap();
    fx.apply(EditRequest::EditGainKeys {
        seq: fx.seq_id(),
        clips: vec![a],
        edit: GainKeyEdit::Set {
            at: src,
            gain: Some(GainDb::from_hundredths(-600)),
        },
    })
    .unwrap();
    assert_eq!(label(&fx), (OpKind::EditGainKeys, 1));
    // An eased volume segment evaluates through the same engine.
    fx.apply(EditRequest::EditGainKeys {
        seq: fx.seq_id(),
        clips: vec![a],
        edit: GainKeyEdit::Set {
            at: src.checked_add(f(10)).unwrap(),
            gain: Some(GainDb::ZERO),
        },
    })
    .unwrap();
    fx.apply(EditRequest::EditGainKeys {
        seq: fx.seq_id(),
        clips: vec![a],
        edit: GainKeyEdit::Interp {
            at: src,
            interp: Interp::Ease {
                curve: EaseCurve::Quad,
                dir: EaseDir::In,
            },
        },
    })
    .unwrap();
    assert_eq!(label(&fx), (OpKind::EditGainKeys, 1));
    let gain = &fx.doc().clip(a).unwrap().audio.active().unwrap().gain;
    assert_eq!(
        gain.value_at(src.checked_add(f(5)).unwrap()),
        GainDb::from_hundredths(-450)
    );
}

#[test]
fn splits_keep_every_key_and_trims_keep_keys_on_the_content() {
    let mut fx = Fixture::new();
    let clip = insert_text(&mut fx, 0, 40);
    keys(
        &mut fx,
        &[clip],
        POSITION,
        KeyEdit::Add {
            at: f(0),
            value: Some(pos(0, 0)),
        },
    )
    .unwrap();
    keys(
        &mut fx,
        &[clip],
        POSITION,
        KeyEdit::Add {
            at: f(39),
            value: Some(pos(390_000, 0)),
        },
    )
    .unwrap();
    let at = |fx: &Fixture, frame| layer(fx, frame).layer_to_canvas;
    let frames = [5, 19, 20, 21, 30, 38];
    let samples: Vec<_> = frames.iter().map(|&n| at(&fx, n)).collect();
    fx.apply(EditRequest::Split(SplitReq {
        seq: fx.seq_id(),
        at: f(20),
        targets: SplitTargets::Clips(vec![clip]),
    }))
    .unwrap();
    let after: Vec<_> = frames.iter().map(|&n| at(&fx, n)).collect();
    assert_eq!(samples, after, "a split never changes the picture");
    let right = fx
        .seq()
        .tracks()
        .flat_map(|(_, t)| t.clips().iter())
        .find(|c| c.start() == f(20))
        .unwrap()
        .clone();
    assert_eq!(right.keys.origin(), f(20));
    assert_eq!(
        right.keys.keys(POSITION).len(),
        2,
        "both halves keep all keys"
    );
    // A head trim moves the content origin with the in-point.
    fx.apply(EditRequest::Trim(TrimReq {
        seq: fx.seq_id(),
        clip: right.id,
        edge: Edge::In,
        delta: f(5),
        ripple: false,
    }))
    .unwrap();
    assert_eq!(fx.doc().clip(right.id).unwrap().keys.origin(), f(25));
    assert_eq!(
        (at(&fx, 30), at(&fx, 38)),
        (samples[4], samples[5]),
        "keys stay on the content"
    );
}

#[test]
fn the_frame_plan_shows_keyed_values() {
    let mut fx = Fixture::new();
    let clip = insert_text(&mut fx, 0, 40);
    keys(
        &mut fx,
        &[clip],
        SIZE,
        KeyEdit::Add {
            at: f(0),
            value: Some(PropValue::Length(RefPx::from_px(40))),
        },
    )
    .unwrap();
    keys(
        &mut fx,
        &[clip],
        SIZE,
        KeyEdit::Add {
            at: f(20),
            value: Some(PropValue::Length(RefPx::from_px(80))),
        },
    )
    .unwrap();
    keys(
        &mut fx,
        &[clip],
        OPACITY,
        KeyEdit::Add {
            at: f(0),
            value: Some(PropValue::Ratio(PerMille::new(200))),
        },
    )
    .unwrap();
    keys(
        &mut fx,
        &[clip],
        PropPath::Text(TextPropPath::OutlineColor),
        KeyEdit::Add {
            at: f(0),
            value: Some(PropValue::Color(Rgba8::new(0, 0, 0, 255))),
        },
    )
    .unwrap();
    keys(
        &mut fx,
        &[clip],
        PropPath::Text(TextPropPath::OutlineColor),
        KeyEdit::Add {
            at: f(20),
            value: Some(PropValue::Color(Rgba8::new(255, 255, 255, 255))),
        },
    )
    .unwrap();
    keys(
        &mut fx,
        &[clip],
        PropPath::Reveal,
        KeyEdit::Add {
            at: f(0),
            value: Some(PropValue::Ratio(PerMille::ZERO)),
        },
    )
    .unwrap();
    keys(
        &mut fx,
        &[clip],
        PropPath::Reveal,
        KeyEdit::Add {
            at: f(20),
            value: Some(PropValue::Ratio(PerMille::ONE)),
        },
    )
    .unwrap();
    let l = layer(&fx, 10);
    let freecut_timeline::plan::LayerSource::Text { data, .. } = &l.source else {
        panic!("not text");
    };
    assert_eq!(data.style.size, RefPx::from_px(60));
    assert_eq!(
        data.style.stroke.value.color,
        Rgba8::new(188, 188, 188, 255),
        "linear light"
    );
    assert!((l.opacity - 0.2).abs() < 1e-12);
    assert!((l.motion.as_ref().unwrap().text_reveal - 0.5).abs() < 1e-12);
    // At rest after the last key: no motion record, stored style untouched.
    let l = layer(&fx, 30);
    assert!(l.motion.is_none());
    assert_eq!(
        fx.doc()
            .clip(clip)
            .unwrap()
            .content
            .text()
            .unwrap()
            .style
            .size,
        RefPx::from_px(64)
    );
}

#[test]
fn keys_on_locked_tracks_are_refused() {
    let mut fx = Fixture::new();
    let clip = insert_text(&mut fx, 0, 20);
    let track = fx.doc().find_clip(clip).unwrap().track;
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
    assert!(matches!(
        keys(
            &mut fx,
            &[clip],
            SIZE,
            KeyEdit::Add {
                at: f(0),
                value: None
            }
        ),
        Err(EditError::TrackLocked { .. })
    ));
}
