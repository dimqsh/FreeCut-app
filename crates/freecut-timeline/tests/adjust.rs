//! Adjustment clips and applying sections (CL-006, CL-025, CL-026, CL-044):
//! `AddAdjustmentClip` with its span and placement rules, the plan's
//! `PlanNode::Adjust`, and `ColorEdit::ApplySections`. Every edit goes
//! through `Fixture::apply`, which checks plan purity, validity and the
//! undo/redo round trip.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;

use common::*;
use freecut_timeline::Committed;
use freecut_timeline::edit::{
    AdjustSpan, ColorEdit, ColorOp, EditError, EditRequest, InsertReq, InsertTarget, KeyEdit,
    NewClip, OpKind, default_adjustment_duration,
};
use freecut_timeline::ids::{ClipId, TrackId};
use freecut_timeline::model::{
    ClipContent, ColorParamId, ColorStack, GeneratedKind, HslChannel, HueBand, LutRef, LutSlot,
    PropPath, PropValue, Section, SectionMask, SectionSnapshot, TextClipData, TrackFlags,
    TrackHeader, TrackKind,
};
use freecut_timeline::plan::{PlanInputs, PlanNode, frame_plan};
use pretty_assertions::assert_eq;

fn add_adjustment(
    fx: &mut Fixture,
    span: AdjustSpan,
    color: Option<ColorStack>,
) -> Result<ClipId, EditError> {
    let req = EditRequest::AddAdjustmentClip {
        seq: fx.seq_id(),
        span,
        color: color.map(Arc::new),
    };
    match fx.apply(req)? {
        Committed::Applied { selection, .. } => Ok(selection.unwrap()[0]),
        Committed::NoOp => panic!("no-op"),
    }
}

fn track_of(fx: &Fixture, clip: ClipId) -> (usize, TrackId) {
    let loc = fx.doc().find_clip(clip).unwrap();
    (loc.slot.index, loc.track)
}

fn label(fx: &Fixture) -> (OpKind, u32) {
    let l = fx.editor.history().undo_label().unwrap();
    (l.op, l.count)
}

fn warm() -> ColorStack {
    ColorStack::default().with(ColorParamId::Temperature, 40)
}

fn set_header(fx: &mut Fixture, track: TrackId, flags: TrackFlags) {
    fx.apply(EditRequest::SetTrackHeader {
        seq: fx.seq_id(),
        track,
        header: TrackHeader { name: None, flags },
    })
    .unwrap();
}

#[test]
fn a_blank_adjustment_clip_at_the_playhead_lasts_three_seconds_above_the_video() {
    let mut fx = Fixture::new();
    let main = fx.main();
    fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 100)]);
    let before = fx.seq().visual_tracks().len();
    let adj = add_adjustment(&mut fx, AdjustSpan::AtPlayhead(f(10)), None).unwrap();
    assert_eq!(label(&fx), (OpKind::AddAdjustmentClip, 1));
    let clip = fx.doc().clip(adj).unwrap();
    assert_eq!(
        clip.content,
        ClipContent::Generated {
            kind: GeneratedKind::Adjustment
        }
    );
    assert_eq!(clip.color, None, "blank: an identity stack stores nothing");
    assert_eq!(clip.start(), f(10));
    assert_eq!(
        clip.duration(),
        default_adjustment_duration(rate()).unwrap()
    );
    assert_eq!(clip.duration(), f(75), "3 s at 25 fps");
    let (index, track) = track_of(&fx, adj);
    assert_eq!(fx.seq().visual_tracks().len(), before + 1);
    assert_eq!(index, 1, "directly above the main track");
    assert_eq!(
        fx.seq().find_track(track).unwrap().1.kind,
        TrackKind::Adjustment
    );
    // A playhead before 0 starts at 0, and a time between frames lands on
    // the grid.
    let early = add_adjustment(&mut fx, AdjustSpan::AtPlayhead(f(-5)), None).unwrap();
    assert_eq!(fx.doc().clip(early).unwrap().start(), f(0));
}

#[test]
fn the_selection_span_is_the_union_of_the_selected_clips() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let a = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 20)])[0];
    let b = fx.insert(InsertTarget::NewTrack, 30, &[(VIDEO_50, 0, 20)])[0];
    let adj = add_adjustment(&mut fx, AdjustSpan::Selection(vec![b, a]), Some(warm())).unwrap();
    let clip = fx.doc().clip(adj).unwrap();
    assert_eq!((clip.start(), clip.end()), (f(0), f(50)));
    assert_eq!(clip.color.as_deref(), Some(&warm()), "it carries the grade");
    // Above the topmost track with content in the span (b's overlay).
    let (b_index, _) = track_of(&fx, b);
    assert!(track_of(&fx, adj).0 > b_index);
    assert_eq!(fx.editor.history().undo_label().unwrap().count, 1);
    assert_eq!(
        add_adjustment(&mut fx, AdjustSpan::Selection(vec![]), None),
        Err(EditError::NothingSelected)
    );
}

#[test]
fn a_free_adjustment_track_above_the_content_is_reused() {
    let mut fx = Fixture::new();
    let main = fx.main();
    fx.insert(
        InsertTarget::Track(main),
        0,
        &[(VIDEO_100, 0, 100), (VIDEO_100, 0, 100)],
    );
    let first = add_adjustment(&mut fx, AdjustSpan::AtPlayhead(f(0)), None).unwrap();
    let tracks = fx.seq().visual_tracks().len();
    // Free there: the same track.
    let second = add_adjustment(&mut fx, AdjustSpan::AtPlayhead(f(80)), None).unwrap();
    assert_eq!(fx.seq().visual_tracks().len(), tracks);
    assert_eq!(track_of(&fx, second).1, track_of(&fx, first).1);
    // Over the first one: a new track above it, so the two stack.
    let third = add_adjustment(&mut fx, AdjustSpan::AtPlayhead(f(10)), None).unwrap();
    assert_eq!(fx.seq().visual_tracks().len(), tracks + 1);
    assert!(track_of(&fx, third).0 > track_of(&fx, first).0);
}

#[test]
fn locked_and_hidden_adjustment_tracks_are_not_used() {
    let mut fx = Fixture::new();
    let main = fx.main();
    fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 100)]);
    let first = add_adjustment(&mut fx, AdjustSpan::AtPlayhead(f(0)), None).unwrap();
    let (_, track) = track_of(&fx, first);
    set_header(
        &mut fx,
        track,
        TrackFlags {
            locked: true,
            ..TrackFlags::default()
        },
    );
    let next = add_adjustment(&mut fx, AdjustSpan::AtPlayhead(f(80)), None).unwrap();
    assert_ne!(track_of(&fx, next).1, track, "a locked track is skipped");
    // A hidden adjustment track with room is skipped too.
    let mut fx = Fixture::new();
    let main = fx.main();
    fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 100)]);
    let a = add_adjustment(&mut fx, AdjustSpan::AtPlayhead(f(0)), None).unwrap();
    let (_, track) = track_of(&fx, a);
    set_header(
        &mut fx,
        track,
        TrackFlags {
            hidden: true,
            ..TrackFlags::default()
        },
    );
    let b = add_adjustment(&mut fx, AdjustSpan::AtPlayhead(f(80)), None).unwrap();
    assert_ne!(track_of(&fx, b).1, track);
}

#[test]
fn text_above_the_video_puts_the_new_track_above_the_text() {
    let mut fx = Fixture::new();
    let main = fx.main();
    fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 100)]);
    let req = EditRequest::Insert(InsertReq {
        seq: fx.seq_id(),
        target: InsertTarget::NewTrack,
        at: f(0),
        clips: vec![NewClip::text(TextClipData::with_content("Hi"), f(50))],
    });
    let Committed::Applied { selection, .. } = fx.apply(req).unwrap() else {
        panic!("no-op");
    };
    let text = selection.unwrap()[0];
    let adj = add_adjustment(&mut fx, AdjustSpan::AtPlayhead(f(0)), Some(warm())).unwrap();
    assert!(
        track_of(&fx, adj).0 > track_of(&fx, text).0,
        "grades the text"
    );
    // Where the text isn't, only the video: still above it, in the upper
    // band.
    let later = add_adjustment(&mut fx, AdjustSpan::AtPlayhead(f(60)), None).unwrap();
    assert!(track_of(&fx, later).0 >= 1);
}

fn nodes(fx: &Fixture, frame: i64) -> Vec<PlanNode> {
    let offline = BTreeSet::new();
    frame_plan(
        fx.doc(),
        fx.seq_id(),
        f(frame),
        &PlanInputs { offline: &offline },
    )
    .unwrap()
    .nodes
}

#[test]
fn the_plan_grades_at_the_adjustment_clips_place_in_the_walk() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let video = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 100)])[0];
    let blank = add_adjustment(&mut fx, AdjustSpan::AtPlayhead(f(0)), None).unwrap();
    // A blank adjustment clip renders nothing: no node.
    assert_eq!(nodes(&fx, 5).len(), 1);
    // Graded: an Adjust node above the video, carrying the stack.
    fx.apply(EditRequest::EditColor {
        seq: fx.seq_id(),
        clips: vec![blank],
        edit: ColorEdit::SetParam(ColorParamId::Saturation, -100),
    })
    .unwrap();
    let req = EditRequest::Insert(InsertReq {
        seq: fx.seq_id(),
        target: InsertTarget::NewTrack,
        at: f(0),
        clips: vec![NewClip::text(TextClipData::with_content("Hi"), f(50))],
    });
    fx.apply(req).unwrap();
    let shown = nodes(&fx, 5);
    let order: Vec<(ClipId, bool)> = shown
        .iter()
        .map(|n| (n.clip(), matches!(n, PlanNode::Adjust(_))))
        .collect();
    assert_eq!(order[0], (video, false));
    let at = order.iter().position(|(c, _)| *c == blank).unwrap();
    assert!(order[at].1, "an adjustment node, not a layer");
    let PlanNode::Adjust(a) = &shown[at] else {
        unreachable!()
    };
    assert_eq!(a.color.basic.saturation, -100);
    assert_eq!(a.frame_index, 5);
    assert_eq!(a.track, track_of(&fx, blank).1);
    // Mix 0, the master bypass and a hidden track leave it out.
    let without = |fx: &Fixture| !nodes(fx, 5).iter().any(|n| n.clip() == blank);
    for (edit, back) in [
        (
            ColorEdit::SetParam(ColorParamId::Amount, 0),
            ColorEdit::SetParam(ColorParamId::Amount, 100),
        ),
        (ColorEdit::SetBypass(true), ColorEdit::SetBypass(false)),
    ] {
        for (e, gone) in [(edit, true), (back, false)] {
            fx.apply(EditRequest::EditColor {
                seq: fx.seq_id(),
                clips: vec![blank],
                edit: e,
            })
            .unwrap();
            assert_eq!(without(&fx), gone);
        }
    }
    let track = track_of(&fx, blank).1;
    set_header(
        &mut fx,
        track,
        TrackFlags {
            hidden: true,
            ..TrackFlags::default()
        },
    );
    assert!(without(&fx), "a hidden adjustment track is inert");
    // Outside its span it isn't there either.
    assert!(!nodes(&fx, 90).iter().any(|n| n.clip() == blank));
}

#[test]
fn a_keyed_mix_is_evaluated_at_the_frame() {
    let mut fx = Fixture::new();
    let main = fx.main();
    fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 100)]);
    let adj = add_adjustment(&mut fx, AdjustSpan::AtPlayhead(f(0)), Some(warm())).unwrap();
    for (at, v) in [(0, 0), (10, 100)] {
        fx.apply(EditRequest::EditKeys {
            seq: fx.seq_id(),
            clips: vec![adj],
            prop: PropPath::Color(ColorParamId::Amount),
            edit: KeyEdit::Add {
                at: f(at),
                value: Some(PropValue::Level(v)),
            },
        })
        .unwrap();
    }
    assert!(nodes(&fx, 0).len() == 1, "Mix 0 at the first key");
    let mixed = nodes(&fx, 5);
    let Some(PlanNode::Adjust(a)) = mixed.last() else {
        panic!("an adjustment node");
    };
    assert_eq!(a.color.amount, 50);
}

fn look() -> ColorStack {
    ColorStack::default()
        .with(ColorParamId::Exposure, 30)
        .with(ColorParamId::Contrast, 10)
        .with(ColorParamId::Saturation, -20)
        .with(
            ColorParamId::Hsl {
                band: HueBand::Green,
                channel: HslChannel::Hue,
            },
            25,
        )
        .with(ColorParamId::Amount, 40)
        .with_lut(Some(LutSlot::new(LutRef::new([3; 16], "teal.cube"))))
}

#[test]
fn a_snapshot_holds_only_its_sections() {
    let snap = SectionSnapshot::new(&look(), SectionMask::of(&[Section::Light, Section::Lut]));
    assert_eq!(snap.values().basic.exposure, 30);
    assert_eq!(snap.values().basic.saturation, 0, "Color isn't held");
    assert!(snap.values().hsl.is_default(), "HSL isn't held");
    assert_eq!(snap.values().amount, 100, "Amount is never held");
    assert_eq!(
        snap.values().lut.as_ref().unwrap().lut.file_name,
        "teal.cube"
    );
    assert_eq!(
        snap.sections().sections().collect::<Vec<_>>(),
        [Section::Light, Section::Lut]
    );
    assert!(SectionSnapshot::new(&look(), SectionMask::NONE).is_empty());
    // The checklist's pre-ticked sections.
    assert_eq!(
        look()
            .changed_sections(&Section::LOOK)
            .sections()
            .collect::<Vec<_>>(),
        [Section::Light, Section::Color, Section::Lut, Section::Hsl]
    );
    assert!(Section::LOOK.iter().all(|s| s.in_look()));
    assert!(!Section::Masks.in_look() && !Section::Auto.in_look());
}

#[test]
fn applying_sections_replaces_them_and_their_keys_only() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let video = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 100)])[0];
    let adj = add_adjustment(&mut fx, AdjustSpan::AtPlayhead(f(0)), None).unwrap();
    let seq = fx.seq_id();
    let text = {
        let req = EditRequest::Insert(InsertReq {
            seq,
            target: InsertTarget::NewTrack,
            at: f(0),
            clips: vec![NewClip::text(TextClipData::with_content("Hi"), f(50))],
        });
        let Committed::Applied { selection, .. } = fx.apply(req).unwrap() else {
            panic!("no-op");
        };
        selection.unwrap()[0]
    };
    // The video: a keyed Exposure, a Tint and Mix-like Amount 70.
    for (at, v) in [(0, -50), (10, 50)] {
        fx.apply(EditRequest::EditKeys {
            seq,
            clips: vec![video],
            prop: PropPath::Color(ColorParamId::Exposure),
            edit: KeyEdit::Add {
                at: f(at),
                value: Some(PropValue::Level(v)),
            },
        })
        .unwrap();
    }
    fx.apply(EditRequest::EditColor {
        seq,
        clips: vec![video],
        edit: ColorEdit::SetParams(vec![(ColorParamId::Tint, 15), (ColorParamId::Amount, 70)]),
    })
    .unwrap();
    let snap = Arc::new(SectionSnapshot::new(
        &look(),
        SectionMask::of(&[Section::Light, Section::Hsl]),
    ));
    fx.apply(EditRequest::EditColor {
        seq,
        clips: vec![video, adj, text],
        edit: ColorEdit::ApplySections(Arc::clone(&snap)),
    })
    .unwrap();
    assert_eq!(label(&fx), (OpKind::EditColor(ColorOp::Look), 2));
    let v = fx.doc().clip(video).unwrap();
    let s = ColorStack::of(v.color.as_ref());
    assert_eq!((s.basic.exposure, s.basic.contrast), (30, 10));
    assert_eq!(s.basic.tint, 15, "Color isn't replaced");
    assert_eq!(s.basic.saturation, 0);
    assert_eq!(s.amount, 70, "Amount stays");
    assert_eq!(s.hsl.band(HueBand::Green).get(HslChannel::Hue), 25);
    assert!(
        v.keys.is_empty(),
        "the replaced section's keys go in the same entry"
    );
    let a = ColorStack::of(fx.doc().clip(adj).unwrap().color.as_ref());
    assert_eq!(a.basic.exposure, 30, "adjustment clips take looks too");
    assert!(fx.doc().clip(text).unwrap().color.is_none());
    // A section switched off in the snapshot arrives off.
    let off = look();
    let off = ColorStack {
        sections_enabled: off.sections_enabled.with(Section::Light, false),
        ..off
    };
    fx.apply(EditRequest::EditColor {
        seq,
        clips: vec![video],
        edit: ColorEdit::ApplySections(Arc::new(SectionSnapshot::new(
            &off,
            SectionMask::of(&[Section::Light]),
        ))),
    })
    .unwrap();
    let s = ColorStack::of(fx.doc().clip(video).unwrap().color.as_ref());
    assert!(!s.sections_enabled.is_on(Section::Light));
    // Only text: refused.
    assert_eq!(
        fx.apply(EditRequest::EditColor {
            seq,
            clips: vec![text],
            edit: ColorEdit::ApplySections(snap),
        }),
        Err(EditError::NoColorClip)
    );
}

#[test]
fn applying_the_same_sections_again_changes_nothing() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let video = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 100)])[0];
    let snap = Arc::new(SectionSnapshot::new(
        &look(),
        SectionMask::of(&Section::LOOK),
    ));
    let apply = |fx: &mut Fixture| {
        fx.apply(EditRequest::EditColor {
            seq: fx.seq_id(),
            clips: vec![video],
            edit: ColorEdit::ApplySections(Arc::clone(&snap)),
        })
        .unwrap()
    };
    assert!(matches!(apply(&mut fx), Committed::Applied { .. }));
    assert!(matches!(apply(&mut fx), Committed::NoOp));
    let s = ColorStack::of(fx.doc().clip(video).unwrap().color.as_ref());
    assert_eq!(s.lut.unwrap().lut.hash, [3; 16]);
}
