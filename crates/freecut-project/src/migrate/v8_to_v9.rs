//! Format 8 → 9: media origins (PM-041, AU-027).
//!
//! Version 9 added `origin` on library items: `"type": "generated"` with
//! its `kind` (a voiceover take, text-to-speech audio) for files FreeCut
//! made. It is left out while an item is imported, which is what every
//! version-8 item was.
//!
//! So the step writes nothing: an absent key already means the version-8
//! state. It exists so the version number records that a version-8 build
//! can't read what version 9 writes.

use serde_json::Value;

pub(super) fn migrate(doc: Value) -> Result<Value, String> {
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_version_8_document_needs_no_change() {
        let doc = json!({
            "document": {"library": [{"id": 10, "source": {"type": "file"}}]}
        });
        assert_eq!(migrate(doc.clone()), Ok(doc));
    }
}
