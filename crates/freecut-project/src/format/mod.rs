//! The project file format: envelope constants, the canonical writer and the
//! typed decoder.
//!
//! A project file is one UTF-8 JSON object (PM-021):
//!
//! ```json
//! {
//!   "format": "freecut-project",
//!   "format_version": 9,
//!   "app": "freecut 0.1.0",
//!   "ids_next": 58,
//!   "document": { "root": 1, "settings": {…}, "library": […], "sequences": […] },
//!   "export": null,
//!   "ui": { "solo": [12] }
//! }
//! ```
//!
//! The bytes are canonical: keys in schema order (unknown keys after the
//! known ones, sorted), 2-space indentation, `\n` line endings on every OS
//! and a trailing newline. Saving the same document twice gives the same
//! bytes, so projects diff cleanly in version control.
//!
//! Values of unknown keys are written back as read, numbers included:
//! serde_json's `arbitrary_precision` feature keeps each number's digits,
//! and [`carry_wide_integers`] gets integers wider than 64 bits through
//! serde's `flatten` buffer.

pub(crate) mod dto;
pub(crate) mod map;

use core::num::NonZeroU64;

use serde::Serialize;
use serde_json::Value;
use serde_json::ser::{PrettyFormatter, Serializer};

use crate::export::ExportSettings;
use crate::extras::Extras;
use crate::open::OpenError;
use crate::ui::UiState;
use freecut_timeline::model::Document;

pub use crate::migrate::CURRENT_FORMAT_VERSION;

/// The value of the envelope's `format` key.
pub const FORMAT_MAGIC: &str = "freecut-project";

/// The writer identity stored in the envelope's `app` key.
pub const APP_ID: &str = concat!("freecut ", env!("CARGO_PKG_VERSION"));

/// Canonical bytes for an already validated document whose numbers passed
/// [`map::check_writable`].
pub(crate) fn to_bytes(
    doc: &Document,
    ids_next: NonZeroU64,
    export: Option<&ExportSettings>,
    ui: &UiState,
    extras: &Extras,
    relative: map::RelativePaths<'_>,
) -> Result<Vec<u8>, serde_json::Error> {
    canonical(&map::write(doc, ids_next, export, ui, extras, relative))
}

/// Serializes `value` canonically: 2-space indent, `\n`, trailing newline.
fn canonical<T: Serialize>(value: &T) -> Result<Vec<u8>, serde_json::Error> {
    let mut out = Vec::with_capacity(16 * 1024);
    let mut ser = Serializer::with_formatter(&mut out, PrettyFormatter::with_indent(b"  "));
    value.serialize(&mut ser)?;
    out.push(b'\n');
    Ok(out)
}

/// Parses `bytes` into a JSON value. A syntax error names the JSON path of
/// the value being read when it happened.
pub(crate) fn parse(bytes: &[u8]) -> Result<Value, OpenError> {
    let mut de = serde_json::Deserializer::from_slice(bytes);
    let value: Value = serde_path_to_error::deserialize(&mut de).map_err(json_error)?;
    de.end().map_err(|e| OpenError::Json {
        path: ".".to_string(),
        message: e.to_string(),
    })?;
    Ok(value)
}

/// Decodes a migrated envelope into model parts.
pub(crate) fn decode(mut value: Value) -> Result<map::Parts, OpenError> {
    carry_wide_integers(&mut value);
    let env: dto::EnvelopeDto = serde_path_to_error::deserialize(value).map_err(json_error)?;
    map::read(env)
}

/// serde_json's name for a number kept as its decimal text (the
/// `arbitrary_precision` feature). A `Value` reads an object with this one
/// key back as that number. The unit test below fails if serde_json ever
/// changes it.
const NUMBER_TOKEN: &str = "$serde_json::private::Number";

/// Wraps integers that need more than 64 bits in serde_json's
/// arbitrary-precision object form.
///
/// Unknown keys reach [`Extras`] through serde's `#[serde(flatten)]` buffer,
/// which has no 128-bit integers: an unknown key holding one would make the
/// file unreadable, and without `arbitrary_precision` it would come back as
/// a rounded float. In the object form the buffer carries the digits and the
/// unknown-key map turns them back into the same number, written back
/// verbatim (PM-021). Such a number in a known field is still refused (it
/// can't be an id or a tick).
pub(crate) fn carry_wide_integers(value: &mut Value) {
    match value {
        Value::Number(n) if !n.is_u64() && !n.is_i64() && !n.is_f64() => {
            let digits = Value::String(n.to_string());
            let mut wrapped = serde_json::Map::new();
            wrapped.insert(NUMBER_TOKEN.to_string(), digits);
            *value = Value::Object(wrapped);
        }
        Value::Array(items) => items.iter_mut().for_each(carry_wide_integers),
        Value::Object(map) => map.values_mut().for_each(carry_wide_integers),
        _ => {}
    }
}

fn json_error(err: serde_path_to_error::Error<serde_json::Error>) -> OpenError {
    OpenError::Json {
        path: err.path().to_string(),
        message: err.inner().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_integers_are_carried_as_the_same_number() {
        let text = "[123456789012345678901234567890, -98765432109876543210, 1, 2.5]";
        let mut value: Value = serde_json::from_str(text).unwrap();
        let before = value.clone();
        carry_wide_integers(&mut value);
        assert!(value[0].is_object() && value[1].is_object());
        assert_eq!(value[2], before[2]);
        assert_eq!(value[3], before[3]);
        // `Value` turns the wrapped form back into the number, digit for digit.
        let back: Vec<Value> = serde_json::from_value(value).unwrap();
        assert_eq!(back, before.as_array().unwrap().clone());
        assert_eq!(
            serde_json::to_string(&back).unwrap(),
            "[123456789012345678901234567890,-98765432109876543210,1,2.5]"
        );
    }
}
