//! Planning cost on a large sequence (not gating; run with
//! `cargo test -p freecut-timeline --release --test bench_plan -- --ignored --nocapture`).
//! Budget from the architecture: a single-clip op on 10,000 clips in < 2 ms;
//! a frame plan of such a document, animated, stays within it too.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use std::time::Instant;

use std::collections::BTreeSet;

use common::*;
use freecut_timeline::edit::{
    Edge, EditRequest, InsertReq, InsertTarget, KeyEdit, MoveReq, NewClip, TrimReq,
};
use freecut_timeline::model::{
    AnimUnit, AnimationSpec, MediaKind, PresetId, PropGroup, PropPath, Slot, TextCard,
    TextClipData, TransformProp,
};
use freecut_timeline::motion::PresetTable;
use freecut_timeline::plan::{AnimContext, PlanInputs, frame_plan_with};
use freecut_timeline::time::TimeRange;

#[test]
#[ignore = "benchmark"]
fn single_clip_ops_on_ten_thousand_main_track_clips() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let clips: Vec<NewClip> = (0..10_000)
        .map(|_| {
            NewClip::media(
                VIDEO_50,
                MediaKind::Video,
                TimeRange::new(f(0), f(2)).unwrap(),
            )
        })
        .collect();
    let req = EditRequest::Insert(InsertReq {
        seq: fx.seq_id(),
        target: InsertTarget::Track(main),
        at: f(0),
        clips,
    });
    let plan = fx.editor.plan(&fx.ctx(), &req).unwrap();
    fx.editor.commit(plan, Default::default()).unwrap();
    let first = fx.seq().main_track().unwrap().clips()[0].id;
    let mid = fx.seq().main_track().unwrap().clips()[5_000].id;

    let cases = [
        (
            "trim first clip (ripples 9,999)",
            EditRequest::Trim(TrimReq {
                seq: fx.seq_id(),
                clip: first,
                edge: Edge::Out,
                delta: f(-1),
                ripple: false,
                partners: Vec::new(),
            }),
        ),
        (
            "move middle clip to the start",
            EditRequest::Move(MoveReq {
                seq: fx.seq_id(),
                clips: vec![mid],
                delta: f(-10_000),
                track_shift: 0,
                followers: Vec::new(),
            }),
        ),
    ];
    for (name, req) in cases {
        let runs = 20;
        let t = Instant::now();
        for _ in 0..runs {
            std::hint::black_box(fx.editor.plan(&fx.ctx(), &req).unwrap());
        }
        let per = t.elapsed() / runs;
        println!("{name}: {per:?} per plan");
    }
}

#[test]
#[ignore = "benchmark"]
fn frame_plan_on_ten_thousand_animated_clips() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let video: Vec<NewClip> = (0..10_000)
        .map(|_| {
            NewClip::media(
                VIDEO_50,
                MediaKind::Video,
                TimeRange::new(f(0), f(2)).unwrap(),
            )
        })
        .collect();
    let seq = fx.seq_id();
    let insert = |target, clips| {
        EditRequest::Insert(InsertReq {
            seq,
            target,
            at: f(0),
            clips,
        })
    };
    let plan = fx
        .editor
        .plan(&fx.ctx(), &insert(InsertTarget::Track(main), video))
        .unwrap();
    fx.editor.commit(plan, Default::default()).unwrap();
    let text: Vec<NewClip> = (0..10_000)
        .map(|_| NewClip::text(TextClipData::with_content("Animated title"), f(4)))
        .collect();
    let plan = fx
        .editor
        .plan(&fx.ctx(), &insert(InsertTarget::NewTrack, text))
        .unwrap();
    fx.editor.commit(plan, Default::default()).unwrap();
    let texts: Vec<_> = fx
        .seq()
        .tracks()
        .flat_map(|(_, t)| t.clips().iter())
        .filter(|c| c.content.text().is_some())
        .map(|c| c.id)
        .collect();
    let mut wave = AnimationSpec::new(PresetId::new("fc.loop.wave"), f(30));
    wave.params.unit = Some(AnimUnit::Letter);
    for (slot, spec) in [
        (
            Slot::Entrance,
            AnimationSpec::new(PresetId::new("fc.in.slide"), f(1)),
        ),
        (
            Slot::Exit,
            AnimationSpec::new(PresetId::new("fc.out.fade"), f(1)),
        ),
        (Slot::Continuous, wave),
    ] {
        let req = EditRequest::SetClipAnimation {
            seq,
            clips: texts.clone(),
            slot,
            spec: Some(spec),
        };
        let plan = fx.editor.plan(&fx.ctx(), &req).unwrap();
        fx.editor.commit(plan, Default::default()).unwrap();
    }
    // Keys on the clips under the measured frames.
    for (frame, card) in [
        (5_000 * 4, PropGroup::Transform),
        (5_000 * 4 + 1, PropGroup::Text(TextCard::Outline)),
    ] {
        let req = EditRequest::EditKeys {
            seq,
            clips: texts.clone(),
            prop: PropPath::Transform(TransformProp::Position),
            edit: KeyEdit::KeyCard { card, at: f(frame) },
        };
        let plan = fx.editor.plan(&fx.ctx(), &req).unwrap();
        fx.editor.commit(plan, Default::default()).unwrap();
    }
    let table = PresetTable::samples();
    let ctx = AnimContext {
        presets: &table,
        text_bounds: None,
    };
    let none = BTreeSet::new();
    let inputs = PlanInputs { offline: &none };
    let runs: u32 = 2_000;
    let t = Instant::now();
    for i in 0..runs {
        // In, Loop and Out frames of the clips around the middle.
        let frame = 5_000 * 4 + i64::from(i % 4);
        std::hint::black_box(frame_plan_with(fx.doc(), seq, f(frame), &inputs, &ctx).unwrap());
    }
    let per = t.elapsed() / runs;
    println!("frame plan, 20,000 clips (10,000 animated text): {per:?} per plan");
    assert!(per.as_micros() < 2_000, "over the 2 ms budget: {per:?}");
}
