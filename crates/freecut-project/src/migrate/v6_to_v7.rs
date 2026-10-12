//! Format 6 → 7: markers (TL-037).
//!
//! Version 7 added `markers` on every sequence (timeline markers, in
//! sequence time) and on every clip (clip markers, in the clip's content
//! time). Each list is left out while it is empty, which is exactly what
//! every version-6 sequence and clip had.
//!
//! So the step writes nothing: an absent key already means the version-6
//! state. It exists so the version number records that a version-6 build
//! can't read what version 7 writes.

use serde_json::Value;

pub(super) fn migrate(doc: Value) -> Result<Value, String> {
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_version_6_document_needs_no_change() {
        let doc = json!({
            "document": {"sequences": [{
                "flags": {"main_magnet": true},
                "visual": [{"clips": [{"id": 1, "keys": {"origin": 5, "tracks": []}}]}],
                "audio": []
            }]}
        });
        assert_eq!(migrate(doc.clone()), Ok(doc));
    }
}
