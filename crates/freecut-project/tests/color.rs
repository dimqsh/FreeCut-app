//! Colour stacks in the project file (format 10, CL-001; format 11's HSL,
//! Curves and Wheels, CL-013 to CL-015): written with
//! every default left out, read back exactly, values outside a slider's
//! range clamped as a repair, and stacks on clips that can't carry one
//! refused at their path.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use common::*;
use freecut_project::{Extras, OpenError, OpenWarning, decode, encode};
use freecut_timeline::ids::ClipId;
use freecut_timeline::model::{
    ColorParamId, ColorStack, CurveChannel, CurvePoint, HslChannel, HueBand, PropPath, PropValue,
    Section, SkinProtect, WheelId,
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
            "amount": 80,
            "off": ["detail", "curves"],
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
            }
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
            "color.wheels.midtones.puck"
        ]
    );
    assert_eq!(tracks[3]["keys"][1]["value"], json!({"x": 300, "y": 700}));
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
    intro["keys"]["tracks"][3]["keys"][0]["value"] = json!({"x": 0, "y": -99999});
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
