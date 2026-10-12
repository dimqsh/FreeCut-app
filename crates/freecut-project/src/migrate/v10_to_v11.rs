//! Format 10 → 11: HSL, Curves and Wheels (CL-013, CL-014, CL-015).
//!
//! Version 11 added `hsl`, `curves` and `wheels` inside a clip's `color`
//! (each left out at its defaults) and keyframes of the wheels' pucks
//! (`color.wheels.<wheel>.puck`) and of the new sliders. None of them
//! existed before, so every version-10 stack has its bands at 0, its
//! curves on the diagonal and its pucks centred, which is what the missing
//! keys mean.
//!
//! So the step writes nothing. It exists so the version number records
//! that a version-10 build can't read what version 11 writes.

use serde_json::Value;

pub(super) fn migrate(doc: Value) -> Result<Value, String> {
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_version_10_document_needs_no_change() {
        let doc = json!({
            "document": {"sequences": [{"visual": [{"clips": [
                {"id": 40, "color": {"amount": 80, "light": {"exposure": 12}}}
            ]}]}]}
        });
        assert_eq!(migrate(doc.clone()), Ok(doc));
    }
}
