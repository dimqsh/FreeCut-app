//! Format 4 → 5: text round 2 (TX-005, TX-008 to TX-016, TX-054, TX-055).
//!
//! Version 5 changed or added, on the content of `"type": "text"` clips:
//!
//! - `style.fill` became an object with variants (TX-014): `"type":
//!   "solid"` with a `color`, `"linear-gradient"` or `"radial-gradient"`.
//!   Version 4 only had solid fills, stored as the colour string; the step
//!   wraps it as `{"type": "solid", "color": …}`.
//! - `style.underline`, `style.strike`, `style.case` and
//!   `style.letter_spacing` (TX-008, TX-009), and `layout.justify` and
//!   `layout.v_anchor` (TX-005). Version 4 had no decoration, showed text
//!   as typed with no extra spacing, never justified and anchored the box
//!   in the middle: the step writes `false`, `false`, `"as-typed"`, `0`,
//!   `false` and `"middle"`.
//! - The optional layers `style.outline`, `shadow`, `glow`, `background`,
//!   `bubble` and `curve` (TX-010 to TX-016, TX-055). They are left out
//!   while they hold the factory default (off), which is exactly what every
//!   version-4 text had, so nothing is written for them.
//!
//! Keys already present are kept. The step never fails; anything malformed
//! (a fill that is neither a string nor an object, say) is left for the
//! typed decoder, which reports it with its JSON path.

use serde_json::{Map, Value};

pub(super) fn migrate(mut doc: Value) -> Result<Value, String> {
    let Some(document) = doc
        .as_object_mut()
        .and_then(|root| root.get_mut("document"))
        .and_then(Value::as_object_mut)
    else {
        return Ok(doc);
    };
    for seq in objects_in(document.get_mut("sequences")) {
        for lane in ["visual", "audio"] {
            for track in objects_in(seq.get_mut(lane)) {
                for clip in objects_in(track.get_mut("clips")) {
                    if let Some(content) = clip.get_mut("content").and_then(Value::as_object_mut)
                        && content.get("type").and_then(Value::as_str) == Some("text")
                    {
                        upgrade_text(content);
                    }
                }
            }
        }
    }
    Ok(doc)
}

fn upgrade_text(content: &mut Map<String, Value>) {
    if let Some(style) = content.get_mut("style").and_then(Value::as_object_mut) {
        if let Some(Value::String(color)) = style.get("fill") {
            let mut solid = Map::new();
            solid.insert("type".into(), Value::String("solid".into()));
            solid.insert("color".into(), Value::String(color.clone()));
            style.insert("fill".into(), Value::Object(solid));
        }
        style.entry("underline").or_insert(Value::Bool(false));
        style.entry("strike").or_insert(Value::Bool(false));
        style
            .entry("case")
            .or_insert_with(|| Value::String("as-typed".into()));
        style.entry("letter_spacing").or_insert(Value::from(0));
    }
    if let Some(layout) = content.get_mut("layout").and_then(Value::as_object_mut) {
        layout.entry("justify").or_insert(Value::Bool(false));
        layout
            .entry("v_anchor")
            .or_insert_with(|| Value::String("middle".into()));
    }
}

/// The objects in the array `value`, if it is one; other entries are
/// skipped.
fn objects_in(value: Option<&mut Value>) -> impl Iterator<Item = &mut Map<String, Value>> {
    value
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object_mut)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    #[test]
    fn text_gets_the_version_4_meaning() {
        let v4 = json!({
            "format_version": 4,
            "document": {
                "sequences": [{
                    "visual": [{"clips": [
                        {"content": {"type": "text", "text": "Hi",
                         "style": {"font": {"family": "Inter"}, "size": 6400,
                                   "fill": "#FF8000C0", "bold": true, "italic": false,
                                   "x_future": 1},
                         "layout": {"align": "left", "box_width": null,
                                    "line_spacing": 1000},
                         "origin": "manual"}},
                        {"content": {"type": "media", "asset": 1}},
                        {"content": {"type": "text",
                         "style": {"fill": {"type": "solid", "color": "#000000FF"},
                                   "case": "upper"},
                         "layout": {"v_anchor": "top"}}},
                    ]}],
                    "audio": []
                }]
            }
        });
        let v5 = migrate(v4).unwrap();
        let clips = &v5["document"]["sequences"][0]["visual"][0]["clips"];
        assert_eq!(
            clips[0]["content"]["style"],
            json!({"font": {"family": "Inter"}, "size": 6400,
                   "fill": {"type": "solid", "color": "#FF8000C0"},
                   "bold": true, "italic": false, "x_future": 1,
                   "underline": false, "strike": false, "case": "as-typed",
                   "letter_spacing": 0})
        );
        assert_eq!(
            clips[0]["content"]["layout"],
            json!({"align": "left", "box_width": null, "line_spacing": 1000,
                   "justify": false, "v_anchor": "middle"})
        );
        // No layer is written: absent is off with the factory settings.
        for layer in ["outline", "shadow", "glow", "background", "bubble", "curve"] {
            assert!(clips[0]["content"]["style"].get(layer).is_none());
        }
        assert_eq!(clips[1]["content"], json!({"type": "media", "asset": 1}));
        // Values already there are kept.
        let kept = &clips[2]["content"];
        assert_eq!(
            kept["style"]["fill"],
            json!({"type": "solid", "color": "#000000FF"})
        );
        assert_eq!(kept["style"]["case"], "upper");
        assert_eq!(kept["layout"]["v_anchor"], "top");
    }

    #[test]
    fn malformed_documents_are_left_for_the_decoder() {
        for doc in [
            json!([]),
            json!({}),
            json!({"document": 3}),
            json!({"document": {"sequences": {}}}),
            json!({"document": {"sequences": [{"visual": [{"clips": [7, {"content": 1}]}]}]}}),
            json!({"document": {"sequences": [{"visual": [{"clips": [
                {"content": {"type": "text", "style": 5, "layout": []}}
            ]}]}]}}),
        ] {
            assert_eq!(migrate(doc.clone()).unwrap(), doc);
        }
    }
}
