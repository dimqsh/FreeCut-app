//! Format versions and the migration runner (PM-021).
//!
//! `format_version` is an integer, bumped for every schema change. Each bump
//! adds one pure step `fn(Value) -> Result<Value, String>` that upgrades a
//! document from version `n` to `n + 1`. On open, the runner applies the
//! steps from the file's version up to [`CURRENT_FORMAT_VERSION`], each
//! exactly once and in order, and sets `format_version` itself after every
//! step, so a step never sees a document that is already upgraded.
//!
//! Steps work on `serde_json::Value`, before typed decoding, because the
//! current DTOs can't represent old layouts. They must not read the clock,
//! the file system or anything outside the value. When a later version
//! changes what an absent or default value means, its step writes the old
//! meaning explicitly into the document.
//!
//! | Version | Change | Step |
//! |---|---|---|
//! | 1 | The first format. | |
//! | 2 | Clip sound (`audio` on every clip) and media sources (`source` on every library item). | `v1_to_v2` |
//! | 3 | Text clips (`"type": "text"` content with text, style and layout), clip `transform` (absent = identity), saved `export` settings. | `v2_to_v3` |
//! | 4 | Volume keyframes (`gain_keys`) and fade curves on active sound, the chosen audio `stream` of media clips, audio stream `default`/`language`/`title`/`layout`, track `mix` (absent = defaults), envelope `ui` with the solo set (absent = empty). | `v3_to_v4` |
//! | 5 | Text round 2: `style.fill` is a tagged object (solid, linear or radial gradient); `style.underline`/`strike`/`case`/`letter_spacing`; `layout.justify`/`v_anchor`; the style layers `outline`, `shadow`, `glow`, `background`, `bubble`, `curve` (absent = off with factory settings). | `v4_to_v5` |
//! | 6 | Keyframes and animations: clip `keys` (absent = none, origin 0) and `animations` (absent = no slots); volume keyframe `interp` gains TR-031's easing presets. | `v5_to_v6` |
//! | 7 | Markers (TL-037): `markers` on sequences and clips (absent = none). | `v6_to_v7` |
//! | 8 | *Follow main track* (TL-029): sequence `flags.follow_main` and `flags.follow_kinds` (absent = on, every kind), clip `attach` (absent = follows nothing). | `v7_to_v8` |
//! | 9 | Media origins (PM-041, AU-027): library item `origin` (absent = imported). | `v8_to_v9` |
//! | 10 | Colour stacks (CL-001, CL-007, CL-008, CL-023, CL-035): clip `color` (absent = the default stack) and `color.*` keyframe paths. | `v9_to_v10` |
//! | 11 | HSL, Curves and Wheels (CL-013, CL-014, CL-015): `color.hsl`, `color.curves` and `color.wheels` (absent = defaults), and `color.wheels.<wheel>.puck` keyframes. | `v10_to_v11` |
//!
//! Each superseded version keeps a frozen file written by its writer under
//! `tests/fixtures/format/v<N>/`; the tests open it with every step since.
//! Versions 7 and 8 never had a writer of their own (the toolbar round's
//! three format changes were merged together), so the tests derive their
//! files from today's instead.

mod v1_to_v2;
mod v2_to_v3;
mod v3_to_v4;
mod v4_to_v5;
mod v5_to_v6;
mod v6_to_v7;
mod v7_to_v8;
mod v8_to_v9;
mod v9_to_v10;
mod v10_to_v11;

use serde_json::Value;

/// The format version this build reads and writes.
pub const CURRENT_FORMAT_VERSION: u32 = 11;

/// The oldest format version this build can migrate from.
pub const OLDEST_READABLE: u32 = 1;

/// One migration step: upgrades a document by exactly one version.
pub type Step = fn(Value) -> Result<Value, String>;

/// `STEPS[i]` upgrades version `OLDEST_READABLE + i` to the next one.
const STEPS: &[Step] = &[
    v1_to_v2::migrate,
    v2_to_v3::migrate,
    v3_to_v4::migrate,
    v4_to_v5::migrate,
    v5_to_v6::migrate,
    v6_to_v7::migrate,
    v7_to_v8::migrate,
    v8_to_v9::migrate,
    v9_to_v10::migrate,
    v10_to_v11::migrate,
];

const _: () = assert!(
    STEPS.len() as u32 + OLDEST_READABLE == CURRENT_FORMAT_VERSION,
    "every version between OLDEST_READABLE and CURRENT_FORMAT_VERSION needs a step"
);

/// Why a document couldn't be brought to the current version.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MigrationError {
    #[error("format version {found} is older than the oldest readable version {oldest}")]
    TooOld { found: u32, oldest: u32 },
    #[error("format version {found} is newer than this build's version {supported}")]
    TooNew { found: u32, supported: u32 },
    #[error("no migration step from format version {from}")]
    MissingStep { from: u32 },
    #[error("migrating from format version {from} failed: {message}")]
    StepFailed { from: u32, message: String },
    #[error("the document is not a JSON object")]
    NotAnObject,
}

/// A document brought to the current version.
#[derive(Debug, Clone, PartialEq)]
pub struct Migrated {
    pub document: Value,
    /// The version the document had before migration.
    pub from: u32,
}

impl Migrated {
    /// Whether any step ran.
    pub fn changed(&self) -> bool {
        self.from != CURRENT_FORMAT_VERSION
    }
}

/// Brings `document`, whose `format_version` is `from`, to
/// [`CURRENT_FORMAT_VERSION`].
pub fn migrate(document: Value, from: u32) -> Result<Migrated, MigrationError> {
    run(
        document,
        from,
        OLDEST_READABLE,
        CURRENT_FORMAT_VERSION,
        STEPS,
    )
}

/// The runner, parameterized so its contract can be tested with fake steps.
fn run(
    mut document: Value,
    from: u32,
    oldest: u32,
    current: u32,
    steps: &[Step],
) -> Result<Migrated, MigrationError> {
    if from < oldest {
        return Err(MigrationError::TooOld {
            found: from,
            oldest,
        });
    }
    if from > current {
        return Err(MigrationError::TooNew {
            found: from,
            supported: current,
        });
    }
    if !document.is_object() {
        return Err(MigrationError::NotAnObject);
    }
    let mut version = from;
    while version < current {
        let index = usize::try_from(version - oldest)
            .map_err(|_| MigrationError::MissingStep { from: version })?;
        let step = steps
            .get(index)
            .ok_or(MigrationError::MissingStep { from: version })?;
        document = step(document).map_err(|message| MigrationError::StepFailed {
            from: version,
            message,
        })?;
        version += 1;
        // The runner, not the step, owns the version field.
        match document.as_object_mut() {
            Some(object) => {
                object.insert("format_version".into(), Value::from(version));
            }
            None => return Err(MigrationError::NotAnObject),
        }
    }
    Ok(Migrated { document, from })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn append(tag: &str, mut doc: Value) -> Result<Value, String> {
        let seen = doc
            .get("seen")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut seen = seen;
        seen.push(json!({
            "step": tag,
            "version_seen": doc.get("format_version").cloned().unwrap_or(Value::Null)
        }));
        doc["seen"] = Value::Array(seen);
        Ok(doc)
    }

    fn one_to_two(doc: Value) -> Result<Value, String> {
        append("1->2", doc)
    }

    fn two_to_three(doc: Value) -> Result<Value, String> {
        append("2->3", doc)
    }

    fn failing(_: Value) -> Result<Value, String> {
        Err("bad data".into())
    }

    fn not_object(_: Value) -> Result<Value, String> {
        Ok(json!([1]))
    }

    const FAKE: &[Step] = &[one_to_two, two_to_three];

    #[test]
    fn current_version_needs_no_steps() {
        let doc = json!({"format_version": CURRENT_FORMAT_VERSION, "x": 1});
        let migrated = migrate(doc.clone(), CURRENT_FORMAT_VERSION).unwrap();
        assert_eq!(migrated.document, doc);
        assert!(!migrated.changed());
    }

    #[test]
    fn steps_run_exactly_once_in_order_and_the_runner_sets_the_version() {
        let migrated = run(json!({"format_version": 1}), 1, 1, 3, FAKE).unwrap();
        assert_eq!(
            migrated.document,
            json!({
                "format_version": 3,
                "seen": [
                    {"step": "1->2", "version_seen": 1},
                    {"step": "2->3", "version_seen": 2},
                ]
            })
        );
        assert_eq!(migrated.from, 1);
    }

    #[test]
    fn migration_from_every_intermediate_version_skips_earlier_steps() {
        let migrated = run(json!({"format_version": 2}), 2, 1, 3, FAKE).unwrap();
        assert_eq!(
            migrated.document["seen"],
            json!([{"step": "2->3", "version_seen": 2}])
        );
    }

    #[test]
    fn errors_name_the_version() {
        assert_eq!(
            run(json!({}), 0, 1, 3, FAKE),
            Err(MigrationError::TooOld {
                found: 0,
                oldest: 1
            })
        );
        assert_eq!(
            run(json!({}), 4, 1, 3, FAKE),
            Err(MigrationError::TooNew {
                found: 4,
                supported: 3
            })
        );
        assert_eq!(
            run(json!({}), 1, 1, 3, &[one_to_two]),
            Err(MigrationError::MissingStep { from: 2 })
        );
        assert_eq!(
            run(json!({}), 1, 1, 2, &[failing]),
            Err(MigrationError::StepFailed {
                from: 1,
                message: "bad data".into()
            })
        );
        assert_eq!(
            run(json!({}), 1, 1, 2, &[not_object]),
            Err(MigrationError::NotAnObject)
        );
        assert_eq!(
            run(json!([]), 1, 1, 2, FAKE),
            Err(MigrationError::NotAnObject)
        );
    }
}
