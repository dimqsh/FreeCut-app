//! JSON-RPC 2.0 framing for MCP.

use serde_json::{Map, Value, json};

/// A JSON-RPC error object.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("JSON-RPC error {code}: {message}")]
pub struct RpcError {
    /// The error code.
    pub code: i64,
    /// The server's message (untrusted text).
    pub message: String,
    /// Extra data, if any.
    pub data: Option<Value>,
}

/// "Method not found", FreeCut's answer to server requests it does not serve.
pub const METHOD_NOT_FOUND: i64 = -32601;

/// A message received from the server.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    /// The answer to a request.
    Response {
        /// The request id it answers.
        id: Value,
        /// The result, or the error.
        outcome: Result<Value, RpcError>,
    },
    /// A request from the server to FreeCut.
    Request {
        /// Its id, echoed in the answer.
        id: Value,
        /// The method.
        method: String,
    },
    /// A notification from the server.
    Notification {
        /// The method.
        method: String,
    },
}

/// A request object.
pub fn request(id: i64, method: &str, params: Option<Value>) -> Value {
    let mut msg = Map::new();
    msg.insert("jsonrpc".into(), "2.0".into());
    msg.insert("id".into(), id.into());
    msg.insert("method".into(), method.into());
    if let Some(params) = params {
        msg.insert("params".into(), params);
    }
    Value::Object(msg)
}

/// A notification object.
pub fn notification(method: &str, params: Option<Value>) -> Value {
    let mut msg = Map::new();
    msg.insert("jsonrpc".into(), "2.0".into());
    msg.insert("method".into(), method.into());
    if let Some(params) = params {
        msg.insert("params".into(), params);
    }
    Value::Object(msg)
}

/// A successful answer to a server request.
pub fn result(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// An error answer to a server request.
pub fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// Splits a body into messages: one object, or an array of them.
///
/// # Errors
/// A description when the value is neither.
pub fn messages(value: Value) -> Result<Vec<Value>, String> {
    match value {
        Value::Array(items) => Ok(items),
        obj @ Value::Object(_) => Ok(vec![obj]),
        other => Err(format!("expected a JSON-RPC message, got {}", kind(&other))),
    }
}

/// Classifies one message.
///
/// # Errors
/// A description when the message is not valid JSON-RPC 2.0.
pub fn parse(message: &Value) -> Result<Incoming, String> {
    let Value::Object(obj) = message else {
        return Err(format!("expected a JSON-RPC object, got {}", kind(message)));
    };
    if obj.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err("missing \"jsonrpc\": \"2.0\"".into());
    }
    if let Some(method) = obj.get("method").and_then(Value::as_str) {
        return Ok(match obj.get("id") {
            Some(id) if !id.is_null() => Incoming::Request {
                id: id.clone(),
                method: method.to_owned(),
            },
            _ => Incoming::Notification {
                method: method.to_owned(),
            },
        });
    }
    let id = obj.get("id").cloned().unwrap_or(Value::Null);
    if let Some(result) = obj.get("result") {
        return Ok(Incoming::Response {
            id,
            outcome: Ok(result.clone()),
        });
    }
    if let Some(err) = obj.get("error") {
        let code = err.get("code").and_then(Value::as_i64).unwrap_or(0);
        let message = err
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        return Ok(Incoming::Response {
            id,
            outcome: Err(RpcError {
                code,
                message,
                data: err.get("data").cloned(),
            }),
        });
    }
    Err("a JSON-RPC message with neither method, result nor error".into())
}

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn framing() {
        assert_eq!(
            request(3, "tools/list", None).to_string(),
            r#"{"id":3,"jsonrpc":"2.0","method":"tools/list"}"#
        );
        assert_eq!(
            notification("notifications/initialized", None).to_string(),
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#
        );
    }

    #[test]
    fn classifies_each_message_kind() {
        assert_eq!(
            parse(&json!({"jsonrpc":"2.0","id":1,"result":{"ok":true}})).unwrap(),
            Incoming::Response {
                id: json!(1),
                outcome: Ok(json!({"ok": true}))
            }
        );
        assert_eq!(
            parse(&json!({"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"bad"}}))
                .unwrap(),
            Incoming::Response {
                id: json!(1),
                outcome: Err(RpcError {
                    code: -32602,
                    message: "bad".into(),
                    data: None
                })
            }
        );
        assert_eq!(
            parse(&json!({"jsonrpc":"2.0","id":"s1","method":"ping"})).unwrap(),
            Incoming::Request {
                id: json!("s1"),
                method: "ping".into()
            }
        );
        assert_eq!(
            parse(&json!({"jsonrpc":"2.0","method":"notifications/progress","params":{}})).unwrap(),
            Incoming::Notification {
                method: "notifications/progress".into()
            }
        );
        assert!(parse(&json!({"id":1,"result":{}})).is_err());
        assert!(parse(&json!({"jsonrpc":"2.0","id":1})).is_err());
        assert!(parse(&json!([1])).is_err());
    }

    #[test]
    fn batches_split_into_messages() {
        assert_eq!(messages(json!([{"a":1},{"b":2}])).unwrap().len(), 2);
        assert_eq!(messages(json!({"a":1})).unwrap().len(), 1);
        assert!(messages(json!("x")).is_err());
    }
}
