//! Colour stacks in the project file (format 10, CL-001): written with
//! every default left out, read back exactly, values outside a slider's
//! range clamped as a repair, and stacks on clips that can't carry one
//! refused at their path.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use common::*;
use freecut_project::{Extras, OpenError, OpenWarning, decode, encode};
use freecut_timeline::ids::ClipId;
use freecut_timeline::model::{ColorParamId, ColorStack, PropPath, PropValue, Section};
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
            "off": ["detail"],
            "light": {"brightness": 12, "contrast": -15, "contrast_pivot": 30, "tone_lift": 100},
            "color": {"temperature": 40, "saturation": -100}
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
