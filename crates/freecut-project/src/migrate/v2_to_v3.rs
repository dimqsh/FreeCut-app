//! Format 2 → 3: text clips (TX-001), clip transforms (TR-001) and saved
//! export settings (EX-021).
//!
//! Version 3 changed or added:
//!
//! - Text clips have their own content, `{"type": "text", …}`, holding the
//!   text, its style and its layout. Version 2 stored a text clip as
//!   `{"type": "generated", "kind": "text"}` with no payload, so each one
//!   becomes the payload a new text clip gets: the placeholder "Your text"
//!   in the factory style (TX-001; table frozen below). Unknown keys of the
//!   old object stay in the new one.
//! - `transform` on clips. It is left out while it is the identity, which
//!   is exactly what every version-2 clip had, so nothing is written for it.
//! - `export` on the envelope: `null` (no export settings were saved).
//!
//! The step never fails; anything malformed is left for the typed decoder,
//! which reports it with its JSON path.

use serde_json::{Map, Value, json};

/// The payload of a new text clip when format 3 was introduced (TX-001:
/// "Your text", Noto Sans Bold, 64 ref px, white, centred, auto width,
/// 100 % line spacing). Frozen here: a later change of the factory default
/// must not change what old files open as.
fn default_text() -> Map<String, Value> {
    let value = json!({
        "type": "text",
        "text": "Your text",
        "style": {
            "font": {"family": "Noto Sans"},
            "size": 6400,
            "fill": "#FFFFFFFF",
            "bold": true,
            "italic": false
        },
        "layout": {
            "align": "center",
            "box_width": null,
            "line_spacing": 1000
        },
        "origin": "manual"
    });
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

pub(super) fn migrate(mut doc: Value) -> Result<Value, String> {
    let Some(root) = doc.as_object_mut() else {
        return Ok(doc);
    };
    root.entry("export").or_insert(Value::Null);
    let Some(document) = root.get_mut("document").and_then(Value::as_object_mut) else {
        return Ok(doc);
    };
    for seq in objects_in(document.get_mut("sequences")) {
        for lane in ["visual", "audio"] {
            for track in objects_in(seq.get_mut(lane)) {
                for clip in objects_in(track.get_mut("clips")) {
                    if let Some(content) = clip.get_mut("content").and_then(Value::as_object_mut)
                        && is_generated_text(content)
                    {
                        upgrade_text(content);
                    }
                }
            }
        }
    }
    Ok(doc)
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

fn is_generated_text(content: &Map<String, Value>) -> bool {
    content.get("type").and_then(Value::as_str) == Some("generated")
        && content.get("kind").and_then(Value::as_str) == Some("text")
}

/// Turns a version-2 generated text object into a text content object in
/// place: the default payload first, then the old object's unknown keys.
fn upgrade_text(content: &mut Map<String, Value>) {
    let mut upgraded = default_text();
    for (key, value) in std::mem::take(content) {
        if key != "type" && key != "kind" {
            upgraded.entry(key).or_insert(value);
        }
    }
    *content = upgraded;
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn clip(id: u64, content: Value) -> Value {
        json!({"id": id, "content": content})
    }

    #[test]
    fn generated_text_gets_the_default_payload_and_other_content_is_kept() {
        let v2 = json!({
            "format_version": 2,
            "document": {"sequences": [{
                "visual": [{"clips": [
                    clip(1, json!({"type": "generated", "kind": "text", "x_future": 7})),
                    clip(2, json!({"type": "generated", "kind": "sticker"})),
                    clip(3, json!({"type": "media", "asset": 4, "kind": "video", "source": {}})),
                ]}],
                "audio": []
            }]}
        });
        let v3 = migrate(v2).unwrap();
        assert_eq!(v3["export"], Value::Null);
        let clips = &v3["document"]["sequences"][0]["visual"][0]["clips"];
        let mut expected = default_text();
        expected.insert("x_future".into(), json!(7));
        assert_eq!(clips[0]["content"], Value::Object(expected));
        assert_eq!(
            clips[1]["content"],
            json!({"type": "generated", "kind": "sticker"})
        );
        assert_eq!(clips[2]["content"]["type"], "media");
        // Transforms stay absent: absent is the identity in every version.
        assert!(clips[0].get("transform").is_none());
    }

    #[test]
    fn the_frozen_default_is_what_a_new_text_clip_gets() {
        // Written by today's writer for `TextClipData::default()`; this
        // only fails if the factory default changes, and then the frozen
        // table above must stay as it is.
        let text = default_text();
        assert_eq!(text["text"], "Your text");
        assert_eq!(text["style"]["size"], 6400);
        assert_eq!(text["layout"]["align"], "center");
    }

    #[test]
    fn an_export_key_already_there_is_kept() {
        let v2 = json!({"export": {"x": 1}, "document": {}});
        assert_eq!(migrate(v2.clone()).unwrap(), v2);
    }

    #[test]
    fn malformed_documents_are_left_for_the_decoder() {
        for doc in [
            json!([]),
            json!({"document": 3}),
            json!({"document": {"sequences": "x"}}),
            json!({"document": {"sequences": [{"visual": [{"clips": [7, {"content": 1}]}]}]}}),
        ] {
            let out = migrate(doc.clone()).unwrap();
            if let Some(object) = doc.as_object() {
                let mut expected = object.clone();
                expected.entry("export").or_insert(Value::Null);
                assert_eq!(out, Value::Object(expected));
            } else {
                assert_eq!(out, doc);
            }
        }
    }
}
