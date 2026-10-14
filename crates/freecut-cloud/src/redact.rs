//! Redaction for anything that may reach a log or a fixture (§5.6, §11):
//! tokens, codes and verifiers are replaced before formatting.

use serde_json::Value;

/// JSON keys and form fields whose values are never logged.
pub const SECRET_FIELDS: &[&str] = &[
    "access_token",
    "refresh_token",
    "id_token",
    "code",
    "code_verifier",
    "device_code",
    "client_secret",
    "registration_access_token",
    "token",
    "authorization",
];

/// What replaces a secret value.
pub const REDACTED: &str = "<redacted>";

fn is_secret_field(name: &str) -> bool {
    SECRET_FIELDS.iter().any(|f| f.eq_ignore_ascii_case(name))
}

/// A copy of `value` with every secret field's value replaced, at any depth.
pub fn json(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| {
                    let v = if is_secret_field(k) {
                        Value::String(REDACTED.into())
                    } else {
                        json(v)
                    };
                    (k.clone(), v)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(json).collect()),
        other => other.clone(),
    }
}

/// A form body (`a=1&code=…`) with secret fields replaced.
pub fn form(body: &str) -> String {
    let pairs = url::form_urlencoded::parse(body.as_bytes()).map(|(k, v)| {
        let v = if is_secret_field(&k) {
            REDACTED.into()
        } else {
            v.into_owned()
        };
        (k.into_owned(), v)
    });
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish()
}

/// A request or response body for a trace log line: JSON and forms are
/// redacted, anything else is reduced to its length.
pub fn body(bytes: &[u8]) -> String {
    if let Ok(value) = serde_json::from_slice::<Value>(bytes) {
        return json(&value).to_string();
    }
    match std::str::from_utf8(bytes) {
        Ok(text) if text.contains('=') && !text.contains(char::is_whitespace) => form(text),
        _ => format!("<{} bytes>", bytes.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn nested_json_secrets_are_replaced() {
        let v =
            json!({"a": 1, "nested": [{"refresh_token": "r1", "keep": "x"}], "access_token": "t"});
        let r = json(&v);
        assert_eq!(
            r,
            json!({"a": 1, "nested": [{"refresh_token": REDACTED, "keep": "x"}], "access_token": REDACTED})
        );
    }

    #[test]
    fn form_secrets_are_replaced() {
        let r = form("grant_type=authorization_code&code=abc&code_verifier=xyz&client_id=c1");
        assert_eq!(
            r,
            "grant_type=authorization_code&code=%3Credacted%3E&code_verifier=%3Credacted%3E&client_id=c1"
        );
    }

    #[test]
    fn unknown_bodies_show_only_their_length() {
        assert_eq!(body(b"not json at all"), "<15 bytes>");
    }
}
