//! Format 5 → 6: keyframes and animations (TR-027, TR-031, TX-021, FX-018,
//! FX-019, TX-022, S2).
//!
//! Version 6 added, on every clip:
//!
//! - `keys`: keyframes on transform and text properties, with the clip's
//!   content-time `origin`. Left out while a clip has none and its origin
//!   is 0, which is exactly what every version-5 clip had.
//! - `animations`: the In, Out and Loop slots. Left out while all are
//!   empty, which is what every version-5 clip had.
//!
//! and widened `interp` on volume keyframes from `"linear"` and `"hold"` to
//! TR-031's easing presets (`"ease-in-out-cubic"`, `"bounce-out"`, …);
//! every version-5 value keeps its meaning.
//!
//! So the step writes nothing: an absent key already means the version-5
//! state. It exists so the version number records that a version-5 build
//! can't read what version 6 writes.

use serde_json::Value;

pub(super) fn migrate(doc: Value) -> Result<Value, String> {
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_version_5_document_needs_no_change() {
        let doc = json!({
            "document": {"sequences": [{"visual": [{"clips": [
                {"id": 1, "transform": {"opacity": 500},
                 "audio": {"type": "active", "gain_keys": [{"at": 0, "gain": 0, "interp": "hold"}]}}
            ]}]}]}
        });
        assert_eq!(migrate(doc.clone()), Ok(doc));
    }
}
