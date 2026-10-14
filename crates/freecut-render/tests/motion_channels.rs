//! The animation engine's newer channels drawn through the whole pipeline
//! (TX-022, TX-023, TX-056, FX-030): documents edited with
//! `SetClipAnimation`, planned with a preset catalogue by the compositor
//! (as preview plans) and by the export renderer, and drawn.
//!
//! Per unit: Scatter (`random.offset`, `random.rotate`), Spread
//! (`toward_line_center`), Word rise (`line_rise`, clipped to the line),
//! Fall away, Wiggle (`jitter.rotate`), Sparkle (`sparkle.scale`) and
//! Shimmer (`style.lightness`); whole text: Shake (`jitter.*`) and Flicker
//! (`flicker.opacity`); whole clips: Blur in, Wipe in, Flash in, 3D flip in
//! (`turn_from_side`) and the Jitter shake loop. Loops ease in after an In
//! and out before an Out (TX-022's seam), and export draws the preview's
//! pixels.
//!
//! The manifests are written here as the bundled ones are
//! (`resources/text-animations`, `resources/animations`): this crate
//! doesn't load them.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;

use common::BG;
use common::project::{Project, ink_box};
use freecut_render::cpu::{Compositor, Frame, LayerImages, Picture, TextLayer};
use freecut_render::export::ExportRenderer;
use freecut_render::text::FontRegistry;
use freecut_render::text::animate::{GlyphAnim, UnitGeometry, UnitMap};
use freecut_timeline::edit::{
    EditRequest, InsertTarget, SplitReq, SplitTargets, TextStyleEdit, TransformEdit,
};
use freecut_timeline::export::ExportClock;
use freecut_timeline::ids::{ClipId, MediaId};
use freecut_timeline::model::{
    AnimOrder, AnimUnit, AnimationSpec, Direction, EaseCurve, EaseDir, Fill, Interp, MediaInfo,
    PerMille, PresetId, RefPx, Rgba8, Scale, SignedPerMille, Slot, TextClipData, TextLayerKind,
    VideoStreamInfo,
};
use freecut_timeline::motion::{
    Channel, ChannelProp, ChannelUnit, Curve, CurvePoint, ParamDefaults, Pivot, PresetKind,
    PresetManifest, PresetTable, StyleChannel, samples,
};
use freecut_timeline::plan::{FramePlan, LayerPlan, LayerSource, PlanInputs, PlanNode};
use freecut_timeline::time::{FrameRate, TICKS_PER_SECOND, Tick, TimeRange};

const W: u32 = 480;
const H: u32 = 270;

/// Frame `n` of the 30 fps sequence.
fn f(n: i64) -> Tick {
    Tick::from_raw(TICKS_PER_SECOND / 30 * n)
}

// --- The manifests (as bundled) ---

fn ease(curve: EaseCurve, dir: EaseDir) -> Interp {
    Interp::Ease { curve, dir }
}

fn points(p: &[(f64, f64)], eased: bool) -> Curve {
    Curve::Sampled {
        points: p.iter().map(|&(t, v)| CurvePoint { t, v }).collect(),
        eased,
    }
}

/// A loop channel's constant envelope.
fn held() -> Curve {
    points(&[(0.0, 1.0)], false)
}

fn ch(property: ChannelProp, curve: Curve, amplitude: f64, unit: ChannelUnit) -> Channel {
    Channel {
        property,
        curve,
        amplitude,
        unit,
        scales_with_intensity: !matches!(
            property,
            ChannelProp::Opacity | ChannelProp::LineRise | ChannelProp::TurnFromSide
        ),
    }
}

struct M {
    id: &'static str,
    kind: PresetKind,
    units: Option<Vec<AnimUnit>>,
    directions: Option<Vec<Direction>>,
    ms: i64,
    easing: Interp,
    overlap: u16,
    order: AnimOrder,
    channels: Vec<Channel>,
}

impl M {
    fn build(self) -> PresetManifest {
        PresetManifest {
            id: PresetId::new(self.id),
            kind: self.kind,
            units: self.units,
            directions: self.directions,
            default_duration: ms(self.ms),
            easing: self.easing,
            pivot: Pivot::Anchor,
            channels: self.channels,
            defaults: ParamDefaults {
                overlap: PerMille::new(self.overlap),
                order: self.order,
                pace: None,
            },
        }
    }
}

fn ms(v: i64) -> Tick {
    Tick::from_raw(TICKS_PER_SECOND / 1000 * v)
}

fn sides() -> Option<Vec<Direction>> {
    Some(vec![
        Direction::Left,
        Direction::Right,
        Direction::Up,
        Direction::Down,
    ])
}

fn manifests() -> Vec<PresetManifest> {
    use ChannelProp as P;
    use ChannelUnit as U;
    let quad_out = ease(EaseCurve::Quad, EaseDir::Out);
    let letters = || Some(vec![AnimUnit::Letter]);
    let fade_in = || ch(P::Opacity, Curve::Eased, -1.0, U::Ratio);
    vec![
        M {
            id: "fc.in.scatter",
            kind: PresetKind::Entrance,
            units: letters(),
            directions: None,
            ms: 1000,
            easing: quad_out,
            overlap: 800,
            order: AnimOrder::Random,
            channels: vec![
                fade_in(),
                ch(P::RandomOffset, Curve::Eased, 1.5, U::Em),
                ch(P::RandomRotate, Curve::Eased, 40.0, U::Deg),
            ],
        },
        M {
            id: "fc.in.spread",
            kind: PresetKind::Entrance,
            units: letters(),
            directions: None,
            ms: 600,
            easing: quad_out,
            overlap: 1000,
            order: AnimOrder::Forward,
            channels: vec![
                fade_in(),
                ch(P::TowardLineCenter, Curve::Eased, 0.6, U::Ratio),
            ],
        },
        M {
            id: "fc.in.word-rise",
            kind: PresetKind::Entrance,
            units: Some(vec![AnimUnit::Word, AnimUnit::Line]),
            directions: None,
            ms: 700,
            easing: ease(EaseCurve::Cubic, EaseDir::Out),
            overlap: 500,
            order: AnimOrder::Forward,
            channels: vec![ch(P::LineRise, Curve::Eased, 1.0, U::Ratio)],
        },
        M {
            id: "fc.out.fall-away",
            kind: PresetKind::Exit,
            units: Some(vec![AnimUnit::Letter, AnimUnit::Word]),
            directions: None,
            ms: 900,
            easing: ease(EaseCurve::Quad, EaseDir::In),
            overlap: 600,
            order: AnimOrder::Random,
            channels: vec![
                ch(P::TranslateY, Curve::Eased, 2.0, U::Em),
                ch(
                    P::Opacity,
                    points(&[(0.0, 0.0), (0.6, 0.0), (1.0, 1.0)], false),
                    -1.0,
                    U::Ratio,
                ),
                ch(P::RandomRotate, Curve::Eased, 30.0, U::Deg),
            ],
        },
        M {
            id: "fc.loop.shake",
            kind: PresetKind::Loop,
            units: None,
            directions: None,
            ms: 1000,
            easing: Interp::Linear,
            overlap: 500,
            order: AnimOrder::Forward,
            channels: vec![
                ch(P::JitterX, held(), 0.06, U::Em),
                ch(P::JitterY, held(), 0.06, U::Em),
            ],
        },
        M {
            id: "fc.loop.flicker",
            kind: PresetKind::Loop,
            units: None,
            directions: None,
            ms: 1000,
            easing: Interp::Linear,
            overlap: 500,
            order: AnimOrder::Forward,
            channels: vec![ch(P::FlickerOpacity, held(), 0.6, U::Ratio)],
        },
        M {
            id: "fc.loop.wiggle",
            kind: PresetKind::Loop,
            units: letters(),
            directions: None,
            ms: 1000,
            easing: Interp::Linear,
            overlap: 0,
            order: AnimOrder::Forward,
            channels: vec![ch(P::JitterRotate, held(), 6.0, U::Deg)],
        },
        M {
            id: "fc.loop.sparkle",
            kind: PresetKind::Loop,
            units: letters(),
            directions: None,
            ms: 2000,
            easing: Interp::Linear,
            overlap: 0,
            order: AnimOrder::Forward,
            channels: vec![ch(P::SparkleScale, held(), 0.2, U::Ratio)],
        },
        M {
            id: "fc.loop.shimmer",
            kind: PresetKind::Loop,
            units: letters(),
            directions: None,
            ms: 1500,
            easing: Interp::Linear,
            overlap: 0,
            order: AnimOrder::Forward,
            channels: vec![ch(
                P::Style(StyleChannel::Lightness),
                points(&[(0.0, 0.0), (0.1, 1.0), (0.2, 0.0), (1.0, 0.0)], true),
                0.4,
                U::Ratio,
            )],
        },
        M {
            id: "fc.clip.in.blur",
            kind: PresetKind::Entrance,
            units: None,
            directions: None,
            ms: 500,
            easing: quad_out,
            overlap: 500,
            order: AnimOrder::Forward,
            channels: vec![
                ch(P::Blur, Curve::Eased, 0.02, U::Se),
                ch(
                    P::Opacity,
                    points(&[(0.0, 1.0), (0.5, 0.0), (1.0, 0.0)], false),
                    -1.0,
                    U::Ratio,
                ),
            ],
        },
        M {
            id: "fc.clip.in.wipe",
            kind: PresetKind::Entrance,
            units: None,
            directions: sides(),
            ms: 500,
            easing: ease(EaseCurve::Sine, EaseDir::InOut),
            overlap: 500,
            order: AnimOrder::Forward,
            channels: vec![Channel {
                scales_with_intensity: false,
                ..ch(P::Reveal, Curve::Eased, -1.0, U::Ratio)
            }],
        },
        M {
            id: "fc.clip.in.flash",
            kind: PresetKind::Entrance,
            units: None,
            directions: None,
            ms: 400,
            easing: Interp::Linear,
            overlap: 500,
            order: AnimOrder::Forward,
            channels: vec![
                ch(
                    P::Tint,
                    points(&[(0.0, 1.0), (0.4, 0.0), (1.0, 0.0)], true),
                    1.0,
                    U::Ratio,
                ),
                ch(
                    P::Opacity,
                    points(&[(0.0, 1.0), (0.1, 0.0), (1.0, 0.0)], false),
                    -1.0,
                    U::Ratio,
                ),
            ],
        },
        M {
            id: "fc.clip.in.flip",
            kind: PresetKind::Entrance,
            units: None,
            directions: sides(),
            ms: 500,
            easing: quad_out,
            overlap: 500,
            order: AnimOrder::Forward,
            channels: vec![ch(P::TurnFromSide, Curve::Eased, 90.0, U::Deg)],
        },
        M {
            id: "fc.clip.loop.jitter",
            kind: PresetKind::Loop,
            units: None,
            directions: None,
            ms: 1000,
            easing: Interp::Linear,
            overlap: 500,
            order: AnimOrder::Forward,
            channels: vec![
                ch(P::JitterX, held(), 0.01, U::Se),
                ch(P::JitterY, held(), 0.01, U::Se),
                ch(P::JitterRotate, held(), 1.5, U::Deg),
            ],
        },
    ]
    .into_iter()
    .map(M::build)
    .collect()
}

fn presets() -> PresetTable {
    let mut all = samples();
    all.extend(manifests());
    PresetTable::new(all)
}

#[test]
fn the_manifests_here_are_well_formed() {
    for m in manifests() {
        assert_eq!(m.check(), Ok(()), "{}", m.id);
    }
}

// --- Projects, frames and poses ---

/// `id` for `d` with the manifest's own defaults for the per-unit
/// settings (as the app's grid applies them) and `seed`.
fn spec(id: &str, d: Tick, seed: u32) -> AnimationSpec {
    let mut s = AnimationSpec::new(PresetId::new(id), d);
    let table = presets();
    if let Some(m) = table.manifests().find(|m| m.id.as_str() == id) {
        s.params.overlap = m.defaults.overlap;
        s.params.order = m.defaults.order;
    }
    s.params.seed = seed;
    s
}

/// A 480 × 270, 30 fps project with `text` (40 px: 160 ref px) over
/// `[0, len)` frames.
fn titled(text: &str, len: i64) -> (Project, ClipId) {
    let mut p = Project::new(W, H, 30);
    let clip = p.add_text(TextClipData::default(), Tick::ZERO, f(len));
    p.set_content(clip, text);
    p.style(clip, TextStyleEdit::Size(RefPx::from_px(160)));
    (p, clip)
}

/// A 480 × 270 checkerboard of 12 px cells, red and green on blue.
fn checker() -> Picture {
    let mut rgba = Vec::with_capacity((W * H * 4) as usize);
    for y in 0..H {
        for x in 0..W {
            rgba.extend_from_slice(&if (x / 12 + y / 12) % 2 == 0 {
                [230, 40, 20, 255]
            } else {
                [20, 200, 60, 255]
            });
        }
    }
    Picture::new(W, H, rgba)
}

fn video_info(frames: i64) -> MediaInfo {
    let duration = f(frames);
    MediaInfo {
        container: "mp4".into(),
        default_video: Some(0),
        duration,
        video: vec![VideoStreamInfo {
            index: 0,
            codec: "h264".into(),
            width: W,
            height: H,
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

/// A 480 × 270, 30 fps project with a `len`-frame video of the
/// checkerboard on the main track, at 50 % in the middle of the canvas.
fn filmed(len: i64) -> (Project, ClipId, MediaId) {
    let mut p = Project::new(W, H, 30);
    let media = p.import("/media/checker.mp4", video_info(len));
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
        Tick::ZERO,
        media,
        TimeRange::new(Tick::ZERO, f(len)).unwrap(),
    );
    p.transform(clip, TransformEdit::Scale(Scale::from_basis_points(5000)));
    (p, clip, media)
}

/// The checkerboard for the video `media`, if any, and every text layer's
/// own text.
struct Images {
    media: Option<MediaId>,
    picture: Arc<Picture>,
}

impl LayerImages for Images {
    fn video(&mut self, asset: MediaId, _: Tick) -> Option<Arc<Picture>> {
        (Some(asset) == self.media).then(|| Arc::clone(&self.picture))
    }

    fn still(&mut self, _: MediaId) -> Option<Arc<Picture>> {
        None
    }
}

fn images(media: Option<MediaId>) -> Images {
    Images {
        media,
        picture: Arc::new(checker()),
    }
}

struct Render {
    c: Compositor,
    table: PresetTable,
    images: Images,
}

impl Render {
    fn new(media: Option<MediaId>) -> Render {
        Render {
            c: Compositor::new(FontRegistry::bundled()),
            table: presets(),
            images: images(media),
        }
    }

    fn plan(&mut self, p: &Project, t: Tick) -> FramePlan {
        let none = BTreeSet::new();
        self.c
            .plan(
                p.editor.document(),
                p.seq(),
                t,
                &PlanInputs { offline: &none },
                &self.table,
            )
            .unwrap()
    }

    fn frame(&mut self, p: &Project, t: Tick) -> Frame {
        let plan = self.plan(p, t);
        self.c.render(&plan, W, H, &mut self.images)
    }

    /// The text layer of the plan at `t`.
    fn text_layer(&mut self, p: &Project, t: Tick) -> (LayerPlan, TextLayer) {
        let plan = self.plan(p, t);
        let layer = layers(&plan)
            .into_iter()
            .find(|l| matches!(l.source, LayerSource::Text { .. }))
            .unwrap();
        let text = TextLayer::from_plan(&layer).unwrap();
        (layer, text)
    }

    /// Each timed unit's pose at `t` (reading order), as the renderer
    /// draws it; empty when the text is at rest.
    fn poses(&mut self, p: &Project, t: Tick) -> Vec<GlyphAnim> {
        let (_, text) = self.text_layer(p, t);
        let Some(motion) = text.motion else {
            return Vec::new();
        };
        let layout = self.c.text_renderer().layout(&text.spec);
        let state = motion.evaluate(&layout, &text.spec.text);
        let n = state.glyphs.iter().map(|g| g.unit + 1).max().unwrap_or(0);
        (0..n)
            .map(|u| state.glyphs.iter().find(|g| g.unit == u).unwrap().anim)
            .collect()
    }

    /// Each timed unit's place in its line, in em.
    fn geometry(&mut self, p: &Project, t: Tick, unit: AnimUnit) -> Vec<UnitGeometry> {
        let (_, text) = self.text_layer(p, t);
        let layout = self.c.text_renderer().layout(&text.spec);
        let anim_unit = match unit {
            AnimUnit::Letter => freecut_render::text::animate::AnimUnit::Grapheme,
            AnimUnit::Word => freecut_render::text::animate::AnimUnit::Word,
            AnimUnit::Line | AnimUnit::Block => freecut_render::text::animate::AnimUnit::Line,
        };
        let map = UnitMap::new(&layout, &text.spec.text, anim_unit);
        UnitGeometry::of_units(&map, &layout)
    }
}

fn layers(plan: &FramePlan) -> Vec<LayerPlan> {
    plan.nodes
        .iter()
        .filter_map(PlanNode::layer)
        .cloned()
        .collect()
}

fn only_background(frame: &Frame) -> bool {
    ink_box(frame, BG).is_none()
}

/// The column runs of the still title's letters (the title's letters
/// don't touch), in reading order.
fn letter_columns(still: &Frame) -> Vec<(u32, u32)> {
    let inked = |x: u32| (0..H).any(|y| still.pixel(x, y) != Some(BG));
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for x in 0..W {
        if !inked(x) {
            continue;
        }
        match runs.last_mut() {
            Some(run) if run.1 == x => run.1 = x + 1,
            _ => runs.push((x, x + 1)),
        }
    }
    runs
}

/// The brightness-weighted centre of everything drawn.
fn centroid(frame: &Frame) -> (f64, f64) {
    let (mut sx, mut sy, mut sw) = (0.0, 0.0, 0.0);
    for y in 0..H {
        for x in 0..W {
            let p = frame.pixel(x, y).unwrap();
            let w = f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2]);
            sx += f64::from(x) * w;
            sy += f64::from(y) * w;
            sw += w;
        }
    }
    (sx / sw, sy / sw)
}

/// The sum of every colour channel.
fn light(frame: &Frame) -> f64 {
    frame
        .rgba
        .chunks(4)
        .map(|p| f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2]))
        .sum()
}

/// Mean absolute difference per channel between two frames.
fn mean_diff(a: &Frame, b: &Frame) -> f64 {
    let sum: u64 = a
        .rgba
        .iter()
        .zip(&b.rgba)
        .map(|(x, y)| u64::from(x.abs_diff(*y)))
        .sum();
    sum as f64 / a.rgba.len() as f64
}

/// How much detail a frame has: the summed horizontal and vertical
/// steps of every channel.
fn sharpness(frame: &Frame) -> f64 {
    let mut s = 0.0;
    for y in 0..H - 1 {
        for x in 0..W - 1 {
            let p = frame.pixel(x, y).unwrap();
            let r = frame.pixel(x + 1, y).unwrap();
            let d = frame.pixel(x, y + 1).unwrap();
            for c in 0..3 {
                s += f64::from(p[c].abs_diff(r[c])) + f64::from(p[c].abs_diff(d[c]));
            }
        }
    }
    s
}

fn len(v: (f32, f32)) -> f32 {
    v.0.hypot(v.1)
}

// --- Per-unit channels on text ---

#[test]
fn scatter_moves_each_letter_its_own_seeded_way() {
    let (mut p, clip) = titled("HHHHH", 60);
    let mut r = Render::new(None);
    let still = r.frame(&p, f(45));
    let mut s = spec("fc.in.scatter", ms(1000), 7);
    s.params.overlap = PerMille::ONE;
    p.animate(clip, Slot::Entrance, Some(s.clone()));
    // All letters together: at 0.2 s each has its own way and turn.
    let poses = r.poses(&p, f(6));
    assert_eq!(poses.len(), 5);
    for (i, a) in poses.iter().enumerate() {
        assert!(len(a.offset) > 0.05 && len(a.offset) <= 1.5, "{i}: {a:?}");
        assert!(a.rotation.abs() <= 40.0, "{i}: {a:?}");
        for b in &poses[..i] {
            assert!(len((a.offset.0 - b.offset.0, a.offset.1 - b.offset.1)) > 1e-3);
            assert_ne!(a.rotation, b.rotation);
        }
    }
    // The letters set off in widely different directions.
    let angles: Vec<f32> = poses.iter().map(|a| a.offset.1.atan2(a.offset.0)).collect();
    let spread = angles
        .iter()
        .flat_map(|a| angles.iter().map(move |b| (a - b).abs()))
        .map(|d| d.min(std::f32::consts::TAU - d))
        .fold(0.0f32, f32::max);
    assert!(spread > 1.0, "{angles:?}");
    // The seeded motion follows the reading order, whatever the play
    // order (all letters play together here).
    let mut forward = s.clone();
    forward.params.order = AnimOrder::Forward;
    p.animate(clip, Slot::Entrance, Some(forward));
    assert_eq!(r.poses(&p, f(6)), poses);
    // Frames: transparent at first, scattered, then the still title.
    p.animate(clip, Slot::Entrance, Some(s.clone()));
    assert!(only_background(&r.frame(&p, f(0))));
    let mid = r.frame(&p, f(6));
    assert!(!only_background(&mid) && mid != still);
    assert_ne!(r.frame(&p, f(12)), mid);
    assert_eq!(r.frame(&p, f(30)), still);
    // The same seed draws the same frame anywhere; another seed doesn't.
    assert_eq!(Render::new(None).frame(&p, f(6)), mid);
    let mut other = s;
    other.params.seed = 8;
    p.animate(clip, Slot::Entrance, Some(other));
    assert_ne!(r.frame(&p, f(6)), mid);
}

#[test]
fn spread_gathers_each_line_toward_its_own_centre() {
    let (mut p, clip) = titled("HHH\nHHHHH", 60);
    let mut r = Render::new(None);
    let still = r.frame(&p, f(45));
    p.animate(clip, Slot::Entrance, Some(spec("fc.in.spread", ms(600), 1)));
    let geometry = r.geometry(&p, f(3), AnimUnit::Letter);
    assert_eq!(geometry.len(), 8);
    // The middle letter of each line sits at its line's centre.
    assert!(geometry[1].to_line_center.0.abs() < 0.05, "{geometry:?}");
    assert!(geometry[5].to_line_center.0.abs() < 0.05, "{geometry:?}");
    assert!(geometry[0].to_line_center.0 > 0.3 && geometry[7].to_line_center.0 < -0.3);
    // Every letter is pulled the same share of its way to its line's
    // centre, along x only (letters share their line's height).
    let poses = r.poses(&p, f(3));
    assert_eq!(poses.len(), 8);
    let share = poses[0].offset.0 / geometry[0].to_line_center.0;
    assert!(share > 0.05 && share <= 0.6, "{share}");
    for (a, g) in poses.iter().zip(&geometry) {
        assert!(
            (a.offset.0 - share * g.to_line_center.0).abs() < 1e-4,
            "{a:?} {g:?}"
        );
        assert!(a.offset.1.abs() < 1e-4 && g.to_line_center.1.abs() < 1e-4);
    }
    // Drawn: the title is narrower about the same centre, then at rest.
    let mid = r.frame(&p, f(3));
    let (a, b) = (ink_box(&still, BG).unwrap(), ink_box(&mid, BG).unwrap());
    assert!(b.0 > a.0 && b.2 < a.2, "{b:?} within {a:?}");
    assert!((i64::from(a.0 + a.2) - i64::from(b.0 + b.2)).abs() <= 2);
    assert_eq!(r.frame(&p, f(18)), still);
}

#[test]
fn word_rise_lifts_each_word_into_its_line_and_clips_it_there() {
    let (mut p, clip) = titled("HH HH\nHH HH", 60);
    let mut r = Render::new(None);
    let still = r.frame(&p, f(45));
    let rest = ink_box(&still, BG).unwrap();
    p.animate(
        clip,
        Slot::Entrance,
        Some(spec("fc.in.word-rise", ms(700), 1)),
    );
    // Each word starts one line height below, clipped while it moves.
    let geometry = r.geometry(&p, f(0), AnimUnit::Word);
    let poses = r.poses(&p, f(0));
    assert_eq!(poses.len(), 4);
    for (a, g) in poses.iter().zip(&geometry) {
        assert!(g.line_height > 1.0, "{g:?}");
        assert!((a.offset.1 - g.line_height).abs() < 1e-4, "{a:?} {g:?}");
        assert!(a.line_clip);
    }
    assert!(
        only_background(&r.frame(&p, f(0))),
        "below their lines, the words are cut away"
    );
    // The bottom of the last line's box, in canvas px: no word ever shows
    // below it (unclipped, the last words would rise from a line lower).
    let (layer, text) = r.text_layer(&p, f(0));
    let layout = r.c.text_renderer().layout(&text.spec);
    let last = layout.lines.last().unwrap();
    let px = layer.layer_to_canvas.x_axis.x as f32;
    let floor = rest.3 as f32 + (last.bottom - last.baseline) * px + 1.0;
    let mut saw_partial = false;
    for n in 1..21 {
        let fr = r.frame(&p, f(n));
        let Some(b) = ink_box(&fr, BG) else {
            continue;
        };
        assert!(b.3 as f32 <= floor, "frame {n}: {b:?}, floor {floor}");
        assert!(b.1 >= rest.1, "frame {n}: words rise from below");
        saw_partial |= fr != still;
    }
    assert!(saw_partial);
    // Rested words are not clipped.
    assert!(r.poses(&p, f(21)).is_empty() || r.poses(&p, f(21)).iter().all(|a| !a.line_clip));
    assert_eq!(r.frame(&p, f(21)), still);
}

/// On curved text (TX-016) the slot follows the arc: the clip is taken in
/// the text's flat frame, before the curve bends it.
#[test]
fn word_rise_clips_to_the_line_on_curved_text_too() {
    let (mut p, clip) = titled("HH HH HH", 60);
    p.style(
        clip,
        TextStyleEdit::SetLayer {
            layer: TextLayerKind::Curve,
            on: true,
        },
    );
    p.style(clip, TextStyleEdit::Curve(SignedPerMille::new(400)));
    let mut r = Render::new(None);
    let still = r.frame(&p, f(45));
    let rest = ink_box(&still, BG).unwrap();
    p.animate(
        clip,
        Slot::Entrance,
        Some(spec("fc.in.word-rise", ms(700), 1)),
    );
    assert!(only_background(&r.frame(&p, f(0))));
    let (layer, text) = r.text_layer(&p, f(0));
    let layout = r.c.text_renderer().layout(&text.spec);
    let line = &layout.lines[0];
    let px = layer.layer_to_canvas.x_axis.x as f32;
    let floor = rest.3 as f32 + (line.bottom - line.baseline) * px + 1.0;
    let mut saw_partial = false;
    for n in 1..21 {
        let fr = r.frame(&p, f(n));
        if let Some(b) = ink_box(&fr, BG) {
            assert!(b.3 as f32 <= floor, "frame {n}: {b:?}, floor {floor}");
            saw_partial |= fr != still;
        }
    }
    assert!(saw_partial);
    // The first word has all but landed by frame 10 (its window is
    // frames 0..10.5): drawn whole where the arch put it, not cut by a
    // straight line's box.
    let near = r.frame(&p, f(10));
    let third = rest.0 + (rest.2 - rest.0) / 4;
    for x in rest.0..third {
        for y in 0..H {
            let (a, b) = (near.pixel(x, y).unwrap(), still.pixel(x, y).unwrap());
            assert!(
                (0..3).all(|c| a[c].abs_diff(b[c]) <= 2),
                "({x}, {y}): {a:?} vs {b:?}"
            );
        }
    }
    assert_eq!(r.frame(&p, f(21)), still);
}

#[test]
fn fall_away_tips_each_letter_its_own_way() {
    let (mut p, clip) = titled("HHHHH", 60);
    let mut r = Render::new(None);
    let still = r.frame(&p, f(10));
    p.animate(clip, Slot::Exit, Some(spec("fc.out.fall-away", ms(900), 3)));
    // The Out (frames 33..60) starts at rest.
    assert_eq!(r.frame(&p, f(33)), still);
    let poses = r.poses(&p, f(52));
    assert_eq!(poses.len(), 5);
    let turned: Vec<f32> = poses.iter().map(|a| a.rotation).collect();
    assert!(turned.iter().all(|t| t.abs() <= 30.0), "{turned:?}");
    assert!(
        turned.iter().any(|&t| t > 0.5) && turned.iter().any(|&t| t < -0.5),
        "letters tip both ways: {turned:?}"
    );
    let frame = r.frame(&p, f(52));
    assert_ne!(frame, still);
    assert_eq!(Render::new(None).frame(&p, f(52)), frame);
}

#[test]
fn wiggle_turns_each_letter_on_its_own_and_repeats_each_cycle() {
    let (mut p, clip) = titled("HHHHH", 90);
    let mut r = Render::new(None);
    let still = r.frame(&p, f(0));
    // All letters in phase: what tells them apart is their seeded jitter.
    let mut s = spec("fc.loop.wiggle", ms(1000), 5);
    s.params.overlap = PerMille::ONE;
    p.animate(clip, Slot::Continuous, Some(s));
    let mut moved = 0;
    for n in [3, 10, 17, 24] {
        let poses = r.poses(&p, f(n));
        assert_eq!(poses.len(), 5);
        let turns: Vec<f32> = poses.iter().map(|a| a.rotation).collect();
        assert!(turns.iter().all(|t| t.abs() <= 6.0 + 1e-4), "{turns:?}");
        let distinct = turns
            .iter()
            .enumerate()
            .all(|(i, a)| turns[..i].iter().all(|b| (a - b).abs() > 1e-4));
        assert!(distinct, "frame {n}: {turns:?}");
        let frame = r.frame(&p, f(n));
        moved += usize::from(frame != still);
        assert_eq!(r.frame(&p, f(n + 30)), frame, "frame {n} a cycle later");
    }
    assert_eq!(moved, 4);
}

#[test]
fn sparkle_swells_each_letter_once_a_cycle_at_its_own_time() {
    let (mut p, clip) = titled("HHHHH", 90);
    let mut r = Render::new(None);
    // All letters in phase: each swells at its own seeded moment.
    let mut s = spec("fc.loop.sparkle", ms(2000), 9);
    s.params.overlap = PerMille::ONE;
    p.animate(clip, Slot::Continuous, Some(s));
    let mut peaks = vec![(0usize, 1.0f32); 5];
    let mut swollen = vec![0usize; 5];
    for n in 0..60 {
        let poses = r.poses(&p, f(n));
        for (u, a) in poses.iter().enumerate() {
            let s = a.scale * a.scale_xy.0;
            assert!((1.0 - 1e-4..=1.2 + 1e-4).contains(&s), "{u}: {a:?}");
            if s > peaks[u].1 {
                peaks[u] = (n as usize, s);
            }
            swollen[u] += usize::from(s > 1.0 + 1e-4);
        }
    }
    for (u, &(_, s)) in peaks.iter().enumerate() {
        assert!(s > 1.12, "letter {u} swells: {peaks:?}");
        // 0.2 s of a 2 s cycle: about 6 of 60 frames.
        assert!((3..=8).contains(&swollen[u]), "letter {u}: {swollen:?}");
    }
    let mut at: Vec<usize> = peaks.iter().map(|p| p.0).collect();
    at.dedup();
    assert!(at.len() >= 4, "letters swell at their own times: {peaks:?}");
    assert_eq!(r.frame(&p, f(70)), r.frame(&p, f(10)));
}

#[test]
fn the_shimmer_band_crosses_the_letters_in_reading_order() {
    let (mut p, clip) = titled("HHHHH", 90);
    p.style(
        clip,
        TextStyleEdit::Fill(Fill::Solid(Rgba8::new(255, 0, 0, 255))),
    );
    let mut r = Render::new(None);
    let still = r.frame(&p, f(0));
    let cols = letter_columns(&still);
    assert_eq!(cols.len(), 5);
    p.animate(
        clip,
        Slot::Continuous,
        Some(spec("fc.loop.shimmer", ms(1500), 1)),
    );
    // A letter is lit when its red fill moved toward white (green rises).
    let lit = |fr: &Frame, (x0, x1): (u32, u32)| {
        (x0..x1).any(|x| (0..H).any(|y| fr.pixel(x, y).unwrap()[1] > 20))
    };
    let mut first: Vec<Option<i64>> = vec![None; 5];
    for n in 0..45 {
        let fr = r.frame(&p, f(n));
        let on: Vec<bool> = cols.iter().map(|&c| lit(&fr, c)).collect();
        assert!(on.iter().filter(|b| **b).count() <= 2, "frame {n}: {on:?}");
        for (u, &o) in on.iter().enumerate() {
            if o && first[u].is_none() {
                first[u] = Some(n);
            }
        }
    }
    let first: Vec<i64> = first.into_iter().map(Option::unwrap).collect();
    assert!(first.windows(2).all(|w| w[0] < w[1]), "{first:?}");
    // The band is a tint: lit letters keep their shape.
    let fr = r.frame(&p, f(first[2]));
    assert_eq!(ink_box(&fr, BG), ink_box(&still, BG));
}

// --- Whole-text channels ---

#[test]
fn shake_jitters_the_whole_title_within_its_amplitude() {
    let (mut p, clip) = titled("HHHHH", 90);
    let mut r = Render::new(None);
    let still = r.frame(&p, f(0));
    let rest = centroid(&still);
    p.animate(
        clip,
        Slot::Continuous,
        Some(spec("fc.loop.shake", ms(1000), 2)),
    );
    // 0.06 em of a 40 px em: 2.4 px each way (text lands on whole
    // pixels).
    let (mut dx_max, mut dy_max) = (0.0f64, 0.0f64);
    for n in 0..30 {
        let fr = r.frame(&p, f(n));
        let (x, y) = centroid(&fr);
        let (dx, dy) = (x - rest.0, y - rest.1);
        assert!(dx.abs() <= 3.5 && dy.abs() <= 3.5, "frame {n}: {dx} {dy}");
        dx_max = dx_max.max(dx.abs());
        dy_max = dy_max.max(dy.abs());
        if n % 7 == 0 {
            assert_eq!(r.frame(&p, f(n + 30)), fr, "frame {n} a cycle later");
        }
    }
    assert!(dx_max > 0.8 && dy_max > 0.8, "in 2D: {dx_max} {dy_max}");
}

#[test]
fn flicker_dims_the_title_for_two_frames_at_least_a_second_apart() {
    let (mut p, clip) = titled("HHHHH", 150);
    let mut r = Render::new(None);
    let still = light(&r.frame(&p, f(0)));
    p.animate(
        clip,
        Slot::Continuous,
        Some(spec("fc.loop.flicker", ms(1000), 4)),
    );
    let dim: Vec<bool> = (0..150)
        .map(|n| {
            let k = light(&r.frame(&p, f(n))) / still;
            assert!(k > 0.35 && k < 1.0 + 1e-9, "frame {n}: {k}");
            if k < 0.9 {
                assert!((k - 0.4).abs() < 0.05, "a dip to 1 − 0.6: frame {n}: {k}");
            }
            k < 0.9
        })
        .collect();
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for (n, &d) in dim.iter().enumerate() {
        match runs.last_mut() {
            Some(run) if d && run.1 == n => run.1 = n + 1,
            _ if d => runs.push((n, n + 1)),
            _ => {}
        }
    }
    assert!(runs.len() >= 3, "{runs:?}");
    for run in &runs {
        assert_eq!(run.1 - run.0, 2, "two frames: {runs:?}");
    }
    for w in runs.windows(2) {
        assert!(w[1].0 - w[0].0 >= 30, "a second apart: {runs:?}");
    }
}

// --- TX-022's loop seams ---

#[test]
fn a_wave_loop_eases_in_after_the_in_and_out_before_the_out() {
    let (mut p, clip) = titled("HHHHH", 120);
    let mut r = Render::new(None);
    let still = r.frame(&p, f(60));
    p.animate(
        clip,
        Slot::Entrance,
        Some(AnimationSpec::new(PresetId::new("fc.in.fade"), ms(500))),
    );
    let mut wave = spec("fc.loop.wave", ms(1200), 1);
    wave.params.unit = Some(AnimUnit::Letter);
    p.animate(clip, Slot::Continuous, Some(wave));
    p.animate(
        clip,
        Slot::Exit,
        Some(AnimationSpec::new(PresetId::new("fc.out.fade"), ms(500))),
    );
    // The loop plays over frames 15..105: it starts and ends at rest.
    assert_eq!(r.frame(&p, f(15)), still, "no pop after the In");
    assert_eq!(r.frame(&p, f(105)), still, "no pop before the Out");
    // Every letter's height changes smoothly over both seams. Without the
    // envelope a letter would jump by up to the full ±0.25 em; with it, a
    // step is about as big as the wave's own steps (the 0.25 s ease is a
    // little quicker than the 1.2 s wave).
    let heights = |r: &mut Render, n: i64| -> Vec<f32> {
        let poses = r.poses(&p, f(n));
        if poses.is_empty() {
            vec![0.0; 5]
        } else {
            poses.iter().map(|a| a.offset.1).collect()
        }
    };
    let steps = |r: &mut Render, frames: std::ops::Range<i64>| -> f32 {
        let mut worst = 0.0f32;
        let mut before = heights(r, frames.start - 1);
        for n in frames {
            let now = heights(r, n);
            for (a, b) in now.iter().zip(&before) {
                worst = worst.max((a - b).abs());
            }
            before = now;
        }
        worst
    };
    let middle = steps(&mut r, 40..80);
    assert!(middle > 0.01, "the wave moves: {middle}");
    for seam in [steps(&mut r, 13..30), steps(&mut r, 90..108)] {
        assert!(seam <= 1.5 * middle && seam < 0.1, "{seam} vs {middle}");
    }
    // And in pixels: frame-to-frame change near the seams stays within
    // what the full wave changes per frame.
    let pixel_steps = |r: &mut Render, frames: std::ops::Range<i64>| -> f64 {
        let mut worst = 0.0f64;
        let mut before = r.frame(&p, f(frames.start - 1));
        for n in frames {
            let now = r.frame(&p, f(n));
            worst = worst.max(mean_diff(&now, &before));
            before = now;
        }
        worst
    };
    let full = pixel_steps(&mut r, 40..80);
    assert!(pixel_steps(&mut r, 16..22) <= 1.5 * full);
    assert!(pixel_steps(&mut r, 99..105) <= 1.5 * full);
}

/// A whole-text loop (the plan's block offsets) eases in and out too.
/// (Clips other than text keep either In and Out or a loop, FX-019.)
#[test]
fn a_whole_text_loop_eases_in_after_the_in_and_out_before_the_out() {
    let (mut p, clip) = titled("HHHHH", 90);
    let mut r = Render::new(None);
    let still = r.frame(&p, f(45));
    p.animate(
        clip,
        Slot::Entrance,
        Some(AnimationSpec::new(PresetId::new("fc.in.fade"), ms(500))),
    );
    p.animate(
        clip,
        Slot::Continuous,
        Some(spec("fc.loop.shake", ms(1000), 3)),
    );
    p.animate(
        clip,
        Slot::Exit,
        Some(AnimationSpec::new(PresetId::new("fc.out.fade"), ms(500))),
    );
    assert_eq!(r.frame(&p, f(15)), still, "the loop starts at rest");
    assert_eq!(r.frame(&p, f(75)), still, "and ends at rest");
    let moved = (16..75).filter(|&n| r.frame(&p, f(n)) != still).count();
    assert!(moved > 40, "{moved}");
}

// --- Whole-clip effects ---

#[test]
fn blur_in_sharpens_the_picture_to_rest() {
    let (mut p, clip, media) = filmed(60);
    let mut r = Render::new(Some(media));
    let still = r.frame(&p, f(30));
    p.animate(
        clip,
        Slot::Entrance,
        Some(spec("fc.clip.in.blur", ms(500), 1)),
    );
    // Opaque from half way (frame 7.5); blurred by 0.02 short edges
    // (5.4 px) at first, sharper every frame (about 1.2, 0.9 and 0.6 px
    // at frames 8, 9 and 10).
    let s: Vec<f64> = [8, 9, 10]
        .iter()
        .map(|&n| sharpness(&r.frame(&p, f(n))))
        .collect();
    assert!(s.windows(2).all(|w| w[0] < w[1]), "{s:?}");
    assert!(s[2] < sharpness(&still));
    // A blur moves light around without adding or losing it.
    let (a, b) = (light(&r.frame(&p, f(9))), light(&still));
    assert!((a / b - 1.0).abs() < 0.02, "{a} vs {b}");
    assert_eq!(r.frame(&p, f(15)), still);
}

#[test]
fn wipe_in_reveals_the_picture_from_the_chosen_side() {
    for (direction, shown, hidden) in [
        // (side, a pixel shown half way, a pixel still hidden)
        (Direction::Left, (150, 135), (330, 135)),
        (Direction::Right, (330, 135), (150, 135)),
        (Direction::Up, (240, 80), (240, 190)),
        (Direction::Down, (240, 190), (240, 80)),
    ] {
        let (mut p, clip, media) = filmed(60);
        let mut r = Render::new(Some(media));
        let still = r.frame(&p, f(30));
        let mut s = spec("fc.clip.in.wipe", ms(500), 1);
        s.direction = Some(direction);
        p.animate(clip, Slot::Entrance, Some(s));
        let mid = r.frame(&p, f(7));
        assert_eq!(
            mid.pixel(shown.0, shown.1),
            still.pixel(shown.0, shown.1),
            "{direction:?}"
        );
        assert_eq!(mid.pixel(hidden.0, hidden.1), Some(BG), "{direction:?}");
        assert!(only_background(&r.frame(&p, f(0))), "{direction:?}");
        assert_eq!(r.frame(&p, f(15)), still, "{direction:?}");
    }
}

#[test]
fn flash_in_starts_white_and_settles_on_the_picture() {
    let (mut p, clip, media) = filmed(60);
    let mut r = Render::new(Some(media));
    let still = r.frame(&p, f(30));
    p.animate(
        clip,
        Slot::Entrance,
        Some(spec("fc.clip.in.flash", ms(400), 1)),
    );
    // Opaque from 10 % of the 12 frames; white tint fading by 40 %.
    let early = r.frame(&p, f(2));
    let (a, b) = (
        early.pixel(240, 135).unwrap(),
        still.pixel(240, 135).unwrap(),
    );
    assert!((0..3).all(|c| a[c] >= b[c]), "{a:?} vs {b:?}");
    assert!(a[0].min(a[1]).min(a[2]) > 120, "near white: {a:?}");
    assert!(light(&early) > 1.5 * light(&still));
    let later = r.frame(&p, f(4));
    assert!(light(&later) < light(&early) && light(&later) > light(&still));
    // Outside the clip nothing is tinted.
    assert_eq!(early.pixel(10, 10), Some(BG));
    assert_eq!(r.frame(&p, f(12)), still);
}

#[test]
fn three_d_flip_in_turns_about_the_axis_of_its_side() {
    for direction in [
        Direction::Left,
        Direction::Right,
        Direction::Up,
        Direction::Down,
    ] {
        let (mut p, clip, media) = filmed(60);
        let mut r = Render::new(Some(media));
        let still = r.frame(&p, f(30));
        let rest = ink_box(&still, BG).unwrap();
        let mut s = spec("fc.clip.in.flip", ms(500), 1);
        s.direction = Some(direction);
        p.animate(clip, Slot::Entrance, Some(s));
        // A third of the way, turned about 40° in perspective.
        let plan = r.plan(&p, f(5));
        let m = layers(&plan)[0].layer_to_canvas;
        assert!(
            m.x_axis.z.abs() + m.y_axis.z.abs() > 1e-6,
            "{direction:?}: a projective matrix {m:?}"
        );
        let b = ink_box(&r.frame(&p, f(5)), BG).unwrap();
        let (w, h) = (b.2 - b.0, b.3 - b.1);
        let (rw, rh) = (rest.2 - rest.0, rest.3 - rest.1);
        match direction {
            // About the vertical axis: narrower, the near edge taller.
            Direction::Left | Direction::Right => {
                assert!(w + 20 < rw && h >= rh, "{direction:?}: {b:?} vs {rest:?}");
            }
            // About the horizontal axis: shorter, the near edge wider.
            _ => assert!(h + 10 < rh && w >= rw, "{direction:?}: {b:?} vs {rest:?}"),
        }
        assert_eq!(r.frame(&p, f(15)), still, "{direction:?}");
    }
    // Opposite sides turn opposite ways: mirrored frames.
    let turned = |d: Direction| {
        let (mut p, clip, media) = filmed(60);
        let mut s = spec("fc.clip.in.flip", ms(500), 1);
        s.direction = Some(d);
        p.animate(clip, Slot::Entrance, Some(s));
        let plan = Render::new(Some(media)).plan(&p, f(5));
        layers(&plan)[0].layer_to_canvas
    };
    let (l, rt) = (turned(Direction::Left), turned(Direction::Right));
    assert!((l.x_axis.z + rt.x_axis.z).abs() < 1e-9 && l.x_axis.z != 0.0);
}

#[test]
fn the_jitter_loop_moves_the_picture_a_little_and_repeats_each_cycle() {
    let (mut p, clip, media) = filmed(90);
    let mut r = Render::new(Some(media));
    let still = r.frame(&p, f(0));
    let rest = ink_box(&still, BG).unwrap();
    p.animate(
        clip,
        Slot::Continuous,
        Some(spec("fc.clip.loop.jitter", ms(1000), 6)),
    );
    let mut moved = 0;
    for n in [2, 9, 16, 23] {
        let fr = r.frame(&p, f(n));
        moved += usize::from(fr != still);
        // 0.01 short edges (2.7 px) and 1.5° (≈ 3.5 px at a corner of the
        // 240 × 135 picture).
        let b = ink_box(&fr, BG).unwrap();
        for (a, c) in [(b.0, rest.0), (b.1, rest.1), (b.2, rest.2), (b.3, rest.3)] {
            assert!(a.abs_diff(c) <= 7, "frame {n}: {b:?} vs {rest:?}");
        }
        assert_eq!(r.frame(&p, f(n + 30)), fr, "frame {n} a cycle later");
    }
    assert_eq!(moved, 4);
}

// --- Preview and export ---

#[test]
fn export_frames_equal_preview_frames_for_every_new_channel() {
    let len = 120;
    // Two pieces of the video: a 3D flip in and a wipe out, then the
    // jitter loop (a clip other than text keeps one kind, FX-019).
    let (mut p, clip, media) = filmed(len);
    let seq = p.seq();
    let second = p.apply(EditRequest::Split(SplitReq {
        seq,
        at: f(60),
        targets: SplitTargets::Clips(vec![clip]),
    }));
    let later = second
        .into_iter()
        .find(|&c| c != clip)
        .expect("the right piece");
    let mut flip = spec("fc.clip.in.flip", ms(500), 1);
    flip.direction = Some(Direction::Up);
    p.animate(clip, Slot::Entrance, Some(flip));
    p.animate(clip, Slot::Exit, Some(spec("fc.clip.out.wipe", ms(500), 1)));
    p.animate(
        later,
        Slot::Continuous,
        Some(spec("fc.clip.loop.jitter", ms(1000), 2)),
    );
    // A title over it: Scatter, Wiggle, Fall away.
    let title = p.add_text(TextClipData::default(), Tick::ZERO, f(60));
    p.set_content(title, "HEY YOU");
    p.style(title, TextStyleEdit::Size(RefPx::from_px(160)));
    p.animate(
        title,
        Slot::Entrance,
        Some(spec("fc.in.scatter", ms(600), 3)),
    );
    p.animate(
        title,
        Slot::Continuous,
        Some(spec("fc.loop.wiggle", ms(1000), 4)),
    );
    p.animate(
        title,
        Slot::Exit,
        Some(spec("fc.out.fall-away", ms(600), 5)),
    );
    // A second title: Word rise, Shimmer, a Spread out, Sparkle in a
    // later clip.
    let second = p.add_text(TextClipData::default(), f(60), f(60));
    p.set_content(second, "ONE TWO\nTHREE");
    p.style(second, TextStyleEdit::Size(RefPx::from_px(120)));
    p.animate(
        second,
        Slot::Entrance,
        Some(spec("fc.in.word-rise", ms(700), 1)),
    );
    p.animate(
        second,
        Slot::Continuous,
        Some(spec("fc.loop.shimmer", ms(1500), 1)),
    );
    p.animate(second, Slot::Exit, Some(spec("fc.out.spread", ms(600), 1)));

    let range = TimeRange::new(Tick::ZERO, f(len)).unwrap();
    let clock = ExportClock::new(range, FrameRate::new(30, 1).unwrap()).unwrap();
    let mut export = ExportRenderer::new(
        p.doc(),
        p.seq(),
        clock,
        (W, H),
        Compositor::new(FontRegistry::bundled()).with_threads(3),
        images(Some(media)),
    )
    .unwrap()
    .with_presets(Arc::new(presets()));
    let mut preview = Render::new(Some(media));
    let mut seen: Vec<Frame> = Vec::new();
    for k in [
        0u64, 3, 7, 14, 15, 22, 31, 45, 50, 58, 61, 66, 70, 80, 95, 100, 109, 119,
    ] {
        let exported = export.render_frame(k).unwrap().clone();
        let previewed = preview.frame(&p, f(k as i64));
        assert_eq!(exported, previewed, "frame {k}");
        if !seen.contains(&exported) {
            seen.push(exported);
        }
    }
    assert!(seen.len() > 12, "the frames move: {}", seen.len());
}
