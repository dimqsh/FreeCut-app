//! Format 11 → 12: the LUT slot and skin protection (CL-017, CL-030).
//!
//! Version 12 added `lut` inside a clip's `color` (the LUT file by hash and
//! name, its Intensity, interpolation and skin protection) and `skin`
//! inside its Color section, each left out at its defaults, plus keyframes
//! of `color.lut.intensity`, `color.skin_amount` and
//! `color.lut.skin_amount`. None of them existed before, so every
//! version-11 stack has no LUT and no skin protection, which is what the
//! missing keys mean.
//!
//! So the step writes nothing. It exists so the version number records
//! that a version-11 build can't read what version 12 writes.

use serde_json::Value;

pub(super) fn migrate(doc: Value) -> Result<Value, String> {
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_version_11_document_needs_no_change() {
        let doc = json!({
            "document": {"sequences": [{"visual": [{"clips": [
                {"id": 40, "color": {"hsl": {"red": {"hue": 12}}, "wheels": {"strength": 80}}}
            ]}]}]}
        });
        assert_eq!(migrate(doc.clone()), Ok(doc));
    }
}
