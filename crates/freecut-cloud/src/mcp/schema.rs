//! A small JSON Schema checker for tool arguments (architecture §4.6).
//!
//! It covers what tool input schemas use: `type`, `enum`, `const`,
//! `required`, `properties`, `additionalProperties`, `items`, numeric and
//! length bounds, and `anyOf`/`oneOf`/`allOf`. Other keywords (`format`,
//! `pattern`, `default`, …) are ignored, so the check can only be more
//! permissive than the server's, never stricter.

use serde_json::{Map, Value};

/// Where and why arguments do not match a schema.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{path}: {problem}")]
pub struct SchemaViolation {
    /// A JSON-pointer-like path to the offending value (`/params/count`).
    pub path: String,
    /// What is wrong.
    pub problem: String,
}

/// Checks `value` against `schema`.
///
/// # Errors
/// The first [`SchemaViolation`] found.
pub fn validate(schema: &Value, value: &Value) -> Result<(), SchemaViolation> {
    check(schema, value, "")
}

fn violation(path: &str, problem: impl Into<String>) -> SchemaViolation {
    SchemaViolation {
        path: if path.is_empty() {
            "/".into()
        } else {
            path.into()
        },
        problem: problem.into(),
    }
}

fn check(schema: &Value, value: &Value, path: &str) -> Result<(), SchemaViolation> {
    let schema = match schema {
        Value::Bool(true) => return Ok(()),
        Value::Bool(false) => return Err(violation(path, "no value is allowed here")),
        Value::Object(s) => s,
        _ => return Ok(()),
    };

    if let Some(types) = schema.get("type") {
        let allowed: Vec<&str> = match types {
            Value::String(t) => vec![t.as_str()],
            Value::Array(ts) => ts.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        };
        if !allowed.is_empty() && !allowed.iter().any(|t| has_type(value, t)) {
            return Err(violation(
                path,
                format!(
                    "expected {}, got {}",
                    allowed.join(" or "),
                    type_name(value)
                ),
            ));
        }
    }
    if let Some(Value::Array(options)) = schema.get("enum")
        && !options.iter().any(|o| json_eq(o, value))
    {
        return Err(violation(path, "not one of the allowed values"));
    }
    if let Some(expected) = schema.get("const")
        && !json_eq(expected, value)
    {
        return Err(violation(path, "not the required value"));
    }
    check_number(schema, value, path)?;
    check_lengths(schema, value, path)?;

    if let Value::Object(obj) = value {
        check_object(schema, obj, path)?;
    }
    if let (Some(items), Value::Array(values)) = (schema.get("items"), value) {
        for (i, item) in values.iter().enumerate() {
            check(items, item, &format!("{path}/{i}"))?;
        }
    }

    if let Some(Value::Array(all)) = schema.get("allOf") {
        for sub in all {
            check(sub, value, path)?;
        }
    }
    for key in ["anyOf", "oneOf"] {
        if let Some(Value::Array(branches)) = schema.get(key)
            && !branches.is_empty()
        {
            let mut first_err = None;
            let mut matched = false;
            for branch in branches {
                match check(branch, value, path) {
                    Ok(()) => {
                        matched = true;
                        break;
                    }
                    Err(e) => {
                        first_err.get_or_insert(e);
                    }
                }
            }
            if !matched {
                return Err(first_err.unwrap_or_else(|| violation(path, "matches no alternative")));
            }
        }
    }
    Ok(())
}

fn check_object(
    schema: &Map<String, Value>,
    obj: &Map<String, Value>,
    path: &str,
) -> Result<(), SchemaViolation> {
    if let Some(Value::Array(required)) = schema.get("required") {
        for name in required.iter().filter_map(Value::as_str) {
            if !obj.contains_key(name) {
                return Err(violation(
                    path,
                    format!("missing required field \"{name}\""),
                ));
            }
        }
    }
    let props = schema.get("properties").and_then(Value::as_object);
    for (name, v) in obj {
        let child = format!("{path}/{name}");
        match props.and_then(|p| p.get(name)) {
            Some(sub) => check(sub, v, &child)?,
            None => match schema.get("additionalProperties") {
                Some(Value::Bool(false)) => {
                    return Err(violation(&child, "field not allowed"));
                }
                Some(extra @ Value::Object(_)) => check(extra, v, &child)?,
                _ => {}
            },
        }
    }
    Ok(())
}

fn check_number(
    schema: &Map<String, Value>,
    value: &Value,
    path: &str,
) -> Result<(), SchemaViolation> {
    let Some(n) = value.as_f64() else {
        return Ok(());
    };
    // Bounds are compared as floats: this is argument validation, not time.
    let bound = |k: &str| schema.get(k).and_then(Value::as_f64);
    if let Some(min) = bound("minimum")
        && n < min
    {
        return Err(violation(path, format!("below the minimum {min}")));
    }
    if let Some(max) = bound("maximum")
        && n > max
    {
        return Err(violation(path, format!("above the maximum {max}")));
    }
    if let Some(min) = bound("exclusiveMinimum")
        && n <= min
    {
        return Err(violation(path, format!("not above {min}")));
    }
    if let Some(max) = bound("exclusiveMaximum")
        && n >= max
    {
        return Err(violation(path, format!("not below {max}")));
    }
    Ok(())
}

fn check_lengths(
    schema: &Map<String, Value>,
    value: &Value,
    path: &str,
) -> Result<(), SchemaViolation> {
    let len = match value {
        Value::String(s) => s.chars().count(),
        Value::Array(a) => a.len(),
        _ => return Ok(()),
    };
    let (min_key, max_key) = if value.is_string() {
        ("minLength", "maxLength")
    } else {
        ("minItems", "maxItems")
    };
    let as_len = |k: &str| {
        schema
            .get(k)
            .and_then(Value::as_u64)
            .and_then(|v| usize::try_from(v).ok())
    };
    if let Some(min) = as_len(min_key)
        && len < min
    {
        return Err(violation(path, format!("shorter than {min}")));
    }
    if let Some(max) = as_len(max_key)
        && len > max
    {
        return Err(violation(path, format!("longer than {max}")));
    }
    Ok(())
}

fn has_type(value: &Value, t: &str) -> bool {
    match t {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "string" => value.is_string(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        "number" => value.is_number(),
        "integer" => {
            value.is_i64()
                || value.is_u64()
                || value
                    .as_f64()
                    .is_some_and(|f| f.is_finite() && f.fract() == 0.0)
        }
        _ => true,
    }
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn json_eq(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) if a.is_number() && b.is_number() => x == y,
        _ => a == b,
    }
}

/// The types a schema allows at a property path, looking through
/// `anyOf`/`oneOf`/`allOf` branches. Empty when the path is not described.
pub fn property_types(schema: &Value, path: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    collect_types(schema, path, &mut found);
    found.sort();
    found.dedup();
    found
}

fn collect_types(schema: &Value, path: &[&str], out: &mut Vec<String>) {
    let Some(obj) = schema.as_object() else {
        return;
    };
    for key in ["anyOf", "oneOf", "allOf"] {
        if let Some(Value::Array(branches)) = obj.get(key) {
            for branch in branches {
                collect_types(branch, path, out);
            }
        }
    }
    match path.split_first() {
        None => match obj.get("type") {
            Some(Value::String(t)) => out.push(t.clone()),
            Some(Value::Array(ts)) => {
                out.extend(ts.iter().filter_map(Value::as_str).map(str::to_owned))
            }
            _ => {}
        },
        Some((first, rest)) => {
            if let Some(sub) = obj.get("properties").and_then(|p| p.get(*first)) {
                collect_types(sub, rest, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The shape of `generate_image`'s live schema (recorded 2026-10-14),
    /// trimmed to the parts FreeCut relies on.
    fn generate_schema() -> Value {
        json!({
            "type": "object",
            "required": ["params"],
            "properties": {
                "context": {"type": "string", "maxLength": 500},
                "params": {"anyOf": [
                    {"type": "object", "additionalProperties": {}, "required": ["model"], "properties": {
                        "model": {"type": "string"},
                        "prompt": {"type": "string"},
                        "aspect_ratio": {"type": "string"},
                        "count": {"type": "integer", "minimum": 1, "maximum": 4, "default": 1},
                        "get_cost": {"type": "boolean"},
                        "medias": {"type": "array", "items": {"type": "object", "required": ["value", "role"],
                            "properties": {"role": {"type": "string"}, "value": {"type": "string"}}}}
                    }},
                    {"type": "string"}
                ]}
            }
        })
    }

    #[test]
    fn valid_arguments_pass() {
        let args = json!({"params": {"model": "m", "count": 2, "get_cost": true, "quality": "low",
            "medias": [{"value": "u", "role": "image_references"}]}});
        assert_eq!(validate(&generate_schema(), &args), Ok(()));
    }

    #[test]
    fn violations_name_the_path() {
        let s = generate_schema();
        let err = validate(&s, &json!({"params": {"model": "m", "count": 9}})).unwrap_err();
        assert_eq!(err.path, "/params/count");
        let err = validate(&s, &json!({"params": {"prompt": "x"}})).unwrap_err();
        assert!(err.problem.contains("model"), "{err}");
        let err = validate(&s, &json!({})).unwrap_err();
        assert!(err.problem.contains("params"), "{err}");
        let err = validate(
            &s,
            &json!({"params": {"model": "m", "medias": [{"value": "u"}]}}),
        )
        .unwrap_err();
        assert_eq!(err.path, "/params/medias/0");
    }

    #[test]
    fn keywords_on_scalars() {
        assert!(validate(&json!({"enum": ["a", "b"]}), &json!("c")).is_err());
        assert!(validate(&json!({"type": "integer"}), &json!(2.0)).is_ok());
        assert!(validate(&json!({"type": "integer"}), &json!(2.5)).is_err());
        assert!(validate(&json!({"type": ["string", "null"]}), &json!(null)).is_ok());
        assert!(validate(&json!({"maxLength": 3}), &json!("abcd")).is_err());
        assert!(validate(&json!({"exclusiveMinimum": 0}), &json!(0)).is_err());
        assert!(
            validate(
                &json!({"additionalProperties": false, "properties": {"a": {}}}),
                &json!({"b": 1})
            )
            .is_err()
        );
        assert!(validate(&json!(false), &json!(1)).is_err());
    }

    #[test]
    fn property_types_look_through_alternatives() {
        let s = generate_schema();
        assert_eq!(property_types(&s, &["params", "get_cost"]), vec!["boolean"]);
        assert_eq!(property_types(&s, &["params", "count"]), vec!["integer"]);
        assert!(property_types(&s, &["params", "nope"]).is_empty());
        assert_eq!(property_types(&s, &["params"]), vec!["object", "string"]);
    }
}
