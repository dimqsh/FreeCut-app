//! Animated text through the whole pipeline (TX-021, TX-022, TX-023,
//! FX-018, FX-030): documents edited with `SetClipAnimation` and
//! `EditKeys`, planned with a preset catalogue
//! ([`Compositor::plan`]) and drawn by the compositor, as preview and
//! export draw them.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;

use common::BG;
use common::project::{Project, ink_box};
use freecut_render::cpu::{Compositor, Frame, NoImages, TextLayer};
use freecut_render::export::ExportRenderer;
use freecut_render::text::FontRegistry;
use freecut_timeline::edit::TextStyleEdit;
use freecut_timeline::export::ExportClock;
use freecut_timeline::ids::ClipId;
use freecut_timeline::model::{
    AnimOrder, AnimUnit, AnimationSpec, CanvasPos, EaseCurve, EaseDir, Fill, Interp, PerMille,
    PresetId, PropPath, PropValue, RefPx, Rgba8, Slot, TextClipData, TextPropPath, TransformProp,
    Travel,
};
use freecut_timeline::motion::{
    Channel, ChannelProp, ChannelUnit, Curve, ParamDefaults, Pivot, PresetKind, PresetManifest,
    PresetTable, samples,
};
use freecut_timeline::plan::{FramePlan, LayerSource, PlanInputs, PlanNode};
use freecut_timeline::time::{FrameRate, TICKS_PER_SECOND, Tick, TimeRange};

const W: u32 = 480;
const H: u32 = 270;

/// Frame `n` of the 30 fps sequence.
fn f(n: i64) -> Tick {
    Tick::from_raw(TICKS_PER_SECOND / 30 * n)
}

fn ms(v: i64) -> Tick {
    Tick::from_raw(TICKS_PER_SECOND / 1000 * v)
}

/// TX-056's Letter rise as a manifest: each letter starts 0.6·k em below
/// and transparent and rises to rest; 0.8 s, ease-out cubic.
fn letter_rise() -> PresetManifest {
    let ease_out = Interp::Ease {
        curve: EaseCurve::Cubic,
        dir: EaseDir::Out,
    };
    PresetManifest {
        id: PresetId::new("fc.in.letter_rise"),
        kind: PresetKind::Entrance,
        units: Some(vec![AnimUnit::Letter, AnimUnit::Word]),
        directions: None,
        default_duration: ms(800),
        easing: ease_out,
        pivot: Pivot::Anchor,
        channels: vec![
            Channel {
                property: ChannelProp::TranslateY,
                curve: Curve::Eased,
                amplitude: 0.6,
                unit: ChannelUnit::Em,
                scales_with_intensity: true,
            },
            Channel {
                property: ChannelProp::Opacity,
                curve: Curve::Eased,
                amplitude: -1.0,
                unit: ChannelUnit::Ratio,
                scales_with_intensity: false,
            },
        ],
        defaults: ParamDefaults {
            overlap: PerMille::new(600),
            ..ParamDefaults::default()
        },
    }
}

/// The built-in samples plus Letter rise.
fn presets() -> PresetTable {
    let mut all = samples();
    all.push(letter_rise());
    PresetTable::new(all)
}

/// A 480 × 270, 30 fps project with "HHHHH" (40 px: 160 ref px) over
/// `[0, len)` frames; the letters don't touch, so each owns its columns.
fn titled(len: i64) -> (Project, ClipId) {
    let mut p = Project::new(W, H, 30);
    let clip = p.add_text(TextClipData::default(), Tick::ZERO, f(len));
    p.set_content(clip, "HHHHH");
    p.style(clip, TextStyleEdit::Size(RefPx::from_px(160)));
    (p, clip)
}

fn spec(id: &str, d: Tick) -> AnimationSpec {
    AnimationSpec::new(PresetId::new(id), d)
}

struct Render {
    c: Compositor,
    table: PresetTable,
}

impl Render {
    fn new() -> Render {
        Render {
            c: Compositor::new(FontRegistry::bundled()),
            table: presets(),
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
        self.c.render(&plan, W, H, &mut NoImages)
    }
}

/// A letter of the still title classified in another frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Letter {
    /// As at rest (within 2/255).
    Rest,
    /// Neither: on its way.
    Moving,
    /// Nothing drawn.
    Blank,
}

/// Every letter of the still frame (a run of inked columns; the title's
/// letters don't touch), in reading order, as drawn in `frame`.
fn letters(frame: &Frame, still: &Frame) -> Vec<Letter> {
    let px = |f: &Frame, x: u32, y: u32| f.pixel(x, y).unwrap();
    let inked = |x: u32| (0..H).any(|y| px(still, x, y) != BG);
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
    runs.into_iter()
        .map(|(x0, x1)| {
            let cols = || (x0..x1).flat_map(|x| (0..H).map(move |y| (x, y)));
            if cols().all(|(x, y)| px(frame, x, y) == BG) {
                Letter::Blank
            } else if cols().all(|(x, y)| {
                let (a, b) = (px(frame, x, y), px(still, x, y));
                (0..4).all(|c| a[c].abs_diff(b[c]) <= 2)
            }) {
                Letter::Rest
            } else {
                Letter::Moving
            }
        })
        .collect()
}

fn only_background(frame: &Frame) -> bool {
    ink_box(frame, BG).is_none()
}

// --- Block animations: offsets in the matrix, opacity multiplied ---

#[test]
fn a_fade_in_starts_invisible_and_ends_on_the_still_frame() {
    let (mut p, clip) = titled(60);
    let mut r = Render::new();
    let still = r.frame(&p, f(30));
    p.animate(clip, Slot::Entrance, Some(spec("fc.in.fade", ms(500))));
    assert!(only_background(&r.frame(&p, f(0))));
    // Half way, the title is there but dimmer.
    let mid = r.frame(&p, f(7));
    let brightest = |fr: &Frame| fr.rgba.chunks(4).map(|c| c[0]).max().unwrap();
    assert!(brightest(&mid) > 0 && brightest(&mid) < brightest(&still));
    // From the end of its 15 frames on, the still frame.
    assert_eq!(r.frame(&p, f(15)), still);
    assert_eq!(r.frame(&p, f(30)), still);
}

#[test]
fn a_short_slide_in_starts_three_em_to_the_left() {
    let (mut p, clip) = titled(60);
    let mut r = Render::new();
    let still = r.frame(&p, f(30));
    let mut s = spec("fc.in.slide", ms(600));
    s.params.travel = Travel::Short;
    p.animate(clip, Slot::Entrance, Some(s));
    let rest = ink_box(&still, BG).unwrap();
    let start = ink_box(&r.frame(&p, f(0)), BG).unwrap();
    // 3·k em with k = 1 and a 40 px em: 120 px, the same height.
    assert!(
        (i64::from(start.0) - i64::from(rest.0) + 120).abs() <= 1,
        "{start:?} vs {rest:?}"
    );
    assert_eq!((start.1, start.3), (rest.1, rest.3));
    assert_eq!(r.frame(&p, f(18)), still);
}

// --- Per-unit animations: the adapter from the plan's TextAnimPlan ---

#[test]
fn letter_rise_reveals_letters_in_reading_order() {
    let (mut p, clip) = titled(60);
    let mut r = Render::new();
    let still = r.frame(&p, f(40));
    p.animate(
        clip,
        Slot::Entrance,
        Some(spec("fc.in.letter_rise", ms(800))),
    );
    // The plan hands the per-letter work to the renderer.
    let plan = r.plan(&p, f(5));
    let PlanNode::Layer(layer) = &plan.nodes[0];
    let text = TextLayer::from_plan(layer).unwrap();
    assert!(text.motion.is_some());
    assert_eq!(letters(&still, &still), vec![Letter::Rest; 5]);
    assert!(
        only_background(&r.frame(&p, f(0))),
        "every letter starts transparent"
    );
    let mut rest_before = 0;
    let mut saw_partial = false;
    for n in 1..24 {
        let cols = letters(&r.frame(&p, f(n)), &still);
        assert!(
            cols.windows(2).all(|w| w[0] <= w[1]),
            "frame {n}: letters at rest, then moving, then not yet shown: {cols:?}"
        );
        let rest = cols.iter().filter(|c| **c == Letter::Rest).count();
        assert!(rest >= rest_before, "frame {n}: letters never go back");
        rest_before = rest;
        saw_partial |= rest > 0 && cols.contains(&Letter::Blank);
    }
    assert!(
        saw_partial,
        "some frame shows the first letters but not the last"
    );
    // The In is 24 frames: from then on, the still frame.
    assert_eq!(r.frame(&p, f(24)), still);
}

#[test]
fn letter_rise_out_starts_at_rest_and_takes_the_first_letters_first() {
    let (mut p, clip) = titled(60);
    let mut r = Render::new();
    let still = r.frame(&p, f(10));
    // No Out manifest of its own: the entrance played backwards.
    p.animate(clip, Slot::Exit, Some(spec("fc.out.letter_rise", ms(800))));
    assert_eq!(r.frame(&p, f(36)), still, "the Out starts at rest");
    let mut saw_partial = false;
    for n in 37..60 {
        let cols = letters(&r.frame(&p, f(n)), &still);
        assert!(
            cols.windows(2).all(|w| w[0] >= w[1]),
            "frame {n}: first letters gone, then leaving, then at rest: {cols:?}"
        );
        saw_partial |= cols.contains(&Letter::Rest) && cols.contains(&Letter::Blank);
    }
    assert!(saw_partial);
}

#[test]
fn typewriter_shows_the_first_letter_on_the_first_frame() {
    let (mut p, clip) = titled(60);
    let mut r = Render::new();
    let still = r.frame(&p, f(50));
    let mut s = spec("fc.in.typewriter", ms(500));
    s.params.overlap = PerMille::ZERO;
    s.params.pace = Some(ms(100));
    p.animate(clip, Slot::Entrance, Some(s));
    let cols = letters(&r.frame(&p, f(0)), &still);
    assert_eq!(cols.first(), Some(&Letter::Rest));
    assert_eq!(cols.last(), Some(&Letter::Blank));
    assert!(!cols.contains(&Letter::Moving), "a step: shown or not");
    // Five letters at 0.1 s each: all shown after 0.5 s.
    assert_eq!(r.frame(&p, f(15)), still);
}

#[test]
fn a_wave_loop_moves_the_letters_and_comes_back_each_cycle() {
    let (mut p, clip) = titled(90);
    let mut r = Render::new();
    let still = r.frame(&p, f(0));
    let mut s = spec("fc.loop.wave", ms(1200));
    s.params.unit = Some(AnimUnit::Letter);
    s.params.overlap = PerMille::ZERO;
    s.params.order = AnimOrder::Forward;
    p.animate(clip, Slot::Continuous, Some(s));
    let first = r.frame(&p, f(0));
    assert_ne!(first, still, "the letters are spread over the wave");
    let quarter = r.frame(&p, f(9));
    assert_ne!(quarter, first);
    // Letters at different heights: taller ink than the still title, the
    // same width (the wave moves them up and down only).
    let (a, b) = (ink_box(&still, BG).unwrap(), ink_box(&quarter, BG).unwrap());
    assert!(b.3 - b.1 > a.3 - a.1, "{a:?} {b:?}");
    assert_eq!((b.0, b.2), (a.0, a.2));
    // One cycle is 36 frames.
    assert_eq!(r.frame(&p, f(36)), first);
    assert_eq!(r.frame(&p, f(72)), first);
    assert_eq!(r.frame(&p, f(45)), quarter);
}

/// TX-022's composition order through the frame plan: the keyed position
/// is in the layer's matrix, the per-letter rise in the text's own frame on
/// top of it. (A slide In and a per-letter preset can't play at once: one
/// slot plays at a time.)
#[test]
fn composition_keyed_position_then_per_letter_rise() {
    let (mut p, clip) = titled(60);
    let pos = PropPath::Transform(TransformProp::Position);
    p.key(
        clip,
        pos,
        f(0),
        PropValue::Position(CanvasPos::new(-250_000, 0)),
    );
    p.key(clip, pos, f(24), PropValue::Position(CanvasPos::new(0, 0)));
    let mut r = Render::new();
    let centred = ink_box(&r.frame(&p, f(30)), BG).unwrap();
    let keyed = r.frame(&p, f(12));
    let k = ink_box(&keyed, BG).unwrap();
    assert!(k.0 + 30 < centred.0, "{k:?} {centred:?}");
    p.animate(
        clip,
        Slot::Entrance,
        Some(spec("fc.in.letter_rise", ms(800))),
    );
    let both = r.frame(&p, f(12));
    let l = letters(&both, &keyed);
    assert_eq!(l.len(), 5);
    assert!(l.windows(2).all(|w| w[0] <= w[1]), "{l:?}");
    assert!(
        l.contains(&Letter::Rest) && l.contains(&Letter::Moving),
        "{l:?}"
    );
    // Letters still rising sit below their keyed place, never beside it.
    let b = ink_box(&both, BG).unwrap();
    assert_eq!(b.0, k.0);
    assert!(b.2 <= k.2 && b.3 > k.3, "{b:?} vs {k:?}");
}

// --- Keyframed text (TX-021) ---

#[test]
fn keyed_fill_colour_is_drawn_at_each_time() {
    let (mut p, clip) = titled(60);
    let fill = PropPath::Text(TextPropPath::FillColor);
    p.key(
        clip,
        fill,
        f(0),
        PropValue::Color(Rgba8::new(255, 0, 0, 255)),
    );
    p.key(
        clip,
        fill,
        f(30),
        PropValue::Color(Rgba8::new(0, 0, 255, 255)),
    );
    let mut r = Render::new();
    let plan = r.plan(&p, f(30));
    let PlanNode::Layer(layer) = &plan.nodes[0];
    let LayerSource::Text { data, .. } = &layer.source else {
        panic!("{layer:?}");
    };
    assert_eq!(data.style.fill, Fill::Solid(Rgba8::new(0, 0, 255, 255)));
    let dominant = |fr: &Frame| {
        let mut sum = [0u64; 3];
        for px in fr.rgba.chunks(4) {
            for c in 0..3 {
                sum[c] += u64::from(px[c]);
            }
        }
        sum
    };
    let red = dominant(&r.frame(&p, f(0)));
    assert!(red[0] > 0 && red[1] == 0 && red[2] == 0, "{red:?}");
    let blue = dominant(&r.frame(&p, f(30)));
    assert!(blue[2] > 0 && blue[0] == 0 && blue[1] == 0, "{blue:?}");
    let mid = dominant(&r.frame(&p, f(15)));
    assert!(mid[0] > 0 && mid[2] > 0, "{mid:?}");
}

#[test]
fn the_reveal_key_shows_letters_from_the_start() {
    let (mut p, clip) = titled(60);
    let mut r = Render::new();
    let still = r.frame(&p, f(45));
    p.key(
        clip,
        PropPath::Reveal,
        f(0),
        PropValue::Ratio(PerMille::ZERO),
    );
    p.key(
        clip,
        PropPath::Reveal,
        f(30),
        PropValue::Ratio(PerMille::ONE),
    );
    assert!(only_background(&r.frame(&p, f(0))));
    let cols = letters(&r.frame(&p, f(15)), &still);
    assert!(cols.windows(2).all(|w| w[0] <= w[1]), "{cols:?}");
    assert!(cols.contains(&Letter::Rest) && cols.contains(&Letter::Blank));
    assert!(!cols.contains(&Letter::Moving), "whole letters: {cols:?}");
    assert_eq!(r.frame(&p, f(30)), still);
}

// --- Preview and export ---

#[test]
fn export_frames_equal_preview_frames() {
    let (mut p, clip) = titled(90);
    let fill = PropPath::Text(TextPropPath::FillColor);
    p.key(
        clip,
        fill,
        f(0),
        PropValue::Color(Rgba8::new(255, 200, 0, 255)),
    );
    p.key(
        clip,
        fill,
        f(60),
        PropValue::Color(Rgba8::new(0, 160, 255, 255)),
    );
    p.animate(
        clip,
        Slot::Entrance,
        Some(spec("fc.in.letter_rise", ms(800))),
    );
    p.animate(clip, Slot::Continuous, Some(spec("fc.loop.wave", ms(1200))));
    let mut out = spec("fc.out.slide", ms(600));
    out.params.travel = Travel::Short;
    p.animate(clip, Slot::Exit, Some(out));

    let range = TimeRange::new(Tick::ZERO, f(90)).unwrap();
    let clock = ExportClock::new(range, FrameRate::new(30, 1).unwrap()).unwrap();
    let mut export = ExportRenderer::new(
        p.doc(),
        p.seq(),
        clock,
        (W, H),
        Compositor::new(FontRegistry::bundled()).with_threads(3),
        NoImages,
    )
    .unwrap()
    .with_presets(Arc::new(presets()));
    let mut preview = Render::new();
    let mut differ = 0;
    for k in [0, 4, 11, 23, 24, 40, 63, 72, 80, 89] {
        let exported = export.render_frame(k).unwrap().clone();
        let previewed = preview.frame(&p, f(k as i64));
        assert_eq!(exported, previewed, "frame {k}");
        differ += usize::from(ink_box(&exported, BG).is_some());
    }
    assert!(differ > 5, "the title is on screen");
    // Without presets, export plays keyframes only.
    let mut keys_only = ExportRenderer::new(
        p.doc(),
        p.seq(),
        clock,
        (W, H),
        Compositor::new(FontRegistry::bundled()),
        NoImages,
    )
    .unwrap();
    assert!(only_background(&preview.frame(&p, f(0))));
    assert!(!only_background(keys_only.render_frame(0).unwrap()));
}
