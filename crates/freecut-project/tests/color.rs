//! Colour stacks in the project file (format 10, CL-001; format 11's HSL,
//! Curves and Wheels, CL-013 to CL-015; format 12's LUT slot and skin
//! protection; format 13's Auto and Match, CL-019, CL-020): written with
//! every default left out, read back exactly, values outside a slider's
//! range clamped as a repair, and stacks on clips that can't carry one
//! refused at their path.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use common::*;
use freecut_project::{Extras, OpenError, OpenWarning, decode, encode};
use freecut_timeline::CommitMeta;
use freecut_timeline::edit::{ColorEdit, EditCtx, EditRequest};
use freecut_timeline::ids::ClipId;
use freecut_timeline::model::{
    AutoSource, AutoStyle, ColorMask, ColorParamId, ColorStack, CurveChannel, CurvePoint,
    HslChannel, HueBand, Mask, MaskPoint, MaskShape, MaskShapeKind, MatchRef, PerMille, PropPath,
    PropValue, Section, SkinProtect, WheelId,
};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

const INTRO: &str = "/document/sequences/0/visual/0/clips/0";
const TITLE: &str = "/document/sequences/0/visual/2/clips/0";

fn sink_value() -> Value {
    let (doc, ids) = kitchen_sink();
    to_value(&encode(&doc, ids, &Extras::new()).unwrap())
}

fn clip(id: u64) -> ClipId {
    ClipId::from_raw(nz(id))
}

#[test]
fn stacks_round_trip_with_defaults_left_out() {
    let v = sink_value();
    let color = v.pointer(&format!("{INTRO}/color")).unwrap();
    assert_eq!(
        color,
        &json!({
            "auto": {
                "offsets": {"exposure": 12, "contrast": -5, "blacks": -100, "temperature": -8, "vibrance": 20, "saturation": 5},
                "style": "vivid",
                "source": "playhead",
                "strength": 70
            },
            "match": {
                "gain": [12345, 9000, 20000],
                "offset": [-1234, 56, -10000],
                "intensity": 80,
                "reference": {"clip": 42, "at": 705600000, "ungraded": true}
            },
            "amount": 80,
            "off": ["match", "detail", "curves"],
            "light": {"brightness": 12, "contrast": -15, "contrast_pivot": 30, "tone_lift": 100},
            "color": {"temperature": 40, "saturation": -100, "skin": {"on": true}},
            "hsl": {
                "red": {"saturation": -40},
                "orange": {"hue": 25},
                "blue": {"lightness": -100},
                "magenta": {"hue": 100}
            },
            "curves": {
                "luma": [[0, 0], [2500, 2000], [7500, 8200], [10000, 10000]],
                "blue": [[1000, 500], [9000, 10000]]
            },
            "wheels": {
                "strength": 80,
                "shadows": {"x": 120, "y": -340, "luma": 15},
                "highlights": {"luma": -20},
                "offset": {"x": -707, "y": 707}
            },
            "lut": {
                "hash": "0123456789abcdeffedcba9876543210",
                "file": "Teal \"night\" ✂.cube",
                "intensity": 60,
                "interp": "trilinear",
                "skin": {"on": true, "amount": 40}
            },
            "masks": [
                {"id": 1, "shape": "ellipse", "center": [1200, -800], "size": [7000, 4500], "linked": false, "rotation": 1550, "feather": 350, "expand": -20},
                {"id": 3, "shape": "rectangle", "invert": true, "roundness": 400, "mode": "subtract"},
                {"id": 2, "shape": "band", "linked": true, "rotation": -9000, "width": 800, "mode": "intersect"},
                {"id": 4, "shape": "linear", "center": [0, 2500]}
            ]
        })
    );
    let opened = decode(&compact(&v)).unwrap();
    assert!(opened.warnings.is_empty(), "{:?}", opened.warnings);
    let intro = opened.document.clip(clip(40)).unwrap();
    assert_eq!(intro.color.as_deref(), Some(&kitchen_intro_color()));
    assert_eq!(
        intro.keys.keys(PropPath::Color(ColorParamId::Exposure))[1].value,
        PropValue::Level(35)
    );
    // Clips without a stack write no `color` at all.
    assert!(v.pointer(&format!("{TITLE}/color")).is_none());
}

#[test]
fn out_of_range_levels_are_clamped_as_a_repair() {
    let mut v = sink_value();
    let intro = v.pointer_mut(INTRO).unwrap();
    intro["color"]["amount"] = json!(400);
    intro["color"]["light"]["brightness"] = json!(-99_999_999_999i64);
    intro["keys"]["tracks"][0]["keys"][0]["value"] = json!(170);
    let opened = decode(&compact(&v)).unwrap();
    assert_eq!(
        opened.warnings,
        vec![OpenWarning::ValuesClamped { clips: 1 }]
    );
    let intro = opened.document.clip(clip(40)).unwrap();
    let s = intro.color.as_deref().unwrap();
    assert_eq!((s.amount, s.basic.brightness), (100, -100));
    assert_eq!(
        intro.keys.keys(PropPath::Color(ColorParamId::Exposure))[0].value,
        PropValue::Level(100)
    );
}

#[test]
fn a_stack_on_a_text_clip_and_unknown_sections_are_refused() {
    let mut v = sink_value();
    v.pointer_mut(TITLE).unwrap()["color"] = json!({"light": {"exposure": 5}});
    match decode(&compact(&v)) {
        Err(OpenError::BadValue { path, message }) => {
            assert_eq!(path, "document.sequences[0].visual[2].clips[0].color");
            assert!(message.contains("video, image and adjustment"), "{message}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
    let mut v = sink_value();
    v.pointer_mut(INTRO).unwrap()["color"]["off"] = json!(["light", "glow"]);
    match decode(&compact(&v)) {
        Err(OpenError::BadValue { path, .. }) => {
            assert_eq!(
                path,
                "document.sequences[0].visual[0].clips[0].color.off[1]"
            );
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
    let mut v = sink_value();
    let title = v.pointer_mut(TITLE).unwrap();
    title["keys"]["tracks"][0]["prop"] = json!("color.hue");
    match decode(&compact(&v)) {
        Err(OpenError::BadValue { message, .. }) => {
            assert!(message.contains("video, image and adjustment"), "{message}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn an_empty_or_default_stack_reads_as_none() {
    let mut v = sink_value();
    v.pointer_mut(INTRO).unwrap()["color"] = json!({"light": {"exposure": 0}});
    let opened = decode(&compact(&v)).unwrap();
    assert_eq!(opened.document.clip(clip(40)).unwrap().color, None);
    // And a bypassed stack with nothing else is kept: the switch is set.
    let mut v = sink_value();
    v.pointer_mut(INTRO).unwrap()["color"] = json!({"bypass": true, "off": ["color"]});
    let opened = decode(&compact(&v)).unwrap();
    let s = opened
        .document
        .clip(clip(40))
        .unwrap()
        .color
        .clone()
        .unwrap();
    assert_eq!(
        *s,
        ColorStack {
            bypass: true,
            sections_enabled: ColorStack::default()
                .sections_enabled
                .with(Section::Color, false),
            ..ColorStack::default()
        }
    );
    let again = to_value(&encode(&opened.document, opened.ids.next(), &opened.extras).unwrap());
    assert_eq!(
        again.pointer(&format!("{INTRO}/color")),
        Some(&json!({"bypass": true, "off": ["color"]}))
    );
}

#[test]
fn k2_keys_round_trip_and_out_of_range_parts_are_repaired() {
    let v = sink_value();
    let tracks = v.pointer(&format!("{INTRO}/keys/tracks")).unwrap();
    let props: Vec<&str> = tracks
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["prop"].as_str().unwrap())
        .collect();
    assert_eq!(
        props,
        [
            "color.exposure",
            "color.hsl.green.saturation",
            "color.lut.intensity",
            "color.auto.strength",
            "color.match.intensity",
            "color.wheels.midtones.puck",
            "color.masks.1.center",
            "color.masks.1.expand",
            "color.masks.2.band_width",
            "color.masks.3.rotation"
        ]
    );
    assert_eq!(tracks[5]["keys"][1]["value"], json!({"x": 300, "y": 700}));
    let opened = decode(&compact(&v)).unwrap();
    let intro = opened.document.clip(clip(40)).unwrap();
    assert_eq!(
        intro.keys.keys(PropPath::ColorPuck(WheelId::Midtones))[0].value,
        PropValue::Puck { x: -600, y: 0 }
    );

    // Levels past their range, coordinates past 0–10000 and pucks off the
    // disk are pulled back and counted as one repaired clip.
    let mut v = sink_value();
    let intro = v.pointer_mut(INTRO).unwrap();
    intro["color"]["hsl"]["red"]["saturation"] = json!(-300);
    intro["color"]["wheels"]["shadows"] = json!({"x": 5000, "y": 0, "luma": 900});
    intro["color"]["wheels"]["strength"] = json!(-4);
    intro["color"]["curves"]["blue"] = json!([[-50, 500], [9000, 12000]]);
    intro["keys"]["tracks"][5]["keys"][0]["value"] = json!({"x": 0, "y": -99999});
    let opened = decode(&compact(&v)).unwrap();
    assert_eq!(
        opened.warnings,
        vec![OpenWarning::ValuesClamped { clips: 1 }]
    );
    let intro = opened.document.clip(clip(40)).unwrap();
    let s = intro.color.as_deref().unwrap();
    assert_eq!(s.hsl.band(HueBand::Red).sat, -100);
    assert_eq!(s.puck(WheelId::Shadows), (1000, 0));
    assert_eq!(s.wheels.shadows.luma(), 100);
    assert_eq!(s.wheels.strength, 0);
    assert_eq!(
        s.curves.get(CurveChannel::Blue).points(),
        [CurvePoint::new(0, 500), CurvePoint::new(9000, 10000)]
    );
    assert_eq!(
        intro.keys.keys(PropPath::ColorPuck(WheelId::Midtones))[0].value,
        PropValue::Puck { x: 0, y: -1000 }
    );
    assert_eq!(
        intro.keys.keys(PropPath::Color(ColorParamId::Hsl {
            band: HueBand::Green,
            channel: HslChannel::Saturation
        }))[0]
            .value,
        PropValue::Level(-55)
    );
}

#[test]
fn a_list_that_is_no_curve_is_refused_at_its_path() {
    for (points, rule) in [
        (json!([[0, 0]]), "two endpoints"),
        (json!([[0, 0], [5000, 1], [5050, 2], [10000, 3]]), "0.01"),
        (json!([[3000, 0], [10000, 1]]), "left endpoint"),
        (json!([[0, 0], [7000, 1]]), "right endpoint"),
    ] {
        let mut v = sink_value();
        v.pointer_mut(INTRO).unwrap()["color"]["curves"]["red"] = points;
        match decode(&compact(&v)) {
            Err(OpenError::BadValue { path, message }) => {
                assert_eq!(
                    path,
                    "document.sequences[0].visual[0].clips[0].color.curves.red"
                );
                assert!(message.contains(rule), "{message}");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }
    let many: Vec<[u32; 2]> = (0..17).map(|i| [i * 600, i * 600]).collect();
    let mut v = sink_value();
    v.pointer_mut(INTRO).unwrap()["color"]["curves"]["luma"] = json!(many);
    assert!(matches!(
        decode(&compact(&v)),
        Err(OpenError::BadValue { .. })
    ));
}

#[test]
fn defaults_of_the_k2_parts_are_left_out() {
    let mut v = sink_value();
    let color = &mut v.pointer_mut(INTRO).unwrap()["color"];
    color["hsl"] = json!({"red": {"hue": 0}});
    color["curves"] = json!({"green": [[0, 0], [10000, 10000]]});
    color["wheels"] = json!({"strength": 100, "midtones": {"x": 0, "y": 0, "luma": 0}});
    color.as_object_mut().unwrap().remove("off");
    let opened = decode(&compact(&v)).unwrap();
    let again = to_value(&encode(&opened.document, opened.ids.next(), &opened.extras).unwrap());
    let color = again.pointer(&format!("{INTRO}/color")).unwrap();
    for part in ["hsl", "curves", "wheels", "off"] {
        assert!(color.get(part).is_none(), "{part}: {color}");
    }
}

#[test]
fn the_lut_slot_and_skin_protection_round_trip() {
    let v = sink_value();
    let color = v.pointer(&format!("{INTRO}/color")).unwrap();
    assert_eq!(
        color["lut"],
        json!({
            "hash": "0123456789abcdeffedcba9876543210",
            "file": "Teal \"night\" ✂.cube",
            "intensity": 60,
            "interp": "trilinear",
            "skin": {"on": true, "amount": 40}
        })
    );
    assert_eq!(color["color"]["skin"], json!({"on": true}));
    let opened = decode(&compact(&v)).unwrap();
    let intro = opened.document.clip(clip(40)).unwrap();
    let s = intro.color.as_deref().unwrap();
    assert_eq!(s.lut, kitchen_intro_color().lut);
    assert_eq!(
        s.basic.skin,
        SkinProtect {
            on: true,
            amount: 70
        }
    );
    assert_eq!(
        intro.keys.keys(PropPath::Color(ColorParamId::LutIntensity))[1].value,
        PropValue::Level(90)
    );

    // Out-of-range levels are pulled back and counted.
    let mut v = sink_value();
    let intro = v.pointer_mut(INTRO).unwrap();
    intro["color"]["lut"]["intensity"] = json!(250);
    intro["color"]["color"]["skin"] = json!({"on": false, "amount": -5});
    let opened = decode(&compact(&v)).unwrap();
    assert_eq!(
        opened.warnings,
        vec![OpenWarning::ValuesClamped { clips: 1 }]
    );
    let s = opened
        .document
        .clip(clip(40))
        .unwrap()
        .color
        .clone()
        .unwrap();
    assert_eq!(s.lut.as_ref().unwrap().intensity, 100);
    assert_eq!(
        s.basic.skin,
        SkinProtect {
            on: false,
            amount: 0
        }
    );
}

#[test]
fn a_bad_lut_hash_or_interpolation_is_refused_at_its_path() {
    for (key, value, path, says) in [
        ("hash", json!("0123"), "color.lut.hash", "32 hex digits"),
        (
            "hash",
            json!("zz23456789abcdeffedcba9876543210"),
            "color.lut.hash",
            "32 hex digits",
        ),
        ("interp", json!("cubic"), "color.lut.interp", "`cubic`"),
    ] {
        let mut v = sink_value();
        v.pointer_mut(INTRO).unwrap()["color"]["lut"][key] = value;
        match decode(&compact(&v)) {
            Err(OpenError::BadValue { path: p, message }) => {
                assert_eq!(
                    p,
                    format!("document.sequences[0].visual[0].clips[0].{path}")
                );
                assert!(message.contains(says), "{message}");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }
}

#[test]
fn defaults_of_the_k3_parts_are_left_out() {
    let mut v = sink_value();
    let color = &mut v.pointer_mut(INTRO).unwrap()["color"];
    color["lut"] = json!({
        "hash": "0123456789ABCDEFFEDCBA9876543210",
        "file": "a.cube",
        "intensity": 100,
        "interp": "tetrahedral",
        "skin": {"on": false, "amount": 70}
    });
    color["color"]["skin"] = json!({"amount": 70});
    let opened = decode(&compact(&v)).unwrap();
    let again = to_value(&encode(&opened.document, opened.ids.next(), &opened.extras).unwrap());
    let color = again.pointer(&format!("{INTRO}/color")).unwrap();
    assert_eq!(
        color["lut"],
        json!({"hash": "0123456789abcdeffedcba9876543210", "file": "a.cube"})
    );
    assert!(color["color"].get("skin").is_none(), "{color}");
}

const STILL: &str = "/document/sequences/0/visual/0/clips/1";

#[test]
fn auto_and_match_round_trip_with_their_keys() {
    let v = sink_value();
    let still = v.pointer(&format!("{STILL}/color/match")).unwrap();
    assert_eq!(
        still,
        &json!({
            "gain": [10000, 10500, 9500],
            "offset": [300, -20, 40],
            "reference": {"media": 12, "at": 0}
        })
    );
    let opened = decode(&compact(&v)).unwrap();
    assert!(opened.warnings.is_empty(), "{:?}", opened.warnings);
    let intro = opened.document.clip(clip(40)).unwrap();
    let s = intro.color.as_deref().unwrap();
    let auto = s.auto.unwrap();
    assert_eq!(
        (auto.style, auto.source, auto.strength, auto.offsets.blacks),
        (AutoStyle::Vivid, AutoSource::Playhead, 70, -100)
    );
    let m = s.match_xf.as_ref().unwrap();
    assert_eq!(m.gain(), [12_345, 9_000, 20_000]);
    assert_eq!(
        m.reference,
        MatchRef::Clip {
            clip: clip(42),
            at: freecut_timeline::time::Tick::from_raw(705_600_000),
            graded: false
        }
    );
    assert!(!s.sections_enabled.is_on(Section::Match));
    assert_eq!(
        intro.keys.keys(PropPath::Color(ColorParamId::AutoStrength))[1].value,
        PropValue::Level(100)
    );
    assert_eq!(
        intro
            .keys
            .keys(PropPath::Color(ColorParamId::MatchIntensity))[0]
            .value,
        PropValue::Level(45)
    );
    let s = opened
        .document
        .clip(clip(41))
        .unwrap()
        .color
        .clone()
        .unwrap();
    assert_eq!(s.match_xf, Some(kitchen_still_match()));

    // Out-of-range offsets, gains, Strength and Intensity are pulled back
    // and counted.
    let mut v = sink_value();
    let color = &mut v.pointer_mut(INTRO).unwrap()["color"];
    color["auto"]["offsets"]["tint"] = json!(900);
    color["auto"]["strength"] = json!(-4);
    color["match"]["gain"] = json!([1, 99_999, 10_000]);
    color["match"]["offset"] = json!([-50_000, 0, 7]);
    color["match"]["intensity"] = json!(101);
    let opened = decode(&compact(&v)).unwrap();
    assert_eq!(
        opened.warnings,
        vec![OpenWarning::ValuesClamped { clips: 1 }]
    );
    let s = opened
        .document
        .clip(clip(40))
        .unwrap()
        .color
        .clone()
        .unwrap();
    let auto = s.auto.unwrap();
    assert_eq!((auto.offsets.tint, auto.strength), (100, 0));
    let m = s.match_xf.as_ref().unwrap();
    assert_eq!(m.gain(), [5_000, 20_000, 10_000]);
    assert_eq!(m.offset(), [-10_000, 0, 7]);
    assert_eq!(m.intensity, 100);
}

#[test]
fn bad_auto_and_match_parts_are_refused_at_their_path() {
    for (part, key, value, path, says) in [
        ("auto", "style", json!("loud"), "color.auto.style", "`loud`"),
        ("auto", "source", json!("ear"), "color.auto.source", "`ear`"),
        (
            "match",
            "reference",
            json!({"at": 0}),
            "color.match.reference",
            "exactly one",
        ),
        (
            "match",
            "reference",
            json!({"clip": 3, "media": 4, "at": 0}),
            "color.match.reference",
            "exactly one",
        ),
    ] {
        let mut v = sink_value();
        v.pointer_mut(INTRO).unwrap()["color"][part][key] = value;
        match decode(&compact(&v)) {
            Err(OpenError::BadValue { path: p, message }) => {
                assert_eq!(
                    p,
                    format!("document.sequences[0].visual[0].clips[0].{path}")
                );
                assert!(message.contains(says), "{message}");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }
}

#[test]
fn defaults_of_the_k5_parts_are_left_out() {
    let mut v = sink_value();
    let color = &mut v.pointer_mut(INTRO).unwrap()["color"];
    color["auto"] = json!({
        "offsets": {"exposure": 0, "tint": 3},
        "style": "natural",
        "source": "clip",
        "strength": 100
    });
    color["match"]["intensity"] = json!(100);
    color["match"]["reference"] = json!({"clip": 42, "at": 0, "ungraded": false});
    let opened = decode(&compact(&v)).unwrap();
    let again = to_value(&encode(&opened.document, opened.ids.next(), &opened.extras).unwrap());
    let color = again.pointer(&format!("{INTRO}/color")).unwrap();
    assert_eq!(color["auto"], json!({"offsets": {"tint": 3}}));
    assert!(color["match"].get("intensity").is_none());
    assert_eq!(color["match"]["reference"], json!({"clip": 42, "at": 0}));
}

// Format 14: colour masks (CL-031).

#[test]
fn masks_round_trip_with_their_keys_and_defaults_left_out() {
    let v = sink_value();
    let tracks = v.pointer(&format!("{INTRO}/keys/tracks")).unwrap();
    assert_eq!(tracks[6]["keys"][1]["value"], json!({"x": 2500, "y": -700}));
    assert_eq!(tracks[8]["keys"][0]["value"], json!(300));
    let opened = decode(&compact(&v)).unwrap();
    let intro = opened.document.clip(clip(40)).unwrap();
    let s = intro.color.as_deref().unwrap();
    let ids: Vec<u16> = s.masks.iter().map(|m| m.id).collect();
    assert_eq!(ids, [1, 3, 2, 4], "list order is kept");
    assert_eq!(
        intro.keys.keys(kitchen_mask_key(1, "center"))[1].value,
        PropValue::MaskPoint(MaskPoint::new(2_500, -700))
    );
    // The adjustment clip's mask is kept while its section is off.
    let adjust = v
        .pointer("/document/sequences/0/visual")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|t| t["clips"].as_array().unwrap())
        .find(|c| c["id"] == 46)
        .unwrap();
    assert_eq!(
        adjust["color"]["masks"],
        json!([{"id": 1, "shape": "ellipse"}])
    );
    assert_eq!(adjust["color"]["off"], json!(["color", "masks"]));
    // A default mask writes only its id and shape.
    let one = ColorStack {
        masks: vec![ColorMask::new(5, Mask::new(MaskShapeKind::Rectangle))],
        ..ColorStack::default().with(ColorParamId::Exposure, 5)
    };
    let mut v = sink_value();
    v.pointer_mut(INTRO).unwrap()["color"] =
        json!({"light": {"exposure": 5}, "masks": [{"id": 5, "shape": "rectangle"}]});
    v.pointer_mut(INTRO)
        .unwrap()
        .as_object_mut()
        .unwrap()
        .remove("keys");
    let opened = decode(&compact(&v)).unwrap();
    assert_eq!(
        opened.document.clip(clip(40)).unwrap().color.as_deref(),
        Some(&one)
    );
}

#[test]
fn out_of_range_mask_values_are_clamped_as_a_repair() {
    let mut v = sink_value();
    let intro = v.pointer_mut(INTRO).unwrap();
    intro["color"]["masks"][0]["center"] = json!([99_999_999_999i64, -40_000]);
    intro["color"]["masks"][0]["feather"] = json!(5_000);
    intro["color"]["masks"][0]["expand"] = json!(-900);
    intro["color"]["masks"][2]["width"] = json!(-3);
    intro["keys"]["tracks"][7]["keys"][0]["value"] = json!(400);
    let opened = decode(&compact(&v)).unwrap();
    assert_eq!(
        opened.warnings,
        vec![OpenWarning::ValuesClamped { clips: 1 }]
    );
    let intro = opened.document.clip(clip(40)).unwrap();
    let s = intro.color.as_deref().unwrap();
    let e = s.mask(1).unwrap();
    assert_eq!(e.mask.center, MaskPoint::new(30_000, -30_000));
    assert_eq!(e.mask.feather, PerMille::ONE);
    assert_eq!(e.expand, -100);
    assert_eq!(
        s.mask(2).unwrap().mask.shape,
        MaskShape::Band {
            width: PerMille::ZERO
        }
    );
    assert_eq!(
        intro.keys.keys(kitchen_mask_key(1, "expand"))[0].value,
        PropValue::Level(100)
    );
}

#[test]
fn bad_masks_are_refused_at_their_path() {
    let at = "document.sequences[0].visual[0].clips[0].color.masks";
    for (change, path, words) in [
        (
            json!({"id": 1, "shape": "heart"}),
            "[0].shape",
            "not `linear`",
        ),
        (json!({"id": 0, "shape": "band"}), "[0].id", "1 to 65535"),
        (
            json!({"id": 70_000, "shape": "band"}),
            "[0].id",
            "1 to 65535",
        ),
        (json!({"id": 3, "shape": "band"}), "[0].id", "used twice"),
        (
            json!({"id": 1, "shape": "ellipse", "roundness": 4}),
            "[0].roundness",
            "no `roundness`",
        ),
        (
            json!({"id": 1, "shape": "rectangle", "width": 4}),
            "[0].width",
            "no `width`",
        ),
        (
            json!({"id": 1, "shape": "rectangle", "roundness": 400, "width": 4}),
            "[0].width",
            "no `width`",
        ),
        (
            json!({"id": 1, "shape": "linear", "mode": "xor"}),
            "[0].mode",
            "not `add`",
        ),
    ] {
        let mut v = sink_value();
        v.pointer_mut(INTRO).unwrap()["color"]["masks"][0] = change.clone();
        v.pointer_mut(INTRO)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("keys");
        let id_twice = path == "[0].id" && words == "used twice";
        let path = if id_twice {
            "[1].id".to_string()
        } else {
            path.to_string()
        };
        match decode(&compact(&v)) {
            Err(OpenError::BadValue { path: p, message }) => {
                assert_eq!(p, format!("{at}{path}"), "{change}");
                assert!(message.contains(words), "{message}");
            }
            other => panic!("expected a refusal of {change}, got {other:?}"),
        }
    }
    // Nine masks are one too many.
    let mut v = sink_value();
    v.pointer_mut(INTRO).unwrap()["color"]["masks"] = (1..=9)
        .map(|id| json!({"id": id, "shape": "ellipse"}))
        .collect();
    match decode(&compact(&v)) {
        Err(OpenError::BadValue { path, message }) => {
            assert_eq!(path, format!("{at}[8]"));
            assert!(message.contains("at most 8"), "{message}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn edit_color(editor: &mut freecut_timeline::Editor, clip_id: u64, edit: ColorEdit) {
    let req = EditRequest::EditColor {
        seq: editor.document().root(),
        clips: vec![clip(clip_id)],
        edit,
    };
    let plan = editor.plan(&EditCtx::default(), &req).unwrap();
    editor.commit(plan, CommitMeta::default()).unwrap();
}

#[test]
fn unknown_keys_of_masks_stay_with_their_mask_id() {
    let mut v = sink_value();
    let masks = &mut v.pointer_mut(INTRO).unwrap()["color"]["masks"];
    for i in 0..4 {
        let id = masks[i]["id"].as_u64().unwrap();
        masks[i]["x_tag"] = json!(format!("belongs-to-mask-{id}"));
    }
    let opened = decode(&compact(&v)).unwrap();
    let mut editor = freecut_timeline::Editor::open(
        opened.document.clone(),
        opened.ids.clone(),
        freecut_timeline::history::HistoryLimits::default(),
    )
    .unwrap();
    // [1, 3, 2, 4] → without 1 → 4 moved to the front: [4, 3, 2].
    edit_color(&mut editor, 40, ColorEdit::RemoveMask(1));
    edit_color(&mut editor, 40, ColorEdit::ReorderMask { id: 4, to: 0 });
    let saved = to_value(&encode(editor.document(), editor.ids().next(), &opened.extras).unwrap());
    let masks = saved.pointer(&format!("{INTRO}/color/masks")).unwrap();
    let tags: Vec<(u64, String)> = masks
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            (
                m["id"].as_u64().unwrap(),
                m["x_tag"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        tags,
        [4, 3, 2]
            .map(|id| (id, format!("belongs-to-mask-{id}")))
            .to_vec()
    );
}

#[test]
fn a_match_reference_keeps_its_id_reserved() {
    for (reference, raised_to) in [
        (json!({"clip": 150, "at": 0}), 151),
        (json!({"media": 160, "at": 0}), 161),
    ] {
        let mut v = sink_value();
        assert_eq!(v["ids_next"], json!(100));
        v.pointer_mut(INTRO).unwrap()["color"]["match"]["reference"] = reference.clone();
        let opened = decode(&compact(&v)).unwrap();
        assert!(
            opened.warnings.contains(&OpenWarning::IdsNextRaised {
                stored: 100,
                raised_to
            }),
            "{reference}: {:?}",
            opened.warnings
        );
        assert_eq!(opened.ids.next().get(), raised_to);
    }
}

#[test]
fn colour_keys_of_values_the_stack_lacks_are_dropped_as_a_repair() {
    let mut v = sink_value();
    let intro = v.pointer_mut(INTRO).unwrap();
    // No LUT, no Auto and no mask 2 any more; their keys are strays.
    let color = intro["color"].as_object_mut().unwrap();
    color.remove("lut");
    color.remove("auto");
    let masks = color["masks"].as_array_mut().unwrap();
    masks.retain(|m| m["id"] != 2);
    let tracks = intro["keys"]["tracks"].as_array_mut().unwrap();
    for (prop, value) in [
        ("color.lut.intensity", json!(50)),
        ("color.auto.strength", json!(50)),
        ("color.masks.2.feather", json!(300)),
    ] {
        if !tracks.iter().any(|t| t["prop"] == prop) {
            tracks.push(
                json!({"prop": prop, "keys": [{"at": 0, "value": value, "interp": "linear"}]}),
            );
        }
    }
    let opened = decode(&compact(&v)).unwrap();
    assert!(
        opened
            .warnings
            .contains(&OpenWarning::StrayColorKeysDropped { clips: 1 }),
        "{:?}",
        opened.warnings
    );
    let intro = opened.document.clip(clip(40)).unwrap();
    for prop in [
        PropPath::Color(ColorParamId::LutIntensity),
        PropPath::Color(ColorParamId::AutoStrength),
        kitchen_mask_key(2, "feather"),
    ] {
        assert!(intro.keys.keys(prop).is_empty(), "{prop} dropped");
    }
    // Keys of values the stack has stay.
    assert!(!intro.keys.keys(kitchen_mask_key(1, "center")).is_empty());
}
