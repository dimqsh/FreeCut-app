//! Adjustment clips on the CPU path (CL-006, README C52,
//! render-compositor.md §17): an `Adjust` node draws what is below it and
//! grades the frame in canvas space, background included; layers above are
//! untouched; stacked adjustments apply bottom-up; Mix 0 grades nothing;
//! the bytes don't depend on the thread count; and an adjustment clip
//! carrying a saved look over three clips and a title exports exactly as
//! previewed.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use std::collections::HashMap;
use std::num::NonZeroU64;
use std::path::PathBuf;
use std::sync::Arc;

use common::*;
use freecut_render::color::{CompiledStack, grade_in_place};
use freecut_render::cpu::{Compositor, LayerImages, Picture};
use freecut_render::text::FontRegistry;
use freecut_timeline::glam::{DMat3, DVec2};
use freecut_timeline::ids::{MediaId, TrackId};
use freecut_timeline::model::{ColorParamId, ColorStack, HslChannel, HueBand, WheelId};
use freecut_timeline::plan::{AdjustPlan, FramePlan, PlanNode};
use freecut_timeline::time::Tick;

const W: u32 = 64;
const H: u32 = 36;

fn adjust(color: ColorStack) -> PlanNode {
    PlanNode::Adjust(AdjustPlan {
        clip: clip(9),
        track: TrackId::from_raw(NonZeroU64::new(9).unwrap()),
        color: Arc::new(color),
        frame_index: 0,
    })
}

/// A still `n` filling the canvas.
fn full(n: u64) -> PlanNode {
    let mut l = layer(still(n), (W, H), DMat3::IDENTITY);
    l.clip = clip(n);
    PlanNode::Layer(l)
}

/// A solid `n` in the top-left quarter, on top.
fn corner(n: u64) -> PlanNode {
    let mut l = layer(
        still(n),
        (W / 2, H / 2),
        DMat3::from_translation(DVec2::ZERO),
    );
    l.clip = clip(n);
    PlanNode::Layer(l)
}

fn frame_plan(nodes: Vec<PlanNode>) -> FramePlan {
    FramePlan {
        nodes,
        ..plan(W, H, Vec::new())
    }
}

fn images() -> Images {
    Images::default()
        .with(1, pattern(W, H))
        .with(2, solid(W / 2, H / 2, GREEN))
}

fn render(p: &FramePlan, threads: usize) -> Vec<u8> {
    Compositor::new(FontRegistry::bundled())
        .with_threads(threads)
        .render(p, W, H, &mut images())
        .rgba
}

fn grey() -> ColorStack {
    ColorStack::default().with(ColorParamId::Saturation, -100)
}

fn warm() -> ColorStack {
    ColorStack::default().with(ColorParamId::Temperature, 100)
}

fn px(rgba: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * W + x) * 4) as usize;
    rgba[i..i + 4].try_into().unwrap()
}

#[test]
fn an_adjustment_grades_what_is_below_it_and_not_what_is_above() {
    let graded = render(&frame_plan(vec![full(1), adjust(grey()), corner(2)]), 2);
    // Below: every opaque pixel of the pattern turned grey.
    for (x, y) in [(40, 20), (63, 35), (33, 2), (5, 30)] {
        let [r, g, b, _] = px(&graded, x, y);
        assert!(r == g && g == b, "({x}, {y}) {r} {g} {b}");
    }
    // Above: the green corner is untouched.
    assert_eq!(px(&graded, 3, 3), GREEN);
    // The pattern alone, graded by hand, is what the adjustment made.
    let mut want = render(&frame_plan(vec![full(1)]), 2);
    grade_in_place(&CompiledStack::new(&grey()), &mut want, W, 1);
    let corner_free = |rgba: &[u8]| {
        (0..H)
            .flat_map(|y| (0..W).map(move |x| (x, y)))
            .filter(|(x, y)| *x >= W / 2 || *y >= H / 2)
            .map(|(x, y)| px(rgba, x, y))
            .collect::<Vec<_>>()
    };
    assert_eq!(corner_free(&graded), corner_free(&want));
}

#[test]
fn the_background_is_graded_too() {
    let mut p = frame_plan(vec![adjust(grey())]);
    p.canvas.background = [40, 80, 160, 255];
    let out = render(&p, 1);
    let [r, g, b, a] = px(&out, 10, 10);
    assert!(r == g && g == b && a == 255, "{r} {g} {b}");
    assert_ne!(r, 40);
    // Where nothing covers the frame in part of a layer's span: the
    // background there is graded, the layer too.
    let mut p = frame_plan(vec![corner(2), adjust(grey())]);
    p.canvas.background = [40, 80, 160, 255];
    let out = render(&p, 1);
    let [r, g, b, _] = px(&out, 50, 30);
    assert!(r == g && g == b);
    let [r, g, b, _] = px(&out, 3, 3);
    assert!(r == g && g == b, "the green below is graded");
}

#[test]
fn stacked_adjustments_apply_bottom_up() {
    let a_then_b = render(
        &frame_plan(vec![full(1), adjust(warm()), adjust(grey())]),
        2,
    );
    let b_then_a = render(
        &frame_plan(vec![full(1), adjust(grey()), adjust(warm())]),
        2,
    );
    assert_ne!(a_then_b, b_then_a, "the order matters");
    let mut want = render(&frame_plan(vec![full(1)]), 2);
    grade_in_place(&CompiledStack::new(&warm()), &mut want, W, 1);
    grade_in_place(&CompiledStack::new(&grey()), &mut want, W, 1);
    assert_eq!(a_then_b, want);
    // Grey last: neutral; warm last: warm.
    let [r, g, b, _] = px(&a_then_b, 40, 20);
    assert!(r == g && g == b);
    let [r, _, b, _] = px(&b_then_a, 40, 20);
    assert!(r > b);
}

#[test]
fn mix_zero_and_the_bypass_grade_nothing() {
    let plain = render(&frame_plan(vec![full(1), corner(2)]), 2);
    for stack in [
        grey().with(ColorParamId::Amount, 0),
        ColorStack {
            bypass: true,
            ..grey()
        },
        ColorStack::default(),
    ] {
        let p = frame_plan(vec![full(1), adjust(stack), corner(2)]);
        assert_eq!(render(&p, 2), plain);
    }
    // Mix 50 lies between.
    let half = render(
        &frame_plan(vec![full(1), adjust(grey().with(ColorParamId::Amount, 50))]),
        2,
    );
    let full_grey = render(&frame_plan(vec![full(1), adjust(grey())]), 2);
    let plain = render(&frame_plan(vec![full(1)]), 2);
    assert!(half != full_grey && half != plain);
}

#[test]
fn thread_counts_give_identical_bytes() {
    let stack = ColorStack::default()
        .with(ColorParamId::Contrast, 40)
        .with_puck(WheelId::Shadows, 300, -200)
        .with(
            ColorParamId::Hsl {
                band: HueBand::Blue,
                channel: HslChannel::Saturation,
            },
            -60,
        );
    let p = frame_plan(vec![full(1), adjust(stack), corner(2), adjust(warm())]);
    let one = render(&p, 1);
    for threads in [2, 3, 8] {
        assert_eq!(render(&p, threads), one, "{threads} threads");
    }
}

/// Checked-in PNG of FreeCut's pattern under a warm adjustment at Mix 60.
/// `FREECUT_BLESS_GOLDEN=1` rewrites it.
#[test]
fn an_adjustment_matches_its_golden_picture() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/color/adjust_warm_mix60.png");
    let stack = warm()
        .with(ColorParamId::Contrast, 30)
        .with(ColorParamId::Amount, 60);
    let out = render(&frame_plan(vec![full(1), adjust(stack), corner(2)]), 2);
    if std::env::var_os("FREECUT_BLESS_GOLDEN").is_some() {
        image::RgbaImage::from_raw(W, H, out.clone())
            .unwrap()
            .save(&path)
            .unwrap();
    }
    let want = image::open(&path)
        .unwrap_or_else(|e| panic!("{}: {e}; bless with FREECUT_BLESS_GOLDEN=1", path.display()))
        .to_rgba8();
    assert_eq!(want.as_raw(), &out);
}

/// Pictures by media id; text layers draw their own text (the default).
#[derive(Default, Clone)]
struct Footage(HashMap<MediaId, Arc<Picture>>);

impl LayerImages for Footage {
    fn video(&mut self, asset: MediaId, _: Tick) -> Option<Arc<Picture>> {
        self.0.get(&asset).cloned()
    }

    fn still(&mut self, asset: MediaId) -> Option<Arc<Picture>> {
        self.0.get(&asset).cloned()
    }
}

fn video_info(frames: i64, w: u32, h: u32) -> freecut_timeline::model::MediaInfo {
    use freecut_timeline::model::{MediaInfo, VideoStreamInfo};
    let duration = Tick::from_raw(frames * 705_600_000 / 30);
    MediaInfo {
        container: "mp4".into(),
        default_video: Some(0),
        duration,
        video: vec![VideoStreamInfo {
            index: 0,
            codec: "h264".into(),
            width: w,
            height: h,
            sar: (1, 1),
            rotation: Default::default(),
            mirrored: false,
            rate: None,
            variable_rate: false,
            has_alpha: false,
            bit_depth: 8,
            start: Tick::ZERO,
            duration,
        }],
        audio: Vec::new(),
        still: None,
    }
}

/// K4's acceptance: an adjustment clip spanning three clips, graded with a
/// saved look's sections and a keyed Mix, over a title too, renders the
/// same bytes in the preview's compositor and in the exporter on every
/// frame.
#[test]
fn an_adjustment_clip_with_a_look_over_three_clips_and_a_title_exports_as_previewed() {
    use freecut_render::export::ExportRenderer;
    use freecut_timeline::edit::{AdjustSpan, ColorEdit, EditRequest, InsertTarget};
    use freecut_timeline::export::ExportClock;
    use freecut_timeline::model::{
        PropPath, PropValue, Section, SectionMask, SectionSnapshot, TextClipData,
    };
    use freecut_timeline::time::{FrameRate, TimeRange};

    let f = |n: i64| Tick::from_raw(n * 705_600_000 / 30);
    let (w, h) = (160u32, 90u32);
    let mut p = common::project::Project::new(w, h, 30);
    let mut footage = Footage::default();
    let mut clips = Vec::new();
    for (i, picture) in [
        pattern(w, h),
        solid(w, h, [200, 120, 60, 255]),
        pattern(w / 2, h / 2),
    ]
    .into_iter()
    .enumerate()
    {
        let media = p.import(
            &format!("/media/{i}.mp4"),
            video_info(10, picture.width, picture.height),
        );
        footage.0.insert(media, Arc::new(picture));
        let main = p
            .editor
            .document()
            .sequence(p.seq())
            .unwrap()
            .main_track()
            .unwrap()
            .id;
        clips.push(p.place_video(
            InsertTarget::Track(main),
            f(10 * i as i64),
            media,
            TimeRange::new(Tick::ZERO, f(10)).unwrap(),
        ));
    }
    p.add_text(TextClipData::with_content("Graded"), f(0), f(30));
    let seq = p.seq();
    let adj = p.apply(EditRequest::AddAdjustmentClip {
        seq,
        span: AdjustSpan::Selection(clips.clone()),
        color: None,
    })[0];
    // A saved look: Light, Color, HSL and Wheels (its Amount isn't part of
    // it).
    let look = ColorStack::default()
        .with(ColorParamId::Exposure, 20)
        .with(ColorParamId::Contrast, 25)
        .with(ColorParamId::Temperature, 35)
        .with(ColorParamId::Saturation, -30)
        .with(
            ColorParamId::Hsl {
                band: HueBand::Orange,
                channel: HslChannel::Hue,
            },
            20,
        )
        .with_puck(WheelId::Shadows, -250, 300)
        .with(ColorParamId::Amount, 10);
    let snapshot = SectionSnapshot::new(
        &look,
        SectionMask::of(&[
            Section::Light,
            Section::Color,
            Section::Hsl,
            Section::Wheels,
        ]),
    );
    p.apply(EditRequest::EditColor {
        seq,
        clips: vec![adj],
        edit: ColorEdit::ApplySections(Arc::new(snapshot)),
    });
    let mix = PropPath::Color(ColorParamId::Amount);
    p.key(adj, mix, f(0), PropValue::Level(40));
    p.key(adj, mix, f(29), PropValue::Level(100));
    let plan = p.plan_at(f(15));
    assert!(
        matches!(plan.nodes.last(), Some(PlanNode::Adjust(a)) if a.clip == adj),
        "the adjustment clip sits above the title: {:?}",
        plan.nodes.iter().map(PlanNode::clip).collect::<Vec<_>>()
    );

    let clock = ExportClock::new(
        TimeRange::new(Tick::ZERO, f(30)).unwrap(),
        FrameRate::new(30, 1).unwrap(),
    )
    .unwrap();
    let mut export = ExportRenderer::new(
        p.doc(),
        seq,
        clock,
        (w, h),
        Compositor::new(FontRegistry::bundled()).with_threads(3),
        footage.clone(),
    )
    .unwrap();
    let mut preview = Compositor::new(FontRegistry::bundled()).with_threads(1);
    let mut preview_images = footage.clone();
    let mut ungraded = Compositor::new(FontRegistry::bundled()).with_threads(2);
    for k in [0u64, 5, 12, 21, 29] {
        let exported = export.render_frame(k).unwrap().clone();
        let t = clock.frame_time(k).unwrap();
        let plan = p.plan_at(t);
        let previewed = preview.render(&plan, w, h, &mut preview_images);
        assert_eq!(exported, previewed, "frame {k}");
        let mut bare = plan.clone();
        bare.nodes.retain(|n| !matches!(n, PlanNode::Adjust(_)));
        let plain = ungraded.render(&bare, w, h, &mut footage.clone());
        assert_ne!(exported, plain, "frame {k} is graded");
    }
}
