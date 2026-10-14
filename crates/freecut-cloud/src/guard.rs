//! The preflight guard (architecture §4.7).
//!
//! A cost check and the paid run are the same tool call; only `get_cost:
//! true` tells them apart. A dropped or renamed flag would turn every check
//! into a paid run, so:
//!
//! 1. [`QuoteRequest`] is the only way to build a check, and it always sets
//!    the flag; [`verify_outgoing_preflight`] re-reads the serialized request
//!    and refuses to send one without it; checks go out only through
//!    [`McpClient::check_cost`](crate::mcp::McpClient::check_cost), which
//!    takes a `QuoteRequest` and calls this for every one;
//! 2. a check is sent only while the tool's live schema still offers
//!    `get_cost` as a boolean ([`schema_offers_get_cost`]);
//! 3. checks are retried only while nothing was sent (the MCP client's
//!    preflight class);
//! 4. an answer that carries job ids is a run that started
//!    ([`job_ids_in`]); its tool's checks stop until restart
//!    ([`PreflightGuard`]).

use std::collections::BTreeSet;
use std::sync::{Mutex, PoisonError};

use serde_json::Value;

use crate::mcp::schema;

/// The guard refused to build or send a cost check. Always a FreeCut bug,
/// never a user error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GuardViolation {
    /// The arguments have no `params` object to put the flag in.
    #[error("the cost check has no params object")]
    NoParams,
    /// The outgoing message is not a `tools/call`.
    #[error("the outgoing cost check is not a tools/call")]
    NotAToolCall,
    /// The outgoing message lacks `params.arguments.params.get_cost: true`.
    #[error("the outgoing cost check does not carry get_cost: true")]
    FlagMissing,
    /// The outgoing message is not JSON.
    #[error("the outgoing cost check cannot be read back")]
    Unreadable,
    /// The tool offers no cost check.
    #[error("the tool offers no cost check")]
    NoCostCheck,
    /// The cost check was built for another tool.
    #[error("the cost check was built for another tool")]
    OtherTool,
}

/// A cost check's arguments. The only constructor sets `get_cost: true`.
#[derive(Debug, Clone, PartialEq)]
pub struct QuoteRequest {
    tool: String,
    arguments: Value,
}

impl QuoteRequest {
    /// Turns a tool call's arguments into a cost check for the same call.
    ///
    /// # Errors
    /// [`GuardViolation::NoParams`] when `arguments.params` is not an object.
    pub fn new(tool: &str, mut arguments: Value) -> Result<QuoteRequest, GuardViolation> {
        let params = arguments
            .get_mut("params")
            .and_then(Value::as_object_mut)
            .ok_or(GuardViolation::NoParams)?;
        params.insert("get_cost".into(), Value::Bool(true));
        Ok(QuoteRequest {
            tool: tool.to_owned(),
            arguments,
        })
    }

    /// The tool.
    pub fn tool(&self) -> &str {
        &self.tool
    }

    /// The arguments, flag included.
    pub fn arguments(&self) -> &Value {
        &self.arguments
    }
}

/// Re-reads a serialized JSON-RPC request and checks that it is a
/// `tools/call` whose `arguments.params.get_cost` is `true`.
///
/// # Errors
/// [`GuardViolation`] when it is not.
pub fn verify_outgoing_preflight(body: &[u8]) -> Result<(), GuardViolation> {
    let message: Value = serde_json::from_slice(body).map_err(|_| GuardViolation::Unreadable)?;
    if message.get("method").and_then(Value::as_str) != Some("tools/call") {
        return Err(GuardViolation::NotAToolCall);
    }
    match message.pointer("/params/arguments/params/get_cost") {
        Some(Value::Bool(true)) => Ok(()),
        _ => Err(GuardViolation::FlagMissing),
    }
}

/// True while the tool's live input schema lists `params.get_cost` as a
/// boolean.
pub fn schema_offers_get_cost(input_schema: &Value) -> bool {
    schema::property_types(input_schema, &["params", "get_cost"])
        .iter()
        .any(|t| t == "boolean")
}

/// Every UUID-shaped string anywhere in a cost-check answer. A cost answer
/// (`{"cost": {...}}`) carries none; the service's job ids are UUIDs, so any
/// UUID means a job may have been created and the answer is treated as a
/// started run.
pub fn job_ids_in(answer: &Value) -> Vec<String> {
    let mut out = Vec::new();
    collect_uuids(answer, &mut out);
    out.sort();
    out.dedup();
    out
}

fn collect_uuids(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(s) if is_uuid(s) => out.push(s.to_ascii_lowercase()),
        Value::Array(items) => items.iter().for_each(|v| collect_uuids(v, out)),
        Value::Object(map) => map.values().for_each(|v| collect_uuids(v, out)),
        _ => {}
    }
}

/// Every UUID-shaped token in raw text, lower-case and deduplicated: used on
/// late answers that may not parse, so no job id in them is lost.
pub fn uuids_in_text(text: &str) -> Vec<String> {
    let mut out: Vec<String> = text
        .split(|c: char| !(c.is_ascii_hexdigit() || c == '-'))
        .filter(|t| is_uuid(t))
        .map(str::to_ascii_lowercase)
        .collect();
    out.sort();
    out.dedup();
    out
}

/// `8-4-4-4-12` hexadecimal digits.
pub fn is_uuid(text: &str) -> bool {
    let groups: Vec<&str> = text.split('-').collect();
    groups.len() == 5
        && groups
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(g, len)| g.len() == len && g.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Tools whose cost check started a run; their checks stay off until FreeCut
/// restarts (§4.7 rule 4).
#[derive(Debug, Default)]
pub struct PreflightGuard {
    blocked: Mutex<BTreeSet<String>>,
}

impl PreflightGuard {
    /// No tool blocked.
    pub fn new() -> PreflightGuard {
        PreflightGuard::default()
    }

    /// True once a check of `tool` started a run.
    pub fn is_blocked(&self, tool: &str) -> bool {
        self.blocked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(tool)
    }

    /// Stops checks of `tool` for the rest of this run of FreeCut.
    pub fn block(&self, tool: &str) {
        self.blocked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(tool.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_only_constructor_sets_the_flag() {
        let q = QuoteRequest::new(
            "generate_image",
            json!({"params": {"model": "m", "get_cost": false}}),
        )
        .unwrap();
        assert_eq!(
            q.arguments(),
            &json!({"params": {"model": "m", "get_cost": true}})
        );
        assert_eq!(
            QuoteRequest::new("t", json!({"params": "m"})),
            Err(GuardViolation::NoParams)
        );
        assert_eq!(
            QuoteRequest::new("t", json!({})),
            Err(GuardViolation::NoParams)
        );
    }

    #[test]
    fn outgoing_checks_must_carry_the_flag_where_expected() {
        let ok = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "generate_image", "arguments": {"params": {"model": "m", "get_cost": true}}}});
        assert_eq!(verify_outgoing_preflight(ok.to_string().as_bytes()), Ok(()));
        for bad in [
            json!({"method": "tools/call", "params": {"arguments": {"params": {"model": "m"}}}}),
            json!({"method": "tools/call", "params": {"arguments": {"params": {"get_cost": "true"}}}}),
            json!({"method": "tools/call", "params": {"arguments": {"get_cost": true, "params": {}}}}),
            json!({"method": "tools/call", "params": {"arguments": {"params": {"getCost": true}}}}),
        ] {
            assert_eq!(
                verify_outgoing_preflight(bad.to_string().as_bytes()),
                Err(GuardViolation::FlagMissing),
                "{bad}"
            );
        }
        assert_eq!(
            verify_outgoing_preflight(br#"{"method":"tools/list"}"#),
            Err(GuardViolation::NotAToolCall)
        );
        assert_eq!(
            verify_outgoing_preflight(b"nope"),
            Err(GuardViolation::Unreadable)
        );
    }

    #[test]
    fn a_schema_without_a_boolean_get_cost_offers_no_check() {
        let with = json!({"properties": {"params": {"anyOf": [
            {"type": "object", "properties": {"get_cost": {"type": "boolean"}}}, {"type": "string"}]}}});
        let renamed = json!({"properties": {"params": {"type": "object", "properties": {"estimate": {"type": "boolean"}}}}});
        let retyped = json!({"properties": {"params": {"type": "object", "properties": {"get_cost": {"type": "string"}}}}});
        assert!(schema_offers_get_cost(&with));
        assert!(!schema_offers_get_cost(&renamed));
        assert!(!schema_offers_get_cost(&retyped));
    }

    #[test]
    fn recorded_cost_answers_carry_no_job_ids() {
        assert!(job_ids_in(&json!({"cost": {"credits": 0.25, "credits_exact": 0.25}})).is_empty());
        assert!(job_ids_in(&json!({"cost": {"credits": 35, "credits_exact": 35}})).is_empty());
    }

    #[test]
    fn job_ids_are_found_at_any_depth() {
        let answer = json!({"jobs": [{"job_id": "0B4F6D2A-1111-4222-8333-944455556666"}],
            "widget": {"ids": ["0b4f6d2a-1111-4222-8333-944455556666", "not-an-id"]}});
        assert_eq!(
            job_ids_in(&answer),
            vec!["0b4f6d2a-1111-4222-8333-944455556666"]
        );
    }

    #[test]
    fn uuids_are_found_in_raw_and_cut_off_text() {
        let text = "data: {\"jobs\":[{\"job_id\":\"0B4F6D2A-1111-4222-8333-944455556666\"},{\"job_id\":\"0b4f6d2a-1111-4222-8333-94445555";
        assert_eq!(
            uuids_in_text(text),
            vec!["0b4f6d2a-1111-4222-8333-944455556666"]
        );
        assert!(uuids_in_text("no ids, 2025-11-25").is_empty());
    }

    #[test]
    fn uuid_shape() {
        assert!(is_uuid("123e4567-e89b-12d3-a456-426614174000"));
        assert!(!is_uuid("123e4567e89b12d3a456426614174000"));
        assert!(!is_uuid("123e4567-e89b-12d3-a456-42661417400g"));
    }

    #[test]
    fn blocking_is_per_tool() {
        let g = PreflightGuard::new();
        g.block("generate_video");
        assert!(g.is_blocked("generate_video"));
        assert!(!g.is_blocked("generate_image"));
    }
}
