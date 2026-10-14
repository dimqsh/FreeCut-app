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
//! 4. an answer that carries job ids is a run that started; its tool's
//!    checks stop until restart ([`PreflightGuard`]). Every path finds ids
//!    with the same scan, [`uuids_in_text`], over all of the answer
//!    (review R5).

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

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
    /// A paid tool through `call_tool`: paid calls go through `call_paid`,
    /// which names the run (review R15).
    #[error("a paid tool needs call_paid and a run tag")]
    NeedsRun,
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

/// Every job id in a JSON answer: the same text scan as for raw answers, so
/// an id inside a longer string ("queued <id>") counts too (review R5).
pub fn job_ids_in(answer: &Value) -> Vec<String> {
    uuids_in_text(&answer.to_string())
}

/// Every UUID-shaped run of 36 characters in raw text, lower-case and
/// deduplicated: the one scan for job ids, used on answers that parse and
/// on those that do not, so no job id is lost. A window is tried at every
/// byte, with no token boundaries, so `job-<id>`, `id-<id>` and
/// `<id>-0.png` count (review R13); finding too much fails closed.
pub fn uuids_in_text(text: &str) -> Vec<String> {
    let mut out: Vec<String> = uuid_spans(text)
        .map(|(start, end)| text[start..end].to_ascii_lowercase())
        .filter(|id| !is_nil_or_max(id))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The byte ranges of the UUID-shaped windows in `text`, left to right and
/// not overlapping. Every byte in a window is ASCII, so each range starts
/// and ends on a character boundary.
pub fn uuid_spans(text: &str) -> impl Iterator<Item = (usize, usize)> + '_ {
    let bytes = text.as_bytes();
    let mut i = 0;
    std::iter::from_fn(move || {
        while i + UUID_LEN <= bytes.len() {
            if uuid_at(bytes, i) {
                let span = (i, i + UUID_LEN);
                i += UUID_LEN;
                return Some(span);
            }
            i += 1;
        }
        None
    })
}

const UUID_LEN: usize = 36;

/// Whether `bytes[i..i + 36]` is `8-4-4-4-12` hexadecimal digits. The dashes
/// are checked first, so most positions cost four byte reads (review N7).
fn uuid_at(bytes: &[u8], i: usize) -> bool {
    const DASHES: [usize; 4] = [8, 13, 18, 23];
    let Some(window) = bytes.get(i..i + UUID_LEN) else {
        return false;
    };
    DASHES.iter().all(|&d| window[d] == b'-')
        && window
            .iter()
            .enumerate()
            .all(|(k, b)| DASHES.contains(&k) || b.is_ascii_hexdigit())
}

/// The nil and max UUIDs: values (they appear in the service's schema
/// patterns), never job ids (review R14).
fn is_nil_or_max(uuid: &str) -> bool {
    let digits = uuid.bytes().filter(|b| *b != b'-');
    let all = |want: u8| digits.clone().all(|b| b.eq_ignore_ascii_case(&want));
    all(b'0') || all(b'f')
}

/// The ids in `found` that are not in `sent`, the arguments of the request
/// that was answered: a refusal that echoes the request's own media ids
/// does not name a job (review R14).
pub fn new_ids(found: Vec<String>, sent: &Value) -> Vec<String> {
    let own = uuids_in_text(&sent.to_string());
    found.into_iter().filter(|id| !own.contains(id)).collect()
}

/// `8-4-4-4-12` hexadecimal digits.
pub fn is_uuid(text: &str) -> bool {
    text.len() == UUID_LEN && uuid_at(text.as_bytes(), 0)
}

/// Cost checks in flight, and tools whose check started or may have started
/// a run (§4.7 rule 4, review R6). Reserving a tool and testing it happen
/// under one lock, so two checks of the same tool never go out together (a
/// check can spend, §15 item 14). The app keeps one guard for its whole run
/// and hands it to every provider it creates, so reconnecting or switching
/// accounts does not lift a block.
#[derive(Debug, Default)]
pub struct PreflightGuard {
    tools: Mutex<BTreeMap<String, Check>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Check {
    InFlight,
    Blocked,
}

/// Why no cost check of a tool may go out now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckRefused {
    /// Another check of the tool is still waiting for its answer.
    InFlight,
    /// A check of the tool started, or may have started, a run; checks stay
    /// off until FreeCut restarts.
    Blocked,
}

impl PreflightGuard {
    /// Nothing reserved or blocked.
    pub fn new() -> PreflightGuard {
        PreflightGuard::default()
    }

    /// True once a check of `tool` started, or may have started, a run.
    pub fn is_blocked(&self, tool: &str) -> bool {
        self.lock().get(tool) == Some(&Check::Blocked)
    }

    /// Reserves `tool` for one cost check, to be settled when its outcome is
    /// known.
    ///
    /// # Errors
    /// [`CheckRefused`] while another check of `tool` is in flight, or once
    /// one blocked it.
    pub fn reserve(&self, tool: &str) -> Result<CheckReservation<'_>, CheckRefused> {
        let mut tools = self.lock();
        match tools.get(tool) {
            Some(Check::InFlight) => Err(CheckRefused::InFlight),
            Some(Check::Blocked) => Err(CheckRefused::Blocked),
            None => {
                tools.insert(tool.to_owned(), Check::InFlight);
                Ok(CheckReservation {
                    guard: self,
                    tool: tool.to_owned(),
                    settled: false,
                })
            }
        }
    }

    fn settle(&self, tool: &str, block: bool) {
        let mut tools = self.lock();
        if block {
            tools.insert(tool.to_owned(), Check::Blocked);
        } else {
            tools.remove(tool);
        }
    }

    fn lock(&self) -> MutexGuard<'_, BTreeMap<String, Check>> {
        self.tools.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// One cost check's hold on its tool. [`release`](Self::release) frees the
/// tool after an answer FreeCut could read, a check that was not sent, or a
/// refusal; [`block`](Self::block) keeps the tool's checks off until
/// restart. Dropping it unsettled (an early return, a panic) blocks too:
/// the guard fails closed.
#[must_use = "settle the reservation with release() or block()"]
#[derive(Debug)]
pub struct CheckReservation<'a> {
    guard: &'a PreflightGuard,
    tool: String,
    settled: bool,
}

impl CheckReservation<'_> {
    /// The check had a readable answer, was not sent, or was refused.
    pub fn release(mut self) {
        self.settled = true;
        self.guard.settle(&self.tool, false);
    }

    /// The check started, or may have started, a run.
    pub fn block(mut self) {
        self.settled = true;
        self.guard.settle(&self.tool, true);
    }
}

impl Drop for CheckReservation<'_> {
    fn drop(&mut self) {
        if !self.settled {
            self.guard.settle(&self.tool, true);
        }
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

    /// R6: one check per tool at a time; a refused or unsent check frees
    /// the tool, anything else (or forgetting to settle) blocks it.
    #[test]
    fn a_reservation_is_exclusive_and_fails_closed() {
        let guard = PreflightGuard::new();
        let first = guard.reserve("generate_image").unwrap();
        assert_eq!(
            guard.reserve("generate_image").unwrap_err(),
            CheckRefused::InFlight
        );
        let other = guard.reserve("generate_video").unwrap();
        first.release();
        guard.reserve("generate_image").unwrap().block();
        assert!(guard.is_blocked("generate_image"));
        assert_eq!(
            guard.reserve("generate_image").unwrap_err(),
            CheckRefused::Blocked
        );
        drop(other); // unsettled
        assert!(guard.is_blocked("generate_video"));
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
        // Inside a longer string too.
        assert_eq!(
            job_ids_in(
                &json!({"message": "queued 7c9e6679-7425-40de-944b-e07fc1f90ae7, then failed"})
            ),
            vec!["7c9e6679-7425-40de-944b-e07fc1f90ae7"]
        );
    }

    /// R13: a UUID glued to a hyphen, a hex letter or a suffix is found;
    /// there are no token boundaries, and over-detection fails closed.
    #[test]
    fn uuids_glued_to_hyphens_or_hex_letters_are_found() {
        let id = "7c9e6679-7425-40de-944b-e07fc1f90ae7";
        for text in [
            format!("job-{id}"),
            format!("request-{id}"),
            format!("id-{id}"),
            format!("{id}-0.png"),
            format!("https://cdn.example/a{}b.webp", id.to_uppercase()),
        ] {
            assert_eq!(uuids_in_text(&text), vec![id], "{text}");
        }
        assert!(uuids_in_text("7c9e6679-7425-40de-944b-e07fc1f90ae").is_empty());
        // R14: nil and max are values, not ids.
        assert!(
            uuids_in_text(
                "00000000-0000-0000-0000-000000000000 FFFFFFFF-ffff-ffff-ffff-ffffffffffff"
            )
            .is_empty()
        );
        assert_eq!(
            new_ids(
                vec![
                    id.to_owned(),
                    "0b4f6d2a-1111-4222-8333-944455556666".to_owned()
                ],
                &serde_json::json!({"medias": [{"value": id.to_uppercase()}]})
            ),
            vec!["0b4f6d2a-1111-4222-8333-944455556666"]
        );
        assert!(
            is_uuid(id)
                && !is_uuid(&format!("{id}0"))
                && !is_uuid("7c9e66797425-40de-944b-e07fc1f90ae7-")
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
        g.reserve("generate_video").unwrap().block();
        assert!(g.is_blocked("generate_video"));
        assert!(!g.is_blocked("generate_image"));
    }
}
