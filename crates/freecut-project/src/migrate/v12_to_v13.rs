//! Format 12 → 13: Auto and Match (CL-019, CL-020).
//!
//! Version 13 added `auto` (an auto correction's slider offsets, style,
//! source and Strength) and `match` (a colour match's gains, offsets,
//! Intensity and reference) inside a clip's `color`, each left out while
//! the clip has none, plus keyframes of `color.auto.strength` and
//! `color.match.intensity`. None of them existed before, so every
//! version-12 stack has no auto correction and no match, which is what the
//! missing keys mean.
//!
//! So the step writes nothing. It exists so the version number records
//! that a version-12 build can't read what version 13 writes.

use serde_json::Value;

pub(super) fn migrate(doc: Value) -> Result<Value, String> {
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_version_12_document_needs_no_change() {
        let doc = json!({
            "document": {"sequences": [{"visual": [{"clips": [
                {"id": 40, "color": {"lut": {"hash": "00112233445566778899aabbccddeeff", "file": "a.cube"}}}
            ]}]}]}
        });
        assert_eq!(migrate(doc.clone()), Ok(doc));
    }
}
