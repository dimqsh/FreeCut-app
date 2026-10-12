//! Format 7 → 8: *Follow main track* (TL-029).
//!
//! Version 8 added:
//!
//! - `follow_main` and `follow_kinds` on every sequence's `flags`. The
//!   reader takes a missing key as on and every kind, the model's default
//!   for new sequences; the toolbar round made following the default for
//!   older projects too (TL-029's build contract).
//! - `attach` on clips over a main-track clip. Left out while a clip
//!   follows none, which is what every version-7 clip did: an older
//!   project opens with following on and nothing attached yet, and items
//!   attach as they are placed, moved or trimmed, or when following is
//!   switched off and on again.
//!
//! So the step writes nothing: an absent key already means the version-7
//! state. It exists so the version number records that a version-7 build
//! can't read what version 8 writes.

use serde_json::Value;

pub(super) fn migrate(doc: Value) -> Result<Value, String> {
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_version_7_document_needs_no_change() {
        let doc = json!({
            "document": {"sequences": [{
                "flags": {"main_magnet": false},
                "markers": [{"id": 9, "at": {"start": 0, "duration": 0}, "name": "",
                             "note": "", "color": "blue", "origin": "user"}],
                "visual": [],
                "audio": []
            }]}
        });
        assert_eq!(migrate(doc.clone()), Ok(doc));
    }
}
