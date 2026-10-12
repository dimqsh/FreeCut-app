//! Format 1 → 2: clip sound (AU-002, TL-030) and media sources (AU-004).
//!
//! Version 2 added two keys that every object of its kind carries:
//!
//! - `source` on each library item. Version 1 only had imported files, so
//!   every item gets `{"type": "file"}`.
//! - `audio` on each clip. Version 1 stored no sound settings, so each clip
//!   gets what a new clip of its content gets today: media clips play their
//!   sound at unity gain without fades (`{"type": "active", …}`), except a
//!   video clip whose library item has no audio stream, which has none
//!   (`{"type": "none"}`). Images and generated items have none. Version 1
//!   had no detached sound, so nothing becomes `"detached"`.
//!
//! A key that is already there is kept: a development build may have
//! written it under version 1, and it then holds the real value. The step
//! never fails; anything malformed is left for the typed decoder, which
//! reports it with its JSON path.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

pub(super) fn migrate(mut doc: Value) -> Result<Value, String> {
    let Some(document) = doc.get_mut("document").and_then(Value::as_object_mut) else {
        return Ok(doc);
    };
    let mut has_sound = BTreeMap::new();
    for item in objects_in(document.get_mut("library")) {
        if let Some(id) = item.get("id").and_then(Value::as_u64) {
            has_sound.insert(id, audio_streams(item) != Some(0));
        }
        item.entry("source")
            .or_insert_with(|| json!({"type": "file"}));
    }
    for seq in objects_in(document.get_mut("sequences")) {
        for lane in ["visual", "audio"] {
            for track in objects_in(seq.get_mut(lane)) {
                for clip in objects_in(track.get_mut("clips")) {
                    if !clip.contains_key("audio") {
                        let audio = default_audio(clip, &has_sound);
                        clip.insert("audio".into(), audio);
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

/// The number of audio streams of a library item, if readable.
fn audio_streams(item: &Map<String, Value>) -> Option<usize> {
    let streams = item.get("info")?.get("audio")?.as_array()?;
    Some(streams.len())
}

/// The sound a version-1 clip had: what `Clip::new` and the inserter give
/// its content today. `has_sound` maps media ids to whether the item has an
/// audio stream.
fn default_audio(clip: &Map<String, Value>, has_sound: &BTreeMap<u64, bool>) -> Value {
    let content = clip.get("content");
    let field = |key: &str| content.and_then(|c| c.get(key));
    let active = json!({
        "type": "active",
        "gain": 0,
        "muted": false,
        "fade_in": 0,
        "fade_out": 0
    });
    let none = json!({"type": "none"});
    if field("type").and_then(Value::as_str) != Some("media") {
        return none;
    }
    if field("kind").and_then(Value::as_str) == Some("audio") {
        // Audio clips always play their sound.
        return active;
    }
    // A video clip; one whose media is missing keeps the default, and the
    // decoder reports the missing media.
    let silent = field("asset")
        .and_then(Value::as_u64)
        .and_then(|asset| has_sound.get(&asset))
        .is_some_and(|has| !has);
    if silent { none } else { active }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn clip(id: u64, content: Value) -> Value {
        json!({"id": id, "content": content, "link": null})
    }

    fn media(asset: u64, kind: &str) -> Value {
        json!({"type": "media", "asset": asset, "kind": kind, "source": {}})
    }

    fn active() -> Value {
        json!({"type": "active", "gain": 0, "muted": false, "fade_in": 0, "fade_out": 0})
    }

    fn item(id: u64, audio_streams: usize) -> Value {
        json!({"id": id, "path": {}, "info": {"audio": vec![json!({}); audio_streams]}})
    }

    #[test]
    fn every_item_reads_a_file_and_every_clip_gets_the_sound_of_its_content() {
        let v1 = json!({
            "format_version": 1,
            "document": {
                "library": [item(1, 2), item(2, 0), item(3, 0)],
                "sequences": [{
                    "visual": [{"clips": [
                        clip(10, media(1, "video")),
                        clip(11, media(2, "video")),
                        clip(12, json!({"type": "image", "asset": 3})),
                        clip(13, media(9, "video")),
                    ]}, {"clips": [
                        clip(14, json!({"type": "generated", "kind": "text"})),
                    ]}],
                    "audio": [{"clips": [
                        clip(15, media(1, "audio")),
                        clip(16, media(2, "audio")),
                    ]}]
                }]
            }
        });
        let v2 = migrate(v1).unwrap();
        let library = v2["document"]["library"].as_array().unwrap();
        for item in library {
            assert_eq!(item["source"], json!({"type": "file"}));
        }
        let seq = &v2["document"]["sequences"][0];
        let sound =
            |lane: &str, track: usize, i: usize| seq[lane][track]["clips"][i]["audio"].clone();
        let none = json!({"type": "none"});
        // Video with sound plays it; video without an audio stream has none.
        assert_eq!(sound("visual", 0, 0), active());
        assert_eq!(sound("visual", 0, 1), none);
        assert_eq!(sound("visual", 0, 2), none, "images have no sound");
        assert_eq!(sound("visual", 0, 3), active(), "missing media: default");
        assert_eq!(sound("visual", 1, 0), none, "generated items have no sound");
        // Audio clips always play, even of an item without audio streams.
        assert_eq!(sound("audio", 0, 0), active());
        assert_eq!(sound("audio", 0, 1), active());
    }

    #[test]
    fn keys_already_there_are_kept() {
        let v1 = json!({"document": {
            "library": [{"id": 1, "source": {"type": "audio-of", "media": 7, "stream": 1}}],
            "sequences": [{"visual": [{"clips": [
                {"id": 2, "content": media(1, "video"), "audio": {"type": "detached", "link": 5}}
            ]}], "audio": []}]
        }});
        let v2 = migrate(v1.clone()).unwrap();
        assert_eq!(v2, v1);
    }

    #[test]
    fn malformed_documents_are_left_for_the_decoder() {
        for doc in [
            json!({}),
            json!({"document": 3}),
            json!({"document": {"library": "x", "sequences": [1, {"visual": {}}]}}),
            json!({"document": {"sequences": [{"visual": [{"clips": [7, {"content": 1}]}]}]}}),
        ] {
            let out = migrate(doc.clone()).unwrap();
            if let Some(clip) = out.pointer("/document/sequences/0/visual/0/clips/1") {
                assert_eq!(clip["audio"], json!({"type": "none"}));
            } else {
                assert_eq!(out, doc);
            }
        }
    }
}
