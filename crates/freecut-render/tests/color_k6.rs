//! Colour masks on the CPU path (CL-031, TR-017/018/020, README C52,
//! render-compositor.md §17): coverage per shape against golden pictures,
//! feather, invert and the combine modes; a zero-size mask hides the grade;
//! masks never touch alpha; a clip's masks follow its transform while an
//! adjustment clip's stay in canvas space; and a grade limited to a keyed
//! ellipse exports exactly as previewed.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use std::collections::HashMap;
use std::num::NonZeroU64;
use std::path::PathBuf;
use std::sync::Arc;

use common::*;
use freecut_render::color::{CompiledStack, grade, grade_displayed};
use freecut_render::cpu::{Compositor, LayerImages, Picture};
use freecut_render::shape::mask::{MaskSet, MaskSpace, combined, coverage};
use freecut_render::text::FontRegistry;
use freecut_timeline::glam::{DMat3, DVec2};
use freecut_timeline::ids::{MediaId, TrackId};
use freecut_timeline::model::{
    Angle, ColorMask, ColorParamId, ColorStack, Mask, MaskCombine, MaskPoint, MaskShape,
    MaskShapeKind, PerMille, Section,
};
use freecut_timeline::plan::{AdjustPlan, FramePlan, PlanNode};
use freecut_timeline::time::Tick;

const W: u32 = 64;
const H: u32 = 36;

fn mask(kind: MaskShapeKind) -> ColorMask {
    ColorMask::new(1, Mask::new(kind))
}

fn grey() -> ColorStack {
    ColorStack::default().with(ColorParamId::Saturation, -100)
}

fn with_masks(stack: ColorStack, masks: Vec<ColorMask>) -> ColorStack {
    ColorStack { masks, ..stack }
}

/// A soft, turned shape of every kind.
fn shapes() -> Vec<(&'static str, ColorMask)> {
    let mut ellipse = mask(MaskShapeKind::Ellipse);
    ellipse.mask.size = MaskPoint::new(9_000, 5_000);
    ellipse.mask.rotation = Angle::from_degrees(30);
    ellipse.mask.feather = PerMille::new(400);
    let mut rect = mask(MaskShapeKind::Rectangle);
    rect.mask.shape = MaskShape::Rectangle {
        roundness: PerMille::new(600),
    };
    rect.mask.center = MaskPoint::new(1_500, -1_000);
    rect.mask.rotation = Angle::from_degrees(-15);
    rect.mask.feather = PerMille::new(200);
    let mut band = mask(MaskShapeKind::Band);
    band.mask.shape = MaskShape::Band {
        width: PerMille::new(300),
    };
    band.mask.rotation = Angle::from_degrees(60);
    band.mask.feather = PerMille::new(300);
    let mut linear = mask(MaskShapeKind::Linear);
    linear.mask.center = MaskPoint::new(0, 2_000);
    linear.mask.rotation = Angle::from_degrees(-20);
    linear.mask.feather = PerMille::new(600);
    linear.expand = 30;
    vec![
        ("mask_ellipse", ellipse),
        ("mask_rectangle", rect),
        ("mask_band", band),
        ("mask_linear", linear),
    ]
}

/// The coverage of `masks` over a `W` × `H` canvas, as grey levels.
fn coverage_map(masks: &[ColorMask]) -> Vec<u8> {
    let space = MaskSpace::canvas(W, H);
    let set = MaskSet::new(masks);
    let mut out = Vec::with_capacity((W * H * 4) as usize);
    for y in 0..H {
        for x in 0..W {
            let c = set.coverage(space.point(x, y), space.pixel());
            let v = (c * 255.0).round() as u8;
            out.extend_from_slice(&[v, v, v, 255]);
        }
    }
    out
}

fn golden(name: &str, rgba: &[u8], w: u32, h: u32) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden/color")
        .join(format!("{name}.png"));
    if std::env::var_os("FREECUT_BLESS_GOLDEN").is_some() {
        image::RgbaImage::from_raw(w, h, rgba.to_vec())
            .unwrap()
            .save(&path)
            .unwrap();
    }
    let want = image::open(&path)
        .unwrap_or_else(|e| panic!("{}: {e}; bless with FREECUT_BLESS_GOLDEN=1", path.display()))
        .to_rgba8();
    assert_eq!(want.as_raw(), rgba, "{name}");
}

/// Checked-in PNGs of each shape's coverage. `FREECUT_BLESS_GOLDEN=1`
/// rewrites them.
#[test]
fn every_shape_matches_its_coverage_golden() {
    for (name, m) in shapes() {
        golden(name, &coverage_map(&[m]), W, H);
    }
}

/// Coverage never rises moving away from the shape (along rays from its
/// centre), at any feather; a wider feather is softer, never harder,
/// outside the edge.
#[test]
fn feather_is_monotonic() {
    let px = (720.0, 720.0);
    for (_, m) in shapes() {
        let (cx, cy) = m.mask.center.to_f64();
        // A line splits the item: walk across it, from the kept side out.
        let linear = m.mask.kind() == MaskShapeKind::Linear;
        let turn = m.mask.rotation.degrees().to_radians();
        for ray in 0..16 {
            let a = if linear {
                turn + std::f64::consts::FRAC_PI_2
            } else {
                f64::from(ray) * std::f64::consts::TAU / 16.0
            };
            let (cx, cy) = if linear {
                (cx - 1.0 * a.cos(), cy - 1.0 * a.sin())
            } else {
                (cx, cy)
            };
            for feather in [0u16, 250, 500, 1000] {
                let mut soft = m;
                soft.mask.feather = PerMille::new(feather);
                let mut last = f32::INFINITY;
                for step in 0..200 {
                    let r = f64::from(step) * 0.01;
                    let p = [(cx + r * a.cos()) as f32, (cy + r * a.sin()) as f32];
                    let c = coverage(&soft, px, p);
                    assert!((0.0..=1.0).contains(&c));
                    assert!(
                        c <= last + 1e-5,
                        "{:?} f{feather} ray {ray} r {r}: {c} > {last}",
                        m.mask.kind()
                    );
                    last = c;
                }
            }
        }
    }
}

#[test]
fn invert_is_the_complement() {
    let px = (300.0, 200.0);
    for (_, m) in shapes() {
        let mut inv = m;
        inv.mask.invert = true;
        for i in 0..400 {
            let p = [(i % 20) as f32 * 0.1 - 1.0, (i / 20) as f32 * 0.06 - 0.6];
            let (c, n) = (coverage(&m, px, p), coverage(&inv, px, p));
            assert!((c + n - 1.0).abs() < 1e-6, "{p:?}: {c} + {n}");
        }
    }
}

#[test]
fn masks_combine_in_list_order() {
    let px = (1000.0, 1000.0);
    let mut left = mask(MaskShapeKind::Ellipse);
    left.mask.center = MaskPoint::new(-1_500, 0);
    left.mask.size = MaskPoint::new(4_000, 4_000);
    left.mask.feather = PerMille::new(500);
    let right = ColorMask {
        id: 2,
        mask: Mask {
            center: MaskPoint::new(1_000, 0),
            ..left.mask
        },
        ..left
    };
    for i in 0..60 {
        let p = [i as f32 * 0.1 - 3.0, 0.05];
        let (a, b) = (coverage(&left, px, p), coverage(&right, px, p));
        for (mode, want) in [
            (MaskCombine::Add, a.max(b)),
            (MaskCombine::Subtract, a * (1.0 - b)),
            (MaskCombine::Intersect, a.min(b)),
        ] {
            let list = [left, ColorMask { mode, ..right }];
            assert!(
                (combined(&list, px, p) - want).abs() < 1e-6,
                "{mode:?} {p:?}"
            );
        }
    }
    // A list that starts by cutting out keeps the rest of the item.
    let cut = ColorMask {
        mode: MaskCombine::Subtract,
        ..left
    };
    assert_eq!(combined(&[cut], px, [-0.15, 0.0]), 0.0);
    assert_eq!(combined(&[cut], px, [2.0, 0.0]), 1.0);
    // Two of the coverage maps, as one golden.
    let mut a = left;
    a.mask.center = MaskPoint::new(-3_000, 0);
    a.mask.size = MaskPoint::new(7_000, 7_000);
    let mut b = right;
    b.mask.center = MaskPoint::new(2_000, 0);
    b.mask.size = MaskPoint::new(7_000, 7_000);
    let maps: Vec<Vec<u8>> = MaskCombine::ALL
        .iter()
        .map(|mode| coverage_map(&[a, ColorMask { mode: *mode, ..b }]))
        .collect();
    let mut sheet = Vec::new();
    for map in &maps {
        sheet.extend_from_slice(map);
    }
    golden("mask_combine_add_subtract_intersect", &sheet, W, H * 3);
}

fn picture_stack(stack: &ColorStack) -> CompiledStack {
    CompiledStack::new(stack)
}

#[test]
fn a_zero_size_mask_hides_the_grade_and_inverted_shows_it_all() {
    let p = pattern(W, H);
    let mut zero = mask(MaskShapeKind::Ellipse);
    zero.mask.size = MaskPoint::new(0, 0);
    zero.mask.feather = PerMille::ONE;
    let hidden = grade(&picture_stack(&with_masks(grey(), vec![zero])), &p, 2);
    assert_eq!(hidden, p);
    zero.mask.invert = true;
    let full = grade(&picture_stack(&with_masks(grey(), vec![zero])), &p, 2);
    assert_eq!(full, grade(&picture_stack(&grey()), &p, 2));
    // The Masks section switched off: the grade covers everything.
    let mut off = with_masks(grey(), vec![mask(MaskShapeKind::Ellipse)]);
    off.sections_enabled = off.sections_enabled.with(Section::Masks, false);
    assert_eq!(grade(&picture_stack(&off), &p, 1), full);
}

#[test]
fn inside_is_the_full_grade_outside_the_footage_and_alpha_never_changes() {
    let p = pattern(W, H);
    let stack = with_masks(grey(), vec![mask(MaskShapeKind::Ellipse)]);
    let out = grade(&picture_stack(&stack), &p, 3);
    let whole = grade(&picture_stack(&grey()), &p, 3);
    let at = |pic: &Picture, x: u32, y: u32| {
        let i = ((y * W + x) * 4) as usize;
        <[u8; 4]>::try_from(&pic.rgba[i..i + 4]).unwrap()
    };
    // The ellipse is 50 % of the 36 px height across, at the centre.
    assert_eq!(at(&out, 32, 18), at(&whole, 32, 18));
    assert_eq!(at(&out, 2, 2), at(&p, 2, 2));
    assert_eq!(at(&out, 60, 33), at(&p, 60, 33));
    for (a, b) in out.rgba.chunks(4).zip(p.rgba.chunks(4)) {
        assert_eq!(a[3], b[3], "alpha is kept");
    }
    // Amount scales the masked grade, and 1, 2 and 8 threads agree.
    let half = ColorStack {
        amount: 50,
        ..stack.clone()
    };
    let one = grade(&picture_stack(&half), &p, 1);
    for threads in [2, 8] {
        assert_eq!(grade(&picture_stack(&half), &p, threads), one);
    }
    assert_ne!(at(&one, 32, 18), at(&out, 32, 18));
    assert_ne!(at(&one, 32, 18), at(&p, 32, 18));
}

#[test]
fn a_clip_mask_lives_in_the_display_frame_whatever_the_decoded_size() {
    // A picture decoded at half size and one at full size, shown in the
    // same display frame, grade the same region.
    let stack = with_masks(grey(), vec![mask(MaskShapeKind::Rectangle)]);
    let c = picture_stack(&stack);
    let big = grade_displayed(&c, &solid(128, 72, [200, 60, 30, 255]), (128, 72), 2);
    let small = grade_displayed(&c, &solid(64, 36, [200, 60, 30, 255]), (128, 72), 2);
    let graded = |pic: &Picture, x: u32, y: u32| {
        let i = ((y * pic.width + x) * 4) as usize;
        pic.rgba[i] == pic.rgba[i + 1]
    };
    assert!(graded(&big, 64, 36) && graded(&small, 32, 18));
    assert!(!graded(&big, 20, 36) && !graded(&small, 10, 18));
    // The rectangle is 60 % × 40 % of the 72 px height: 43 × 29 px.
    assert!(graded(&big, 64 + 20, 36) && !graded(&big, 64 + 23, 36));
}

/// Pictures by media id.
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

fn render(nodes: Vec<PlanNode>, threads: usize) -> Vec<u8> {
    let p = FramePlan {
        nodes,
        ..plan(W, H, Vec::new())
    };
    let mut images = Images::default().with(1, solid(16, 16, [220, 90, 40, 255]));
    Compositor::new(FontRegistry::bundled())
        .with_threads(threads)
        .render(&p, W, H, &mut images)
        .rgba
}

fn px(rgba: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * W + x) * 4) as usize;
    rgba[i..i + 4].try_into().unwrap()
}

fn is_grey(p: [u8; 4]) -> bool {
    p[0] == p[1] && p[1] == p[2]
}

/// A 16 px picture-in-picture at `(x, y)`, graded inside a small ellipse.
fn pip(x: f64, y: f64, color: Option<ColorStack>) -> PlanNode {
    let mut l = layer(
        still(1),
        (16, 16),
        DMat3::from_translation(DVec2::new(x, y)),
    );
    l.color = color.map(Arc::new);
    PlanNode::Layer(l)
}

#[test]
fn clip_masks_follow_the_clip_and_adjustment_masks_stay_on_the_canvas() {
    let mut small = mask(MaskShapeKind::Ellipse);
    small.mask.size = MaskPoint::new(5_000, 5_000);
    let graded = with_masks(grey(), vec![small]);
    for (x, y) in [(4.0, 4.0), (40.0, 14.0)] {
        let out = render(vec![pip(x, y, Some(graded.clone()))], 2);
        let (cx, cy) = (x as u32 + 8, y as u32 + 8);
        assert!(
            is_grey(px(&out, cx, cy)),
            "the clip's centre is graded at {x}"
        );
        assert!(!is_grey(px(&out, cx - 7, cy - 7)), "its corner is not");
    }
    // An adjustment clip's mask is in canvas space: the canvas centre is
    // graded wherever the clip below sits, and the clip's own centre isn't.
    let adjust = PlanNode::Adjust(AdjustPlan {
        clip: clip(9),
        track: TrackId::from_raw(NonZeroU64::new(9).unwrap()),
        color: Arc::new(with_masks(grey(), vec![small])),
        frame_index: 0,
    });
    let out = render(vec![pip(24.0, 10.0, None), adjust.clone()], 1);
    assert!(is_grey(px(&out, 32, 18)));
    assert!(!is_grey(px(&out, 25, 11)));
    let out = render(vec![pip(2.0, 2.0, None), adjust], 3);
    assert!(
        !is_grey(px(&out, 10, 10)),
        "the moved clip's centre is outside"
    );
}

/// The acceptance: a grade limited to an ellipse keyed to follow a subject
/// renders the same bytes in the preview and the exporter, frame by frame,
/// and the ellipse moves.
#[test]
fn a_grade_limited_to_a_keyed_ellipse_exports_as_previewed() {
    use freecut_render::export::ExportRenderer;
    use freecut_timeline::edit::{ColorEdit, EditRequest, InsertTarget};
    use freecut_timeline::export::ExportClock;
    use freecut_timeline::model::{ColorMaskParam, MaskParam, PropPath, PropValue};
    use freecut_timeline::time::{FrameRate, TimeRange};

    let f = |n: i64| Tick::from_raw(n * 705_600_000 / 30);
    let (w, h) = (160u32, 90u32);
    let mut p = common::project::Project::new(w, h, 30);
    let mut footage = Footage::default();
    let media = p.import("/media/subject.mp4", video_info(30, w, h));
    footage.0.insert(media, Arc::new(pattern(w, h)));
    let main = p
        .editor
        .document()
        .sequence(p.seq())
        .unwrap()
        .main_track()
        .unwrap()
        .id;
    let clip = p.place_video(
        InsertTarget::Track(main),
        f(0),
        media,
        TimeRange::new(Tick::ZERO, f(30)).unwrap(),
    );
    let seq = p.seq();
    for edit in [
        ColorEdit::SetParams(vec![
            (ColorParamId::Exposure, 40),
            (ColorParamId::Saturation, -100),
        ]),
        ColorEdit::AddMask(Mask {
            feather: PerMille::new(300),
            ..Mask::new(MaskShapeKind::Ellipse)
        }),
    ] {
        p.apply(EditRequest::EditColor {
            seq,
            clips: vec![clip],
            edit,
        });
    }
    let center = PropPath::ColorMask {
        id: 1,
        param: ColorMaskParam::Shape(MaskParam::Center),
    };
    p.key(
        clip,
        center,
        f(0),
        PropValue::MaskPoint(MaskPoint::new(-6_000, -2_000)),
    );
    p.key(
        clip,
        center,
        f(29),
        PropValue::MaskPoint(MaskPoint::new(6_000, 2_000)),
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
    let mut images = footage.clone();
    let mut frames = Vec::new();
    for k in [0u64, 7, 15, 22, 29] {
        let exported = export.render_frame(k).unwrap().clone();
        let t = clock.frame_time(k).unwrap();
        let previewed = preview.render(&p.plan_at(t), w, h, &mut images);
        assert_eq!(exported, previewed, "frame {k}");
        frames.push(exported);
    }
    let at = |frame: &freecut_render::cpu::Frame, x: u32, y: u32| frame.pixel(x, y).unwrap();
    // The subject's spot is graded at each end of the move, not the other.
    let (left, right) = ((80 - 54, 45 - 18), (80 + 54, 45 + 18));
    let grey_at = |frame: &freecut_render::cpu::Frame, (x, y): (u32, u32)| {
        let [r, g, b, _] = at(frame, x, y);
        r.abs_diff(g) <= 1 && g.abs_diff(b) <= 1
    };
    assert!(grey_at(&frames[0], left) && !grey_at(&frames[0], right));
    assert!(grey_at(&frames[4], right) && !grey_at(&frames[4], left));
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

/// Checked-in PNG of FreeCut's pattern graded inside a soft ellipse and
/// outside a subtracted rectangle.
#[test]
fn a_masked_grade_matches_its_golden_picture() {
    let mut e = mask(MaskShapeKind::Ellipse);
    e.mask.size = MaskPoint::new(15_000, 9_000);
    e.mask.feather = PerMille::new(500);
    let mut r = ColorMask::new(2, Mask::new(MaskShapeKind::Rectangle));
    r.mask.size = MaskPoint::new(3_000, 3_000);
    r.mode = MaskCombine::Subtract;
    let stack = with_masks(
        ColorStack::default()
            .with(ColorParamId::Saturation, -100)
            .with(ColorParamId::Exposure, 30),
        vec![e, r],
    );
    let out = grade(&picture_stack(&stack), &pattern(W, H), 2);
    golden("mask_graded_ellipse_minus_square", &out.rgba, W, H);
}
