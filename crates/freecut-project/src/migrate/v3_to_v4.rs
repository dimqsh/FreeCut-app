//! Format 3 → 4: the v1 sound features (AU-006, AU-008, AU-012, AU-013,
//! AU-038).
//!
//! Version 4 changed or added:
//!
//! - On a clip's `"active"` sound: `gain_keys` (volume keyframes, AU-008)
//!   and `fade_in_curve` / `fade_out_curve` (AU-006). Version 3 had no
//!   keyframes and only linear fades, so the step writes `[]` and
//!   `"linear"`.
//! - On media clip content: `stream` (AU-038), the chosen audio stream.
//!   Version 3 always played the media's default stream: `null`.
//! - On each audio stream of a media item: `default`, `language`, `title`
//!   and `layout` from the probe (AU-038). Version 3 didn't record them:
//!   `false` and `null`. With no stream flagged, the first plays, which is
//!   what version 3 played.
//! - `mix` on tracks (AU-013) and `ui` on the envelope (AU-012's solo set).
//!   Both are left out while they hold the defaults, which is exactly what
//!   every version-3 file had, so nothing is written for them.
//!
//! Keys already present are kept. The step never fails; anything malformed
//! is left for the typed decoder, which reports it with its JSON path.

use serde_json::{Map, Value};

pub(super) fn migrate(mut doc: Value) -> Result<Value, String> {
    let Some(document) = doc
        .as_object_mut()
        .and_then(|root| root.get_mut("document"))
        .and_then(Value::as_object_mut)
    else {
        return Ok(doc);
    };
    for item in objects_in(document.get_mut("library")) {
        let info = item.get_mut("info").and_then(Value::as_object_mut);
        for stream in objects_in(info.and_then(|info| info.get_mut("audio"))) {
            stream.entry("default").or_insert(Value::Bool(false));
            for key in ["language", "title", "layout"] {
                stream.entry(key).or_insert(Value::Null);
            }
        }
    }
    for seq in objects_in(document.get_mut("sequences")) {
        for lane in ["visual", "audio"] {
            for track in objects_in(seq.get_mut(lane)) {
                for clip in objects_in(track.get_mut("clips")) {
                    upgrade_clip(clip);
                }
            }
        }
    }
    Ok(doc)
}

fn upgrade_clip(clip: &mut Map<String, Value>) {
    if let Some(content) = clip.get_mut("content").and_then(Value::as_object_mut)
        && is_type(content, "media")
    {
        content.entry("stream").or_insert(Value::Null);
    }
    if let Some(audio) = clip.get_mut("audio").and_then(Value::as_object_mut)
        && is_type(audio, "active")
    {
        audio
            .entry("gain_keys")
            .or_insert_with(|| Value::Array(Vec::new()));
        for key in ["fade_in_curve", "fade_out_curve"] {
            audio
                .entry(key)
                .or_insert_with(|| Value::String("linear".into()));
        }
    }
}

fn is_type(object: &Map<String, Value>, tag: &str) -> bool {
    object.get("type").and_then(Value::as_str) == Some(tag)
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
    fn sound_settings_get_the_version_3_meaning() {
        let v3 = json!({
            "format_version": 3,
            "document": {
                "library": [{"id": 1, "info": {"audio": [
                    {"index": 1, "codec": "aac"},
                    {"index": 2, "default": true, "x_future": 1},
                ]}}],
                "sequences": [{
                    "visual": [{"clips": [
                        {"content": {"type": "media", "asset": 1},
                         "audio": {"type": "active", "gain": 0}},
                        {"content": {"type": "media", "asset": 1, "stream": {"type": "all-mixed"}},
                         "audio": {"type": "detached", "link": 9}},
                        {"content": {"type": "image", "asset": 2},
                         "audio": {"type": "none"}},
                    ]}],
                    "audio": [{"clips": [
                        {"content": {"type": "media", "asset": 1},
                         "audio": {"type": "active", "fade_in_curve": "fast"}},
                    ]}]
                }]
            }
        });
        let v4 = migrate(v3).unwrap();
        let audio = &v4["document"]["library"][0]["info"]["audio"];
        assert_eq!(
            audio[0],
            json!({"index": 1, "codec": "aac", "default": false, "language": null,
                   "title": null, "layout": null})
        );
        assert_eq!(audio[1]["default"], true, "a value already there is kept");
        assert_eq!(audio[1]["x_future"], 1);

        let seq = &v4["document"]["sequences"][0];
        let clips = &seq["visual"][0]["clips"];
        assert_eq!(clips[0]["content"]["stream"], Value::Null);
        assert_eq!(
            clips[0]["audio"],
            json!({"type": "active", "gain": 0, "gain_keys": [],
                   "fade_in_curve": "linear", "fade_out_curve": "linear"})
        );
        assert_eq!(clips[1]["content"]["stream"], json!({"type": "all-mixed"}));
        assert_eq!(clips[1]["audio"], json!({"type": "detached", "link": 9}));
        assert_eq!(clips[2]["content"], json!({"type": "image", "asset": 2}));
        assert_eq!(clips[2]["audio"], json!({"type": "none"}));
        let music = &seq["audio"][0]["clips"][0]["audio"];
        assert_eq!(music["fade_in_curve"], "fast");
        assert_eq!(music["fade_out_curve"], "linear");
        // Track stages and UI state stay absent: absent is the default.
        assert!(seq["visual"][0].get("mix").is_none());
        assert!(v4.get("ui").is_none());
    }

    #[test]
    fn malformed_documents_are_left_for_the_decoder() {
        for doc in [
            json!([]),
            json!({}),
            json!({"document": 3}),
            json!({"document": {"library": "x", "sequences": {}}}),
            json!({"document": {"library": [{"info": 1}, {"info": {"audio": [1]}}]}}),
            json!({"document": {"sequences": [{"visual": [{"clips": [7, {"content": 1}]}]}]}}),
        ] {
            assert_eq!(migrate(doc.clone()).unwrap(), doc);
        }
    }
}
