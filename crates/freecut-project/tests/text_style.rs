//! Format 5's text style (TX-005, TX-008 to TX-016, TX-054, TX-055): every
//! value survives save and open, a missing key means the factory style,
//! and hand-edited values are clamped or refused like the other text
//! values.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use common::*;
use freecut_project::{Extras, OpenError, OpenWarning, Opened, decode, encode};
use freecut_timeline::ids::ClipId;
use freecut_timeline::model::{
    BoxHeight, BoxShape, BubbleKind, CaseTransform, DeciDegrees, Fill, FillCenter, FillExtent,
    FillKind, GlowDirection, GradientStop, GradientStops, HAlign, Layer, PerMille, RadialGradient,
    Rgba8, Side, SignedPerMille, Stroke, StrokePosition, TextClipData, TextStyle, VAnchor,
};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

const TITLE: &str = "/document/sequences/0/visual/2/clips/0";
const CAPTION: &str = "/document/sequences/0/visual/2/clips/1";
const TITLE_PATH: &str = "document.sequences[0].visual[2].clips[0]";

fn sink_value() -> Value {
    let (doc, ids) = kitchen_sink();
    to_value(&encode(&doc, ids, &Extras::new()).unwrap())
}

fn open_value(v: &Value) -> Result<Opened, OpenError> {
    decode(&compact(v))
}

fn title(opened: &Opened) -> TextClipData {
    let clip = opened.document.clip(ClipId::from_raw(nz(43))).unwrap();
    TextClipData::clone(clip.content.text().unwrap())
}

fn style_mut<'a>(v: &'a mut Value, clip: &str) -> &'a mut Value {
    &mut v.pointer_mut(clip).unwrap()["content"]["style"]
}

/// Saves the kitchen sink with `text` as its title, opens it again, and
/// checks that nothing changed and nothing was repaired.
fn round_trip(text: TextClipData) {
    let (doc, ids) = kitchen_sink_with_title(text.clone());
    let bytes = encode(&doc, ids, &Extras::new()).unwrap();
    let opened = decode(&bytes).unwrap();
    assert!(opened.warnings.is_empty(), "{:?}", opened.warnings);
    assert_eq!(title(&opened), text);
    assert_eq!(opened.document, doc);
    let again = encode(&opened.document, opened.ids.next(), &opened.extras).unwrap();
    assert_eq!(again, bytes);
}

#[test]
fn the_kitchen_sink_title_writes_every_layer_and_its_settings() {
    let v = sink_value();
    let style = &v.pointer(TITLE).unwrap()["content"]["style"];
    assert_eq!(
        style["fill"],
        json!({"type": "linear-gradient", "angle": 1355, "stops": [
            {"at": 0, "color": "#FF8000C0"},
            {"at": 400, "color": "#FFFFFFFF"},
            {"at": 400, "color": "#0040FFFF"},
            {"at": 1000, "color": "#0A141E00"},
        ], "extent": "line"})
    );
    assert_eq!(
        (&style["underline"], &style["strike"], &style["case"]),
        (&json!(true), &json!(true), &json!("title"))
    );
    assert_eq!(style["letter_spacing"], -150);
    assert_eq!(
        style["outline"],
        json!({"on": true, "color": "#000000FF", "width": 120, "opacity": 800,
               "position": "center"})
    );
    assert_eq!(
        style["shadow"],
        json!({"on": true, "color": "#140028FF", "opacity": 650, "blur": 200,
               "distance": 150, "angle": 1355, "include_box": true})
    );
    assert_eq!(
        style["glow"],
        json!({"on": true, "color": "#00FFC8FF", "intensity": 900, "radius": 400,
               "offset": 50, "angle": 2700, "direction": "both"})
    );
    // Off, but changed: written with its settings kept (TX-054).
    assert_eq!(
        style["background"],
        json!({"on": false, "color": "#FFE600FF", "opacity": 550, "shape": "per-char",
               "pad_x": 100, "pad_y": 50, "roundness": 1000, "height": "lower-half",
               "border": {"on": true, "color": "#FF00FFFF", "width": 20}})
    );
    assert_eq!(
        style["bubble"],
        json!({"on": true, "kind": {"type": "arrow-tag", "point": "left"},
               "tail": {"on": false, "tip": {"x": -12345, "y": 25000}, "base": 300,
                        "bend": -450},
               "color": "#1E1E24E6", "opacity": 900, "pad_x": 500, "pad_y": 350,
               "roundness": 600,
               "border": {"on": false, "color": "#FF0000FF", "width": 80},
               "spikes": 20, "spike_depth": 250, "jitter": 700, "seed": 4000000000_u64,
               "lobe": 300, "trail": 4})
    );
    assert_eq!(style["curve"], json!({"on": true, "amount": -350}));
    let layout = &v.pointer(TITLE).unwrap()["content"]["layout"];
    assert_eq!(
        layout,
        &json!({"align": "right", "justify": true, "box_width": 72000,
                "line_spacing": 1250, "v_anchor": "bottom"})
    );
}

#[test]
fn a_factory_style_writes_no_layers() {
    let v = sink_value();
    let caption = &v.pointer(CAPTION).unwrap()["content"];
    assert_eq!(
        caption["style"],
        json!({"font": {"family": "Noto Sans"}, "size": 6400,
               "fill": {"type": "solid", "color": "#FFFFFFFF"},
               "bold": true, "italic": false, "underline": false, "strike": false,
               "case": "as-typed", "letter_spacing": 0})
    );
    assert_eq!(
        caption["layout"],
        json!({"align": "center", "justify": false, "box_width": null,
               "line_spacing": 1000, "v_anchor": "middle"})
    );
}

/// Absent keys mean the factory style *as of format 5*. If a later build
/// changes a factory default, this test fails: that change needs a format
/// step that writes the old value into older files (project-format.md,
/// migration rule 3).
#[test]
fn missing_keys_mean_the_format_5_factory_style() {
    let mut v = sink_value();
    *style_mut(&mut v, CAPTION) = json!({
        "font": {"family": "Noto Sans"}, "size": 6400,
        "fill": {"type": "solid", "color": "#FFFFFFFF"}, "bold": true, "italic": false,
        "underline": false, "strike": false, "case": "as-typed", "letter_spacing": 0,
        "outline": {"on": false, "color": "#000000FF", "width": 60, "opacity": 1000,
                    "position": "outside"},
        "shadow": {"on": false, "color": "#000000FF", "opacity": 500, "blur": 100,
                   "distance": 80, "angle": 450, "include_box": false},
        "glow": {"on": false, "color": "#FFFFFFFF", "intensity": 600, "radius": 250,
                 "offset": 0, "angle": 450, "direction": "outward"},
        "background": {"on": false, "color": "#000000FF", "opacity": 700, "shape": "block",
                       "pad_x": 250, "pad_y": 150, "roundness": 200, "height": "full",
                       "border": {"on": false, "color": "#FFFFFFFF", "width": 30}},
        "bubble": {"on": false, "kind": {"type": "rounded"},
                   "tail": {"on": true, "tip": {"x": -9000, "y": 13500}, "base": 240,
                            "bend": 200},
                   "color": "#FFFFFFFF", "opacity": 1000, "pad_x": 450, "pad_y": 300,
                   "roundness": 400, "border": {"on": true, "color": "#1E1E24FF", "width": 40},
                   "spikes": 14, "spike_depth": 180, "jitter": 300, "seed": 0, "lobe": 220,
                   "trail": 3},
        "curve": {"on": false, "amount": 0}
    });
    v.pointer_mut(CAPTION).unwrap()["content"]["layout"] = json!({"align": "center", "justify": false, "box_width": null, "line_spacing": 1000,
               "v_anchor": "middle"});
    let spelled = open_value(&v).unwrap();
    // The same text with every format-5 key left out.
    let mut bare = sink_value();
    *style_mut(&mut bare, CAPTION) = json!({
        "font": {"family": "Noto Sans"}, "size": 6400,
        "fill": {"type": "solid", "color": "#FFFFFFFF"}, "bold": true, "italic": false
    });
    bare.pointer_mut(CAPTION).unwrap()["content"]["layout"] =
        json!({"align": "center", "box_width": null, "line_spacing": 1000});
    let missing = open_value(&bare).unwrap();
    assert_eq!(spelled.document, missing.document);
    assert!(spelled.warnings.is_empty() && missing.warnings.is_empty());
    let caption = missing
        .document
        .clip(ClipId::from_raw(nz(51)))
        .unwrap()
        .content
        .text()
        .unwrap()
        .clone();
    assert_eq!(*caption, TextClipData::default());
    // Both save as the short form.
    let written = to_value(&encode(&spelled.document, spelled.ids.next(), &Extras::new()).unwrap());
    assert_eq!(written, sink_value());
}

#[test]
fn every_enum_value_round_trips() {
    let base = kitchen_title();
    let with = |f: &dyn Fn(&mut TextClipData)| {
        let mut t = base.clone();
        f(&mut t);
        round_trip(t);
    };
    for case in [
        CaseTransform::AsTyped,
        CaseTransform::Upper,
        CaseTransform::Lower,
        CaseTransform::Title,
        CaseTransform::Sentence,
    ] {
        with(&|t| t.style.case = case);
    }
    for position in [
        StrokePosition::Outside,
        StrokePosition::Center,
        StrokePosition::Inside,
    ] {
        with(&|t| t.style.stroke.value.position = position);
    }
    for direction in [
        GlowDirection::Outward,
        GlowDirection::Inward,
        GlowDirection::Both,
    ] {
        with(&|t| t.style.glow.value.direction = direction);
    }
    for shape in [BoxShape::Block, BoxShape::PerLine, BoxShape::PerChar] {
        for height in [BoxHeight::Full, BoxHeight::LowerHalf] {
            with(&|t| {
                t.style.background.value.shape = shape;
                t.style.background.value.height = height;
            });
        }
    }
    for kind in [
        BubbleKind::Rounded,
        BubbleKind::Oval,
        BubbleKind::Thought,
        BubbleKind::Shout,
        BubbleKind::Square,
        BubbleKind::Tag,
        BubbleKind::ArrowTag { point: Side::Left },
        BubbleKind::ArrowTag { point: Side::Right },
        BubbleKind::Ticket,
    ] {
        with(&|t| t.style.bubble.value.spec.kind = kind);
    }
    for align in [HAlign::Left, HAlign::Center, HAlign::Right] {
        for anchor in [VAnchor::Top, VAnchor::Middle, VAnchor::Bottom] {
            with(&|t| {
                t.layout.align = align;
                t.layout.v_anchor = anchor;
            });
        }
    }
    // Every fill kind and extent; the radial gradient with its centre.
    for kind in [
        FillKind::Solid,
        FillKind::LinearGradient,
        FillKind::RadialGradient,
    ] {
        with(&|t| t.style.fill = t.style.fill.with_kind(kind));
    }
    for extent in [FillExtent::Block, FillExtent::Line] {
        with(&|t| {
            t.style.fill = Fill::RadialGradient(RadialGradient {
                center: FillCenter {
                    x: PerMille::new(0),
                    y: PerMille::new(1000),
                },
                spread: RadialGradient::MAX_SPREAD,
                stops: GradientStops::new(
                    (0..8)
                        .map(|i| GradientStop {
                            at: PerMille::new(i * 125),
                            color: Rgba8::new(i as u8 * 30, 0, 255, 255),
                        })
                        .collect(),
                )
                .unwrap(),
                extent,
            });
        });
    }
    // Background on (and so the bubble off) with every layer switch flipped.
    with(&|t| {
        t.style.background.on = true;
        t.style.bubble.on = false;
        t.style.stroke.on = false;
        t.style.background.value.border.on = false;
        t.style.bubble.value.spec.tail.on = true;
        t.style.bubble.value.border.on = true;
        t.style.curve.on = false;
    });
}

#[test]
fn a_layer_switched_off_keeps_its_settings_in_the_file() {
    // Only the switch differs from the factory: still written, off.
    let mut text = TextClipData::default();
    text.style.stroke = Layer::off(Stroke {
        width: PerMille::new(200),
        ..Stroke::default()
    });
    text.style.curve = Layer::off(SignedPerMille::new(500));
    round_trip(text.clone());
    let (doc, ids) = kitchen_sink_with_title(text);
    let v = to_value(&encode(&doc, ids, &Extras::new()).unwrap());
    let style = &v.pointer(TITLE).unwrap()["content"]["style"];
    assert_eq!(style["outline"]["on"], false);
    assert_eq!(style["outline"]["width"], 200);
    assert_eq!(style["curve"], json!({"on": false, "amount": 500}));
    for absent in ["shadow", "glow", "background", "bubble"] {
        assert!(style.get(absent).is_none(), "{absent}");
    }
}

#[test]
fn a_file_with_both_background_and_bubble_on_opens_with_the_bubble() {
    let mut v = sink_value();
    style_mut(&mut v, TITLE)["background"]["on"] = json!(true);
    let opened = open_value(&v).unwrap();
    assert_eq!(
        opened.warnings,
        vec![OpenWarning::ValuesClamped { clips: 1 }]
    );
    assert!(opened.needs_save, "switching the box off is a repair");
    let text = title(&opened);
    assert!(text.style.bubble.on && !text.style.background.on);
    // The box keeps its settings, so switching it back restores them.
    assert_eq!(
        text.style.background.value,
        kitchen_title_style().background.value
    );
    // The repaired document saves and opens without warnings.
    let bytes = encode(&opened.document, opened.ids.next(), &opened.extras).unwrap();
    assert!(decode(&bytes).unwrap().warnings.is_empty());
}

/// Reads one value of a text, widened.
type Read = dyn Fn(&TextClipData) -> i64;

#[test]
fn out_of_range_style_values_are_clamped_as_a_repair() {
    let cases: Vec<(&str, Value, &Read, i64)> = vec![
        (
            "letter_spacing",
            json!(-999_999),
            &|t| t.style.letter_spacing.get().into(),
            -200,
        ),
        (
            "letter_spacing",
            json!(1_001),
            &|t| t.style.letter_spacing.get().into(),
            1000,
        ),
        (
            "outline/width",
            json!(5_000),
            &|t| t.style.stroke.value.width.get().into(),
            300,
        ),
        (
            "outline/opacity",
            json!(-3),
            &|t| t.style.stroke.value.opacity.get().into(),
            0,
        ),
        (
            "shadow/angle",
            json!(-450),
            &|t| t.style.shadow.value.angle.tenths().into(),
            3150,
        ),
        (
            "shadow/angle",
            json!(3_600),
            &|t| t.style.shadow.value.angle.tenths().into(),
            0,
        ),
        (
            "shadow/blur",
            json!(501),
            &|t| t.style.shadow.value.blur.get().into(),
            500,
        ),
        (
            "glow/intensity",
            json!(70_000),
            &|t| t.style.glow.value.intensity.get().into(),
            1000,
        ),
        (
            "glow/offset",
            json!(900),
            &|t| t.style.glow.value.offset.get().into(),
            500,
        ),
        (
            "background/pad_x",
            json!(1_200),
            &|t| t.style.background.value.pad_x.get().into(),
            1000,
        ),
        (
            "background/border/width",
            json!(300),
            &|t| t.style.background.value.border.value.width.get().into(),
            200,
        ),
        (
            "bubble/spikes",
            json!(300),
            &|t| t.style.bubble.value.spec.spikes.into(),
            32,
        ),
        (
            "bubble/spikes",
            json!(1),
            &|t| t.style.bubble.value.spec.spikes.into(),
            8,
        ),
        (
            "bubble/trail",
            json!(0),
            &|t| t.style.bubble.value.spec.trail.into(),
            2,
        ),
        (
            "bubble/lobe",
            json!(50),
            &|t| t.style.bubble.value.spec.lobe.get().into(),
            100,
        ),
        (
            "bubble/seed",
            json!(-1),
            &|t| t.style.bubble.value.spec.seed.into(),
            0,
        ),
        (
            "bubble/seed",
            json!(1_i64 << 40),
            &|t| t.style.bubble.value.spec.seed.into(),
            u32::MAX.into(),
        ),
        (
            "bubble/tail/bend",
            json!(99_999),
            &|t| t.style.bubble.value.spec.tail.value.bend.get().into(),
            1000,
        ),
        (
            "bubble/tail/base",
            json!(10),
            &|t| t.style.bubble.value.spec.tail.value.base.get().into(),
            100,
        ),
        (
            "bubble/tail/tip/x",
            json!(-10_000_000_000_i64),
            &|t| {
                t.style
                    .bubble
                    .value
                    .spec
                    .tail
                    .value
                    .tip
                    .x
                    .hundredths()
                    .into()
            },
            -4_000_000,
        ),
        (
            "curve/amount",
            json!(5_000),
            &|t| t.style.curve.value.get().into(),
            1000,
        ),
        (
            "fill/angle",
            json!(-10),
            &|t| match &t.style.fill {
                Fill::LinearGradient(g) => g.angle.tenths().into(),
                _ => -1,
            },
            3590,
        ),
    ];
    for (pointer, value, read, expected) in cases {
        let mut v = sink_value();
        *v.pointer_mut(&format!("{TITLE}/content/style/{pointer}"))
            .unwrap() = value.clone();
        let opened = open_value(&v).unwrap_or_else(|e| panic!("{pointer} = {value}: {e}"));
        assert_eq!(
            opened.warnings,
            vec![OpenWarning::ValuesClamped { clips: 1 }],
            "{pointer} = {value}"
        );
        assert_eq!(read(&title(&opened)), expected, "{pointer} = {value}");
        let bytes = encode(&opened.document, opened.ids.next(), &opened.extras).unwrap();
        assert!(decode(&bytes).unwrap().warnings.is_empty(), "{pointer}");
    }
}

#[test]
fn gradient_stops_out_of_place_are_clamped_and_sorted() {
    let mut v = sink_value();
    style_mut(&mut v, TITLE)["fill"]["stops"] = json!([
        {"at": 1500, "color": "#000000FF"},
        {"at": 200, "color": "#FFFFFFFF"},
        {"at": -4, "color": "#FF0000FF"},
    ]);
    let opened = open_value(&v).unwrap();
    assert_eq!(
        opened.warnings,
        vec![OpenWarning::ValuesClamped { clips: 1 }]
    );
    let stops: Vec<(u16, Rgba8)> = title(&opened)
        .style
        .fill
        .stops()
        .unwrap()
        .as_slice()
        .iter()
        .map(|s| (s.at.get(), s.color))
        .collect();
    assert_eq!(
        stops,
        vec![
            (0, Rgba8::new(255, 0, 0, 255)),
            (200, Rgba8::WHITE),
            (1000, Rgba8::BLACK),
        ]
    );
    // Equal positions in file order are a hard edge, not a repair.
    let opened = open_value(&sink_value()).unwrap();
    assert!(opened.warnings.is_empty());
}

#[test]
fn unknown_keys_stay_on_their_stop_when_stops_are_sorted() {
    let mut v = sink_value();
    style_mut(&mut v, TITLE)["fill"]["stops"] = json!([
        {"at": 800, "color": "#FF0000FF", "x_mid": "red-stop"},
        {"at": 200, "color": "#00FF00FF"},
        {"at": 500, "color": "#0000FFFF", "x_mid": "blue-stop"},
    ]);
    let opened = open_value(&v).unwrap();
    assert_eq!(
        opened.warnings,
        vec![
            OpenWarning::ValuesClamped { clips: 1 },
            OpenWarning::UnknownFields { objects: 2 },
        ]
    );
    let bytes = encode(&opened.document, opened.ids.next(), &opened.extras).unwrap();
    let mut saved = to_value(&bytes);
    assert_eq!(
        style_mut(&mut saved, TITLE)["fill"]["stops"],
        json!([
            {"at": 200, "color": "#00FF00FF"},
            {"at": 500, "color": "#0000FFFF", "x_mid": "blue-stop"},
            {"at": 800, "color": "#FF0000FF", "x_mid": "red-stop"},
        ])
    );
}

#[test]
fn malformed_style_values_are_refused_with_their_path() {
    let bad: Vec<(&str, Value, &str, &str)> = vec![
        (
            "fill/stops",
            json!([{"at": 0, "color": "#000000FF"}]),
            "content.style.fill.stops",
            "2 to 8",
        ),
        (
            "fill/stops/2/color",
            json!("blue"),
            "content.style.fill.stops[2].color",
            "#RRGGBBAA",
        ),
        (
            "outline/color",
            json!("#000"),
            "content.style.outline.color",
            "#RRGGBBAA",
        ),
        (
            "bubble/color",
            json!(""),
            "content.style.bubble.color",
            "#RRGGBBAA",
        ),
        (
            "bubble/border/color",
            json!("#GG0000FF"),
            "content.style.bubble.border.color",
            "#RRGGBBAA",
        ),
        (
            "background/border/color",
            json!("#00"),
            "content.style.background.border.color",
            "#RRGGBBAA",
        ),
    ];
    for (pointer, value, path, needle) in bad {
        let mut v = sink_value();
        *v.pointer_mut(&format!("{TITLE}/content/style/{pointer}"))
            .unwrap() = value;
        match open_value(&v) {
            Err(OpenError::BadValue { path: got, message }) => {
                assert_eq!(got, format!("{TITLE_PATH}.{path}"), "{pointer}");
                assert!(message.contains(needle), "{pointer}: {message}");
            }
            other => panic!("{pointer}: expected a bad value, got {other:?}"),
        }
    }
    let nine: Vec<Value> = (0..9)
        .map(|i| json!({"at": i * 100, "color": "#000000FF"}))
        .collect();
    let schema: Vec<(&str, Value, &str)> = vec![
        ("fill/stops", Value::Array(nine), "content.style.fill.stops"),
        (
            "fill/type",
            json!("conic-gradient"),
            "content.style.fill.type",
        ),
        ("fill/extent", json!("char"), "content.style.fill.extent"),
        ("fill/angle", json!(45.5), "content.style.fill.angle"),
        ("case", json!("shout"), "content.style.case"),
        ("underline", json!(1), "content.style.underline"),
        (
            "outline/position",
            json!("middle"),
            "content.style.outline.position",
        ),
        (
            "glow/direction",
            json!("up"),
            "content.style.glow.direction",
        ),
        (
            "background/shape",
            json!("circle"),
            "content.style.background.shape",
        ),
        (
            "background/height",
            json!("half"),
            "content.style.background.height",
        ),
        ("bubble/kind", json!("rounded"), "content.style.bubble.kind"),
        (
            "bubble/kind/type",
            json!("heart"),
            "content.style.bubble.kind.type",
        ),
        (
            "bubble/kind/point",
            json!("up"),
            "content.style.bubble.kind.point",
        ),
        (
            "bubble/tail/tip/y",
            json!("down"),
            "content.style.bubble.tail.tip.y",
        ),
        ("curve/on", json!(null), "content.style.curve.on"),
    ];
    for (pointer, value, path) in schema {
        let mut v = sink_value();
        *v.pointer_mut(&format!("{TITLE}/content/style/{pointer}"))
            .unwrap() = value;
        match open_value(&v) {
            // Nine stops are refused by the model, with their path.
            Err(OpenError::BadValue { path: got, .. } | OpenError::Json { path: got, .. }) => {
                assert_eq!(got, format!("{TITLE_PATH}.{path}"), "{pointer}");
            }
            other => panic!("{pointer}: expected an error, got {other:?}"),
        }
    }
    // A layer missing one of its keys names the layer.
    let mut v = sink_value();
    style_mut(&mut v, TITLE)["glow"]
        .as_object_mut()
        .unwrap()
        .remove("radius");
    match open_value(&v) {
        Err(OpenError::Json { path, message }) => {
            assert_eq!(path, format!("{TITLE_PATH}.content.style.glow"));
            assert!(message.contains("radius"), "{message}");
        }
        other => panic!("expected a JSON error, got {other:?}"),
    }
    // Layout keys added with format 5.
    for (key, value) in [("v_anchor", json!("center")), ("justify", json!("yes"))] {
        let mut v = sink_value();
        v.pointer_mut(TITLE).unwrap()["content"]["layout"][key] = value;
        match open_value(&v) {
            Err(OpenError::Json { path, .. }) => {
                assert_eq!(path, format!("{TITLE_PATH}.content.layout.{key}"));
            }
            other => panic!("{key}: expected a JSON error, got {other:?}"),
        }
    }
}

/// `roundtrip.rs`'s random documents (save, open, same bytes) really
/// exercise the format-5 style: some of them hold gradients, bubbles,
/// background boxes and layers switched off with changed settings.
#[test]
fn random_plans_reach_the_format_5_style() {
    use proptest::strategy::{Strategy, ValueTree};
    use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};
    let mut runner = TestRunner::new_with_rng(
        Config::default(),
        TestRng::deterministic_rng(RngAlgorithm::ChaCha),
    );
    let factory = TextStyle::default();
    let (mut gradients, mut bubbles, mut boxes, mut kept, mut layouts) = (0, 0, 0, 0, 0);
    for _ in 0..128 {
        let plan = doc_plan().new_tree(&mut runner).unwrap().current();
        let (doc, _) = build(&plan);
        for clip in doc
            .sequences()
            .flat_map(|s| s.tracks().map(|(_, t)| t))
            .flat_map(|t| t.clips())
        {
            let Some(text) = clip.content.text() else {
                continue;
            };
            let s = &text.style;
            gradients += usize::from(s.fill.stops().is_some());
            bubbles += usize::from(s.bubble.on);
            boxes += usize::from(s.background.on);
            kept += usize::from(
                (!s.glow.on && s.glow.value != factory.glow.value)
                    || (!s.shadow.on && s.shadow.value != factory.shadow.value),
            );
            layouts += usize::from(text.layout.justify || text.layout.v_anchor != VAnchor::Middle);
        }
    }
    assert!(
        gradients > 0 && bubbles > 0 && boxes > 0 && kept > 0 && layouts > 0,
        "{gradients} {bubbles} {boxes} {kept} {layouts}"
    );
}

#[test]
fn angles_are_stored_in_tenths_within_one_turn() {
    let mut text = kitchen_title();
    text.style.glow.value.angle = DeciDegrees::new(3_599);
    text.style.shadow.value.angle = DeciDegrees::ZERO;
    round_trip(text);
}
