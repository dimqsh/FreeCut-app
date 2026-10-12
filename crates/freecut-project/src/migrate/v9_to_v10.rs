//! Format 9 → 10: colour stacks (CL-001, CL-007, CL-008, CL-023, CL-035).
//!
//! Version 10 added `color` on clips (the default stack is left out) and
//! keyframes of `color.*` properties. Neither existed before, so every
//! version-9 clip shows its footage ungraded, which is what a missing key
//! means.
//!
//! So the step writes nothing. It exists so the version number records
//! that a version-9 build can't read what version 10 writes.

use serde_json::Value;

pub(super) fn migrate(doc: Value) -> Result<Value, String> {
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_version_9_document_needs_no_change() {
        let doc = json!({
            "document": {"sequences": [{"visual": [{"clips": [{"id": 40}]}]}]}
        });
        assert_eq!(migrate(doc.clone()), Ok(doc));
    }
}
