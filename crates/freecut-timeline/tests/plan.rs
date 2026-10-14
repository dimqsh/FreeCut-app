//! Frame plans: what is visible at a time and where it comes from (FX-001).
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;

use common::*;
use freecut_timeline::edit::{EditRequest, InsertReq, InsertTarget, NewClip, TransformEdit};
use freecut_timeline::glam::{DMat3, DVec2};
use freecut_timeline::ids::ClipId;
use freecut_timeline::model::{
    Angle, CanvasPos, GeneratedKind, PerMille, ProjectSettings, Scale, TextClipData, TrackFlags,
    TrackHeader, TrackSlot,
};
use freecut_timeline::plan::{LayerPlan, LayerSource, PlanInputs, PlanNode, frame_plan};
use pretty_assertions::assert_eq;

fn layers(fx: &Fixture, frame: i64) -> Vec<LayerPlan> {
    let none = BTreeSet::new();
    let plan = frame_plan(
        fx.doc(),
        fx.seq_id(),
        f(frame),
        &PlanInputs { offline: &none },
    )
    .unwrap();
    plan.nodes
        .into_iter()
        .filter_map(|n| match n {
            PlanNode::Layer(l) => Some(l),
            PlanNode::Adjust(_) => None,
        })
        .collect()
}

fn transform(fx: &mut Fixture, clip: ClipId, edits: &[TransformEdit]) {
    for edit in edits {
        fx.apply(EditRequest::SetTransform {
            seq: fx.seq_id(),
            clips: vec![clip],
            edit: *edit,
        })
        .unwrap();
    }
}

fn canvas(fx: &mut Fixture, width: u32, height: u32) {
    let settings = ProjectSettings {
        width,
        height,
        ..*fx.doc().settings()
    };
    fx.apply(EditRequest::SetProjectSettings {
        settings,
        conform: false,
    })
    .unwrap();
}

fn insert_text(fx: &mut Fixture, data: TextClipData) -> ClipId {
    let req = EditRequest::Insert(InsertReq {
        seq: fx.seq_id(),
        target: InsertTarget::FirstFree,
        at: f(10),
        clips: vec![NewClip::text(data, f(5))],
    });
    match fx.apply(req).unwrap() {
        freecut_timeline::Committed::Applied { selection, .. } => selection.unwrap()[0],
        freecut_timeline::Committed::NoOp => panic!("insert was a no-op"),
    }
}

#[track_caller]
fn assert_maps(m: &DMat3, from: (f64, f64), to: (f64, f64)) {
    let got = m.transform_point2(DVec2::new(from.0, from.1));
    assert!(
        (got - DVec2::new(to.0, to.1)).length() < 1e-6,
        "{from:?} maps to {got:?}, expected {to:?}"
    );
}

fn sources(
    fx: &Fixture,
    frame: i64,
    offline: &BTreeSet<freecut_timeline::ids::MediaId>,
) -> Vec<LayerSource> {
    let plan = frame_plan(fx.doc(), fx.seq_id(), f(frame), &PlanInputs { offline }).unwrap();
    plan.nodes
        .into_iter()
        .filter_map(|n| match n {
            PlanNode::Layer(l) => Some(l.source),
            PlanNode::Adjust(_) => None,
        })
        .collect()
}

#[test]
fn layers_come_bottom_up_with_exact_source_times() {
    let mut fx = Fixture::new();
    let main = fx.main();
    fx.insert(
        InsertTarget::Track(main),
        0,
        &[(VIDEO_100, 20, 10), (VIDEO_50, 5, 10)],
    );
    fx.insert(InsertTarget::NewTrack, 12, &[(VIDEO_50, 30, 4)]);
    let none = BTreeSet::new();
    assert_eq!(
        sources(&fx, 13, &none),
        vec![
            LayerSource::Video {
                asset: VIDEO_50,
                source_time: f(8)
            },
            LayerSource::Video {
                asset: VIDEO_50,
                source_time: f(31)
            },
        ]
    );
    assert_eq!(
        sources(&fx, 3, &none),
        vec![LayerSource::Video {
            asset: VIDEO_100,
            source_time: f(23)
        }]
    );
    assert_eq!(sources(&fx, 25, &none), vec![]);
}

#[test]
fn hidden_tracks_and_offline_media() {
    let mut fx = Fixture::new();
    let main = fx.main();
    fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)]);
    fx.insert(InsertTarget::NewTrack, 0, &[(VIDEO_50, 0, 10)]);
    let v1 = fx.track_at(TrackSlot::visual(1));
    fx.apply(EditRequest::SetTrackHeader {
        seq: fx.seq_id(),
        track: v1,
        header: TrackHeader {
            name: None,
            flags: TrackFlags {
                hidden: true,
                ..TrackFlags::default()
            },
        },
    })
    .unwrap();
    let offline = BTreeSet::from([VIDEO_100]);
    assert_eq!(
        sources(&fx, 2, &offline),
        vec![LayerSource::Slate { asset: VIDEO_100 }]
    );
}

#[test]
fn generated_items_carry_clip_local_time_and_fill_the_canvas() {
    let mut fx = Fixture::new();
    fx.apply(EditRequest::Insert(InsertReq {
        seq: fx.seq_id(),
        target: InsertTarget::NewTrack,
        at: f(10),
        clips: vec![NewClip::generated(GeneratedKind::Sticker, f(5))],
    }))
    .unwrap();
    let none = BTreeSet::new();
    let plan = frame_plan(fx.doc(), fx.seq_id(), f(12), &PlanInputs { offline: &none }).unwrap();
    let layer = plan.nodes[0].layer().unwrap();
    assert_eq!(
        layer.source,
        LayerSource::Generated {
            kind: GeneratedKind::Sticker,
            local_time: f(2)
        }
    );
    assert_eq!(layer.display_size, (1920, 1080));
    assert!(layer.layer_to_canvas.abs_diff_eq(DMat3::IDENTITY, 1e-12));
    assert_eq!(layer.opacity, 1.0);
    assert_eq!(plan.frame_index, 12);
}

#[test]
fn an_untransformed_picture_is_placed_by_the_fit_baseline() {
    let mut fx = Fixture::new();
    let main = fx.main();
    fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)]);
    // 1920×1080 footage on a 1920×1080 canvas: identity.
    let l = &layers(&fx, 2)[0];
    assert!(l.layer_to_canvas.abs_diff_eq(DMat3::IDENTITY, 1e-9));
    // On a portrait canvas it is letterboxed: 1080 × 607.5, centred.
    canvas(&mut fx, 1080, 1920);
    let l = &layers(&fx, 2)[0];
    assert_maps(&l.layer_to_canvas, (0.0, 0.0), (0.0, 656.25));
    assert_maps(&l.layer_to_canvas, (1920.0, 1080.0), (1080.0, 1263.75));
}

#[test]
fn picture_corners_land_after_flip_scale_rotation_and_position() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let clip = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)])[0];
    transform(
        &mut fx,
        clip,
        &[
            TransformEdit::FlipH(true),
            TransformEdit::Scale(Scale::from_percent(50)),
            TransformEdit::Rotation(Angle::from_degrees(90)),
            // A quarter canvas width right: the anchor goes to (1440, 540).
            TransformEdit::Position(CanvasPos::new(250_000, 0)),
            TransformEdit::Opacity(PerMille::new(250)),
        ],
    );
    let l = &layers(&fx, 2)[0];
    assert_eq!(l.opacity, 0.25);
    let m = &l.layer_to_canvas;
    // Top-left (-960, -540 from the centre): flipped to (960, -540), halved
    // to (480, -270), turned 90° clockwise to (270, 480), moved.
    assert_maps(m, (0.0, 0.0), (1710.0, 1020.0));
    // Top-right (960, -540) → (-960, -540) → (-480, -270) → (270, -480).
    assert_maps(m, (1920.0, 0.0), (1710.0, 60.0));
    // Bottom-right (960, 540) → (-960, 540) → (-480, 270) → (-270, -480).
    assert_maps(m, (1920.0, 1080.0), (1170.0, 60.0));
    // The centre stays the anchor.
    assert_maps(m, (960.0, 540.0), (1440.0, 540.0));
    // The inverse maps canvas points back for hit-testing (TR-007).
    let back = m.inverse().transform_point2(DVec2::new(1710.0, 1020.0));
    assert!(back.length() < 1e-6);
}

#[test]
fn letterboxed_footage_scales_about_its_fitted_centre() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let clip = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)])[0];
    canvas(&mut fx, 1080, 1920);
    transform(
        &mut fx,
        clip,
        &[
            TransformEdit::Scale(Scale::from_percent(200)),
            TransformEdit::FlipV(true),
        ],
    );
    let m = &layers(&fx, 2)[0].layer_to_canvas;
    // Fitted 1080 × 607.5 doubles to 2160 × 1215 about (540, 960); the
    // vertical flip sends the top-left corner to the bottom-left.
    assert_maps(m, (0.0, 0.0), (-540.0, 1567.5));
    assert_maps(m, (1920.0, 1080.0), (1620.0, 352.5));
}

#[test]
fn text_layers_carry_their_data_and_a_centred_matrix() {
    let mut fx = Fixture::new();
    let data = TextClipData::with_content("Title");
    let clip = insert_text(&mut fx, data.clone());
    let l = &layers(&fx, 12)[0];
    assert_eq!(
        l.source,
        LayerSource::Text {
            data: Arc::new(data.clone()),
            scale_px: 1.0
        }
    );
    // The text box centre sits at the canvas centre.
    assert_maps(&l.layer_to_canvas, (0.0, 0.0), (960.0, 540.0));
    assert_maps(&l.layer_to_canvas, (-100.0, 20.0), (860.0, 560.0));

    transform(
        &mut fx,
        clip,
        &[
            TransformEdit::Position(CanvasPos::new(-250_000, 250_000)),
            TransformEdit::Scale(Scale::from_percent(300)),
            TransformEdit::Rotation(Angle::from_degrees(-90)),
            TransformEdit::FlipH(true),
        ],
    );
    let l = &layers(&fx, 12)[0];
    // Anchor (480, 810). (10, 0) → flip (-10, 0) → ×3 (-30, 0) → 90°
    // counter-clockwise on screen (0, 30) → (480, 840).
    assert_maps(&l.layer_to_canvas, (10.0, 0.0), (480.0, 840.0));
    assert_maps(&l.layer_to_canvas, (0.0, 0.0), (480.0, 810.0));
}

#[test]
fn text_scale_follows_the_canvas_short_edge() {
    let mut fx = Fixture::new();
    insert_text(&mut fx, TextClipData::default());
    for (w, h, expected) in [
        (3840, 2160, 2.0),
        (1080, 1920, 1.0),
        (720, 1280, 720.0 / 1080.0),
    ] {
        canvas(&mut fx, w, h);
        let l = &layers(&fx, 12)[0];
        let LayerSource::Text { scale_px, .. } = l.source else {
            panic!("not a text layer: {:?}", l.source);
        };
        assert!((scale_px - expected).abs() < 1e-12);
        assert_eq!(l.display_size, (w, h));
        assert_maps(
            &l.layer_to_canvas,
            (0.0, 0.0),
            (f64::from(w) / 2.0, f64::from(h) / 2.0),
        );
    }
}
