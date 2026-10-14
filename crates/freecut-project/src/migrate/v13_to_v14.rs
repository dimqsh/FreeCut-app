//! Format 13 → 14: colour masks (CL-031).
//!
//! Version 14 added `masks` inside a clip's `color`: a list of shape masks
//! (shape, centre, size, link, rotation, feather, invert, the shape's own
//! value, Expand and combine mode, each left out at its default), left out
//! while the stack has none, plus keyframes of `color.masks.<id>.<value>`.
//! None of them existed before, so every version-13 stack has no masks,
//! which is what the missing list means.
//!
//! So the step writes nothing. It exists so the version number records
//! that a version-13 build can't read what version 14 writes.

use serde_json::Value;

pub(super) fn migrate(doc: Value) -> Result<Value, String> {
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_version_13_document_needs_no_change() {
        let doc = json!({
            "document": {"sequences": [{"visual": [{"clips": [
                {"id": 40, "color": {"auto": {"offsets": {"exposure": 12}}}}
            ]}]}]}
        });
        assert_eq!(migrate(doc.clone()), Ok(doc));
    }
}
