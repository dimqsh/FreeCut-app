//! The MCP client: JSON-RPC 2.0 over Streamable HTTP (architecture §4).
//!
//! Every message is a `POST` to the endpoint; the answer is one JSON body or
//! an SSE stream, checked for cancellation between chunks. The client keeps
//! the session id and negotiated revision for its lifetime.
//!
//! Retries follow §4.5 by [`CallClass`]:
//! - **free** calls are retried on connect errors, `429` and `5xx` (three
//!   retries, 1 s / 2 s / 4 s ± 25 %), and resent once after a `401` (with a
//!   renewed sign-in) or a session `404` (with a new session);
//! - **preflights** are retried only while nothing was sent, and never resent
//!   after the service answered;
//! - **paid** calls are never retried or resent: after a `401` or session
//!   `404` the sign-in or session is renewed and the call fails as
//!   *not accepted*.

pub mod jsonrpc;
pub mod schema;
pub mod sse;

use std::fmt;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use freecut_media::cancel::CancelToken;
use serde_json::{Value, json};

use crate::error::{CloudError, NotAcceptedReason};
use crate::guard::{self, GuardViolation, QuoteRequest};
use crate::oauth::discovery::{BearerChallenge, parse_bearer_challenge};
use crate::redact;
use crate::secret::Secret;
use crate::transport::{
    BodyError, CallLimits, HttpRequest, HttpResponse, HttpTransport, LateResponse, LateSink,
    Method, SendState, TransportError, TransportErrorKind, Waiter, backoff_delay, read_body_into,
};

use self::jsonrpc::{Incoming, RpcError};
use self::sse::SseParser;

/// The MCP revision FreeCut offers.
pub const LATEST_PROTOCOL_VERSION: &str = "2025-11-25";

/// Revisions FreeCut accepts when the server picks one (2025-06-18 added
/// `structuredContent` and the protocol-version header).
pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18"];

/// The tracing target for request and response bodies (redacted, `trace`
/// level only); the app routes it to its separate cloud trace log (§11).
pub const WIRE_TARGET: &str = "freecut_cloud::wire";

const MAX_ANSWER_BYTES: usize = 16 * 1024 * 1024;
const MAX_EXCERPT_BYTES: usize = 2 * 1024;
/// How much of an error body is scanned for job ids.
const MAX_ERROR_SCAN_BYTES: usize = 64 * 1024;
const MAX_RETRIES: u32 = 3;
const MAX_TOOL_PAGES: usize = 50;
/// Times one answer's event stream is resumed before the answer counts as
/// lost.
pub const MAX_RESUMES: u32 = 5;
const DEFAULT_RESUME_DELAY_MS: u64 = 1_000;
const MAX_RESUME_DELAY_MS: u64 = 30_000;

/// A tool FreeCut may call, with the rules the transport applies to it
/// (§4.5, §6.3). Only this crate's allow lists can create one, so a caller
/// can neither choose a call's class nor name an unlisted tool:
///
/// ```compile_fail
/// use freecut_cloud::mcp::AllowedTool;
/// use freecut_cloud::transport::CallLimits;
/// // Not possible outside freecut-cloud: the constructors are crate-private.
/// static FREE_GENERATION: AllowedTool =
///     AllowedTool::free("generate_image", CallLimits::READ, "a bypass");
/// ```
#[derive(Debug, PartialEq, Eq)]
pub struct AllowedTool {
    name: &'static str,
    class: CallClass,
    cost_preflight: bool,
    limits: CallLimits,
    used_for: &'static str,
}

impl AllowedTool {
    /// A free, read-only tool: retried and resent as §4.5 allows.
    pub(crate) const fn free(
        name: &'static str,
        limits: CallLimits,
        used_for: &'static str,
    ) -> AllowedTool {
        AllowedTool {
            name,
            class: CallClass::Free,
            cost_preflight: false,
            limits,
            used_for,
        }
    }

    /// A tool that spends money or must happen at most once: never retried
    /// or resent. `cost_preflight` says whether it offers `get_cost`.
    pub(crate) const fn paid(
        name: &'static str,
        limits: CallLimits,
        cost_preflight: bool,
        used_for: &'static str,
    ) -> AllowedTool {
        AllowedTool {
            name,
            class: CallClass::Paid,
            cost_preflight,
            limits,
            used_for,
        }
    }

    /// The tool name.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// [`CallClass::Free`] or [`CallClass::Paid`]; cost checks of a paid tool
    /// go through [`McpClient::check_cost`] instead.
    pub fn class(&self) -> CallClass {
        self.class
    }

    /// Whether the tool offers a `get_cost` preflight.
    pub fn has_cost_preflight(&self) -> bool {
        self.cost_preflight
    }

    /// The timeouts of an ordinary call.
    pub fn limits(&self) -> CallLimits {
        self.limits
    }

    /// The feature that uses it.
    pub fn used_for(&self) -> &'static str {
        self.used_for
    }
}

/// How a call may be retried or resent (§4.5). Taken from the tool's
/// [`AllowedTool`] entry, never chosen by a caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallClass {
    /// Read-only and free.
    Free,
    /// A cost preflight: free, but it shares its tool with the paid run.
    Preflight,
    /// Spends money (or must otherwise happen at most once).
    Paid,
}

impl CallClass {
    fn resends_after_refusal(self) -> bool {
        self == CallClass::Free
    }

    fn may_retry(self, err: &McpError) -> bool {
        match self {
            CallClass::Free => match err {
                McpError::Transport(e) => e.send == SendState::NotSent,
                McpError::Http { status, .. } => *status == 429 || (500..=599).contains(status),
                _ => false,
            },
            CallClass::Preflight => {
                matches!(err, McpError::Transport(e) if e.send == SendState::NotSent)
            }
            CallClass::Paid => false,
        }
    }

    fn token_purpose(self) -> TokenPurpose {
        match self {
            CallClass::Paid => TokenPurpose::BeforePaid,
            CallClass::Free | CallClass::Preflight => TokenPurpose::Ordinary,
        }
    }
}

/// How much validity an access token must have left before it is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenPurpose {
    /// Ordinary calls: renew when under 60 s remain.
    Ordinary,
    /// Before a paid submission: renew when under 5 minutes remain.
    BeforePaid,
}

impl TokenPurpose {
    /// The minimum remaining validity, in seconds.
    pub const fn margin_secs(self) -> i64 {
        match self {
            TokenPurpose::Ordinary => 60,
            TokenPurpose::BeforePaid => 300,
        }
    }
}

/// Supplies access tokens, read right before each request (CG-002).
pub trait TokenSource: Send + Sync + fmt::Debug {
    /// A token with at least `purpose`'s validity left, renewed if needed.
    ///
    /// # Errors
    /// [`CloudError`] when no usable token exists.
    fn bearer(&self, purpose: TokenPurpose, cancel: &CancelToken) -> Result<Secret, CloudError>;

    /// Renews the token after the service answered `401`.
    ///
    /// # Errors
    /// [`CloudError::SignInExpired`] when it cannot be renewed.
    fn renew(&self, cancel: &CancelToken) -> Result<(), CloudError>;
}

/// A fixed token that cannot be renewed (a pasted token, or tests).
#[derive(Debug, Clone)]
pub struct StaticToken(pub Secret);

impl TokenSource for StaticToken {
    fn bearer(&self, _purpose: TokenPurpose, _cancel: &CancelToken) -> Result<Secret, CloudError> {
        Ok(self.0.clone())
    }

    fn renew(&self, _cancel: &CancelToken) -> Result<(), CloudError> {
        Err(CloudError::SignInExpired)
    }
}

/// A failed MCP exchange, before classification ([`crate::error::classify`]).
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum McpError {
    /// No HTTP answer.
    #[error(transparent)]
    Transport(TransportError),
    /// No token could be supplied; nothing was sent.
    #[error("sign-in: {0}")]
    SignIn(CloudError),
    /// `401`.
    #[error("unauthorized")]
    Unauthorized {
        /// The `WWW-Authenticate` challenge, if readable.
        challenge: Option<BearerChallenge>,
    },
    /// `403`.
    #[error("forbidden")]
    Forbidden {
        /// The `WWW-Authenticate` challenge, if readable.
        challenge: Option<BearerChallenge>,
    },
    /// `404` on a request that carried a session id.
    #[error("the session expired")]
    SessionExpired,
    /// Refused, renewed, and deliberately not resent.
    #[error("not accepted: {0:?}")]
    NotAccepted(NotAcceptedReason),
    /// Any other non-success status.
    #[error("HTTP {status}: {excerpt}")]
    Http {
        /// The status.
        status: u16,
        /// The start of the body, as quoted text.
        excerpt: String,
        /// Job ids in the raw body, up to 64 KiB of it (review R12).
        job_ids: Vec<String>,
    },
    /// A JSON-RPC error answer.
    #[error("{error}")]
    Rpc {
        /// The error.
        error: RpcError,
        /// Job ids anywhere in the raw answer that carried it, earlier
        /// events included (review R12).
        job_ids: Vec<String>,
    },
    /// A tool result with `isError: true`.
    #[error("tool error: {message}")]
    Tool {
        /// The first text content.
        message: String,
        /// Structured content, or the text parsed as JSON.
        structured: Option<Value>,
        /// Every text content item, for the job-id scan (review R5).
        texts: Vec<String>,
        /// Job ids anywhere in the raw answer: other content items,
        /// `_meta`, earlier events (review R12).
        job_ids: Vec<String>,
    },
    /// An answer that cannot be read.
    #[error("unreadable answer: {detail}")]
    Protocol {
        /// Details for the log.
        detail: String,
        /// Job ids named in the part of the answer that was read (review
        /// R4): the request may have started them.
        job_ids: Vec<String>,
    },
    /// The server chose a revision FreeCut does not implement.
    #[error("unsupported protocol version {0}")]
    UnsupportedVersion(String),
    /// The caller cancelled.
    #[error("cancelled")]
    Cancelled {
        /// Whether the request may have been sent.
        send: SendState,
        /// Job ids in the part of the answer read before the cancel (review
        /// R37).
        job_ids: Vec<String>,
    },
    /// The preflight guard refused to send.
    #[error("preflight guard: {0}")]
    Guard(GuardViolation),
    /// The call was not sent because the handshake before it failed.
    #[error("not sent, the handshake failed: {0}")]
    BeforeSending(Box<McpError>),
}

impl McpError {
    /// Whether the failed request may have reached the server.
    pub fn send_state(&self) -> SendState {
        match self {
            McpError::Transport(e) => e.send,
            McpError::Cancelled { send, .. } => *send,
            McpError::SignIn(_) | McpError::Guard(_) | McpError::BeforeSending(_) => {
                SendState::NotSent
            }
            _ => SendState::MaybeSent,
        }
    }

    fn protocol(detail: impl Into<String>) -> McpError {
        McpError::Protocol {
            detail: detail.into(),
            job_ids: Vec::new(),
        }
    }

    /// Every job id the failed exchange's answer names (review R5, R12),
    /// found with one scan ([`guard::uuids_in_text`]) over the raw answer:
    /// every byte of it that arrived, earlier events and non-text items
    /// included, a JSON-RPC error's message and data, and an error status's
    /// body (4xx or 5xx, before it is cut for the message; on a paid call
    /// or cost check a 401, 403 or session 404 too, review R35). A failure that
    /// names a job may have started it, so it is never reported as rejected
    /// or not sent ([`crate::error::submit_disposition`] leaves out the
    /// request's own ids).
    pub fn job_ids(&self) -> Vec<String> {
        let (text, carried) = match self {
            McpError::Tool {
                message,
                structured,
                texts,
                job_ids,
            } => {
                let mut all = texts.join("\n");
                all.push('\n');
                all.push_str(message);
                if let Some(value) = structured {
                    all.push('\n');
                    all.push_str(&value.to_string());
                }
                (all, job_ids)
            }
            McpError::Rpc { error, job_ids } => match &error.data {
                Some(data) => (format!("{}\n{data}", error.message), job_ids),
                None => (error.message.clone(), job_ids),
            },
            McpError::Http {
                excerpt, job_ids, ..
            } => (excerpt.clone(), job_ids),
            McpError::Protocol { job_ids, .. } | McpError::Cancelled { job_ids, .. } => {
                (String::new(), job_ids)
            }
            _ => return Vec::new(),
        };
        let mut ids = guard::uuids_in_text(&text);
        ids.extend(carried.iter().cloned());
        ids.sort();
        ids.dedup();
        ids
    }

    /// Adds the job ids in `read`, the raw answer or the part of it that
    /// arrived, to an error that came with an answer (review R4, R12).
    /// Other errors are returned as they are.
    fn with_answer(self, read: &[u8]) -> McpError {
        self.with_job_ids(guard::uuids_in_text(&String::from_utf8_lossy(read)))
    }

    fn with_job_ids(mut self, ids: Vec<String>) -> McpError {
        if let McpError::Protocol { job_ids, .. }
        | McpError::Cancelled { job_ids, .. }
        | McpError::Tool { job_ids, .. }
        | McpError::Rpc { job_ids, .. }
        | McpError::Http { job_ids, .. } = &mut self
        {
            job_ids.extend(ids);
            job_ids.sort();
            job_ids.dedup();
        }
        self
    }
}

/// An answer to a paid call or a cost check that the caller stopped reading
/// before its end: after a cancel, or after a part it could not read (review
/// M3, R4, §4.4, §7.2). Its job ids must reach the ledger: the cloud worker
/// drains [`McpClient::take_late_results`] on every tick and before quitting
/// (R9). Part of the answer may have been read by the caller too, so ids are
/// recorded idempotently.
#[derive(Debug, Clone, PartialEq)]
pub struct LateToolResult {
    /// The run the call belongs to.
    pub run: Option<RunTag>,
    /// A paid call or a cost check.
    pub class: CallClass,
    /// The tool called.
    pub tool: String,
    /// The JSON-RPC request id.
    pub request_id: i64,
    /// The HTTP status.
    pub status: u16,
    /// Whether the whole body arrived.
    pub complete: bool,
    /// The tool result, when it could be read.
    pub value: Option<Value>,
    /// Every UUID-shaped string in the answer, read or not.
    pub job_ids: Vec<String>,
}

/// The run a paid call or cost check belongs to, chosen by the caller (the
/// worker's ledger id for the run). It never leaves the computer; a late
/// answer carries it so its job ids land on the right run (review R15).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RunTag(String);

impl RunTag {
    /// A tag for the run `id`.
    pub fn new(id: impl Into<String>) -> RunTag {
        RunTag(id.into())
    }

    /// The run id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Late answers to paid calls and cost checks (review M3, R4, R10). The app
/// keeps one for its whole run and hands it to every provider (and so every
/// client) it creates: a helper thread may deliver an answer up to minutes
/// after a cancel, when the provider that sent the request has long been
/// dropped or replaced, and its job ids must still reach the ledger.
#[derive(Debug, Default)]
pub struct LateQueue {
    results: Mutex<Vec<LateToolResult>>,
}

impl LateQueue {
    /// An empty queue.
    pub fn new() -> LateQueue {
        LateQueue::default()
    }

    /// The late answers so far, oldest first; taking them empties the queue.
    pub fn take(&self) -> Vec<LateToolResult> {
        std::mem::take(&mut *self.lock())
    }

    fn push(&self, result: LateToolResult) {
        self.lock().push(result);
    }

    fn lock(&self) -> MutexGuard<'_, Vec<LateToolResult>> {
        self.results.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// What the server said about itself in `initialize`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInfo {
    /// `serverInfo.name`.
    pub name: String,
    /// `serverInfo.version`.
    pub version: String,
    /// The negotiated revision.
    pub protocol_version: String,
    /// The server's `instructions`: text for chat agents. Kept for the debug
    /// view only; FreeCut never acts on it.
    pub instructions: Option<String>,
}

/// One tool from `tools/list`.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolInfo {
    /// The tool name.
    pub name: String,
    /// Its description (untrusted text, not shown as instructions).
    pub description: String,
    /// Its JSON Schema for arguments.
    pub input_schema: Value,
}

/// A successful tool result.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ToolOutput {
    /// `structuredContent`, when present.
    pub structured: Option<Value>,
    /// The `text` content items.
    pub texts: Vec<String>,
    /// Job ids anywhere in the raw answer: non-text items, `_meta`, events
    /// before the response (review R12).
    pub answer_job_ids: Vec<String>,
}

impl ToolOutput {
    /// Every job id the result names, in its structured content or any text
    /// item (review R5).
    pub fn job_ids(&self) -> Vec<String> {
        let mut all = self.texts.join("\n");
        if let Some(value) = &self.structured {
            all.push('\n');
            all.push_str(&value.to_string());
        }
        let mut ids = guard::uuids_in_text(&all);
        ids.extend(self.answer_job_ids.iter().cloned());
        ids.sort();
        ids.dedup();
        ids
    }

    /// The result as JSON (§4.3): `structuredContent`, else the first text
    /// parsed as JSON, else that text as a string.
    pub fn value(&self) -> Option<Value> {
        if let Some(v) = &self.structured {
            return Some(v.clone());
        }
        let text = self.texts.first()?;
        Some(serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.clone())))
    }
}

#[derive(Debug, Default)]
struct Session {
    id: Option<String>,
    protocol: Option<String>,
    server: Option<ServerInfo>,
    next_id: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Request,
    Notification,
}

#[derive(Debug)]
struct Outgoing<'a> {
    method: &'a str,
    params: Option<Value>,
    kind: Kind,
    /// Part of the handshake: a session `404` here means the handshake
    /// failed, and must never start another one (which would recurse).
    in_handshake: bool,
    /// The run a paid call or cost check belongs to.
    run: Option<&'a RunTag>,
}

/// What resuming an answer's event stream needs (review F4).
struct Resume {
    limits: CallLimits,
    late: Option<LateRoute>,
    session: Option<String>,
}

impl Resume {
    /// The caller has the response: what the transport may still hand the
    /// late sink is not late (review R4).
    fn answered(&self) {
        if let Some(route) = &self.late {
            route.answered.store(true, Ordering::Release);
        }
    }
}

/// Where the rest of an answer goes if the caller stops reading early.
struct LateRoute {
    sink: LateSink,
    /// Set once the caller has the response to the request.
    answered: Arc<AtomicBool>,
}

#[derive(Debug)]
struct Answer {
    result: Value,
    session: Option<String>,
    /// Job ids anywhere in the raw answer (review R12).
    job_ids: Vec<String>,
}

/// An MCP client for one server and one account.
pub struct McpClient {
    endpoint: String,
    transport: Arc<dyn HttpTransport>,
    tokens: Arc<dyn TokenSource>,
    waiter: Arc<dyn Waiter>,
    client_version: String,
    session: Mutex<Session>,
    late: Arc<LateQueue>,
}

impl fmt::Debug for McpClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpClient")
            .field("endpoint", &self.endpoint)
            .field("client_version", &self.client_version)
            .finish_non_exhaustive()
    }
}

impl McpClient {
    /// A client for `endpoint` with a late queue of its own (see
    /// [`McpClient::with_late_queue`]). Nothing is sent until the first call.
    pub fn new(
        endpoint: impl Into<String>,
        transport: Arc<dyn HttpTransport>,
        tokens: Arc<dyn TokenSource>,
        waiter: Arc<dyn Waiter>,
        client_version: impl Into<String>,
    ) -> McpClient {
        McpClient {
            endpoint: endpoint.into(),
            transport,
            tokens,
            waiter,
            client_version: client_version.into(),
            session: Mutex::new(Session::default()),
            late: Arc::new(LateQueue::new()),
        }
    }

    /// Sends late answers to `queue`, the app's (review R10).
    #[must_use]
    pub fn with_late_queue(mut self, queue: Arc<LateQueue>) -> McpClient {
        self.late = queue;
        self
    }

    /// The endpoint URL.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// The server's description from the last handshake.
    pub fn server(&self) -> Option<ServerInfo> {
        self.lock().server.clone()
    }

    /// Takes the late answers from this client's queue (the app's, when it
    /// passed one), oldest first.
    pub fn take_late_results(&self) -> Vec<LateToolResult> {
        self.late.take()
    }

    /// The current session id, if the server issued one.
    pub fn session_id(&self) -> Option<String> {
        self.lock().id.clone()
    }

    fn lock(&self) -> MutexGuard<'_, Session> {
        self.session.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs the handshake (§4.2): `initialize`, then
    /// `notifications/initialized`. Starts a new session.
    ///
    /// # Errors
    /// [`McpError`] when the handshake fails or the server's revision is not
    /// supported.
    pub fn initialize(&self, cancel: &CancelToken) -> Result<ServerInfo, McpError> {
        {
            let mut s = self.lock();
            s.id = None;
            s.protocol = None;
            s.server = None;
        }
        let params = json!({
            "protocolVersion": LATEST_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {"name": "FreeCut", "version": self.client_version},
        });
        let answer = self.exchange(
            &Outgoing {
                method: "initialize",
                params: Some(params),
                kind: Kind::Request,
                in_handshake: true,
                run: None,
            },
            CallClass::Free,
            CallLimits::READ,
            cancel,
        )?;
        let info = parse_initialize(&answer.result)?;
        if let Some(id) = &answer.session
            && !id.bytes().all(|b| (0x21..=0x7e).contains(&b))
        {
            return Err(McpError::protocol(
                "the session id has characters MCP does not allow",
            ));
        }
        {
            let mut s = self.lock();
            s.id = answer.session;
            s.protocol = Some(info.protocol_version.clone());
            s.server = Some(info.clone());
        }
        let notified = self.exchange(
            &Outgoing {
                method: "notifications/initialized",
                params: None,
                kind: Kind::Notification,
                in_handshake: true,
                run: None,
            },
            CallClass::Free,
            CallLimits::READ,
            cancel,
        );
        if let Err(e) = notified {
            // The handshake did not finish: the next call starts a new one.
            let mut s = self.lock();
            s.id = None;
            s.protocol = None;
            s.server = None;
            return Err(e);
        }
        tracing::debug!(server = %info.name, version = %info.version, protocol = %info.protocol_version, "mcp session started");
        Ok(info)
    }

    fn ensure_initialized(&self, cancel: &CancelToken) -> Result<(), McpError> {
        let ready = self.lock().protocol.is_some();
        if !ready {
            self.initialize(cancel)?;
        }
        Ok(())
    }

    /// Lists every tool, following `nextCursor` (§4.2).
    ///
    /// # Errors
    /// [`McpError`] when a page cannot be fetched or read.
    pub fn list_tools(&self, cancel: &CancelToken) -> Result<Vec<ToolInfo>, McpError> {
        self.ensure_initialized(cancel)?;
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_TOOL_PAGES {
            let params = cursor.as_ref().map(|c| json!({ "cursor": c }));
            let page = self
                .exchange(
                    &Outgoing {
                        method: "tools/list",
                        params,
                        kind: Kind::Request,
                        in_handshake: false,
                        run: None,
                    },
                    CallClass::Free,
                    CallLimits::READ,
                    cancel,
                )?
                .result;
            let items = page
                .get("tools")
                .and_then(Value::as_array)
                .ok_or_else(|| McpError::protocol("tools/list without a tools array"))?;
            for item in items {
                let name = item
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| McpError::protocol("a tool without a name"))?;
                tools.push(ToolInfo {
                    name: name.to_owned(),
                    description: item
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    input_schema: item
                        .get("inputSchema")
                        .cloned()
                        .unwrap_or_else(|| json!({})),
                });
            }
            match page.get("nextCursor").and_then(Value::as_str) {
                Some(next) if !next.is_empty() => cursor = Some(next.to_owned()),
                _ => return Ok(tools),
            }
        }
        Err(McpError::protocol("tools/list did not end"))
    }

    /// Calls a free allow-listed tool (§4.3), with the timeouts the allow
    /// list gives it.
    ///
    /// # Errors
    /// [`McpError::Guard`] for a tool that is not free (use
    /// [`McpClient::call_paid`]); otherwise [`McpError`], and a result with
    /// `isError: true` is [`McpError::Tool`].
    pub fn call_tool(
        &self,
        tool: &'static AllowedTool,
        arguments: Value,
        cancel: &CancelToken,
    ) -> Result<ToolOutput, McpError> {
        if tool.class != CallClass::Free {
            return Err(McpError::Guard(GuardViolation::NeedsRun));
        }
        self.call(tool.name, arguments, tool.class, tool.limits, None, cancel)
    }

    /// Calls a paid allow-listed tool for `run` (§4.3): never retried or
    /// resent, with the allow list's timeouts. A late answer to it carries
    /// `run` (review R15).
    ///
    /// # Errors
    /// As [`McpClient::call_tool`].
    pub fn call_paid(
        &self,
        tool: &'static AllowedTool,
        arguments: Value,
        run: &RunTag,
        cancel: &CancelToken,
    ) -> Result<ToolOutput, McpError> {
        self.call(
            tool.name,
            arguments,
            tool.class,
            tool.limits,
            Some(run),
            cancel,
        )
    }

    /// Sends a cost check (§4.7) for a paid tool that offers one. The
    /// arguments can only come from a [`QuoteRequest`], which always carries
    /// `get_cost: true`; the check is retried only while nothing was sent.
    ///
    /// # Errors
    /// [`McpError::Guard`] when the tool offers no cost check or the request
    /// is for another tool; otherwise as [`McpClient::call_tool`].
    pub fn check_cost(
        &self,
        tool: &'static AllowedTool,
        quote: &QuoteRequest,
        run: &RunTag,
        cancel: &CancelToken,
    ) -> Result<ToolOutput, McpError> {
        if !tool.cost_preflight {
            return Err(McpError::Guard(GuardViolation::NoCostCheck));
        }
        if quote.tool() != tool.name {
            return Err(McpError::Guard(GuardViolation::OtherTool));
        }
        self.call(
            tool.name,
            quote.arguments().clone(),
            CallClass::Preflight,
            CallLimits::PREFLIGHT,
            Some(run),
            cancel,
        )
    }

    fn call(
        &self,
        name: &str,
        arguments: Value,
        class: CallClass,
        limits: CallLimits,
        run: Option<&RunTag>,
        cancel: &CancelToken,
    ) -> Result<ToolOutput, McpError> {
        // A failed handshake means this call never left, whatever the
        // handshake's own requests did.
        self.ensure_initialized(cancel)
            .map_err(|e| McpError::BeforeSending(Box::new(e)))?;
        tracing::debug!(tool = name, ?class, "tools/call");
        let answer = self.exchange(
            &Outgoing {
                method: "tools/call",
                params: Some(json!({ "name": name, "arguments": arguments })),
                kind: Kind::Request,
                in_handshake: false,
                run,
            },
            class,
            limits,
            cancel,
        )?;
        let raw = answer.result.to_string();
        match parse_tool_result(answer.result) {
            Ok(mut output) => {
                output.answer_job_ids = answer.job_ids;
                Ok(output)
            }
            Err(e) => Err(e.with_answer(raw.as_bytes()).with_job_ids(answer.job_ids)),
        }
    }

    /// Ends the session with `DELETE` (on disconnect and quit; best effort,
    /// 2 s). The next call starts a new session.
    pub fn close(&self) {
        let (session, protocol) = {
            let mut s = self.lock();
            s.server = None;
            (s.id.take(), s.protocol.take())
        };
        let Some(id) = session else {
            return;
        };
        let cancel = CancelToken::new();
        let Ok(token) = self.tokens.bearer(TokenPurpose::Ordinary, &cancel) else {
            return;
        };
        let mut request = HttpRequest::get(self.endpoint.clone(), CallLimits::CLOSE)
            .header("mcp-session-id", id)
            .with_bearer(token);
        // Every request after initialization names the revision (review F10).
        if let Some(version) = protocol {
            request = request.header("mcp-protocol-version", version);
        }
        request.method = Method::Delete;
        if let Err(e) = self.transport.send(request, &cancel) {
            tracing::debug!(error = %e, "could not close the mcp session");
        }
    }

    /// One logical exchange with §4.5's retry and resend rules.
    fn exchange(
        &self,
        out: &Outgoing<'_>,
        class: CallClass,
        limits: CallLimits,
        cancel: &CancelToken,
    ) -> Result<Answer, McpError> {
        let mut retries = 0;
        let mut renewed = false;
        let mut reinitialized = false;
        loop {
            if cancel.is_cancelled() {
                return Err(McpError::Cancelled {
                    send: SendState::NotSent,
                    job_ids: Vec::new(),
                });
            }
            let err = match self.send_once(out, class, limits, cancel) {
                Ok(answer) => return Ok(answer),
                Err(err) => err,
            };
            match err {
                McpError::Unauthorized { challenge } if !renewed => {
                    renewed = true;
                    if let Err(e) = self.tokens.renew(cancel) {
                        return Err(if class.resends_after_refusal() {
                            McpError::SignIn(e)
                        } else {
                            McpError::Unauthorized { challenge }
                        });
                    }
                    if !class.resends_after_refusal() {
                        return Err(McpError::NotAccepted(NotAcceptedReason::Unauthorized));
                    }
                }
                McpError::SessionExpired if !reinitialized && !out.in_handshake => {
                    reinitialized = true;
                    let renewed = self.initialize(cancel);
                    if !class.resends_after_refusal() {
                        // The refusal is the answer to this call; a failed
                        // renewal only affects the next one.
                        if let Err(e) = renewed {
                            tracing::debug!(error = %e, "could not open a new session");
                        }
                        return Err(McpError::NotAccepted(NotAcceptedReason::SessionExpired));
                    }
                    renewed?;
                }
                err if retries < MAX_RETRIES && class.may_retry(&err) => {
                    let delay = backoff_delay(retries, self.waiter.jitter());
                    retries += 1;
                    tracing::debug!(method = out.method, retry = retries, ?delay, error = %err, "retrying");
                    if !self.waiter.wait(delay, cancel) {
                        return Err(McpError::Cancelled {
                            send: SendState::NotSent,
                            job_ids: Vec::new(),
                        });
                    }
                }
                err => return Err(err),
            }
        }
    }

    /// One HTTP request, no retries.
    fn send_once(
        &self,
        out: &Outgoing<'_>,
        class: CallClass,
        limits: CallLimits,
        cancel: &CancelToken,
    ) -> Result<Answer, McpError> {
        let is_initialize = out.method == "initialize";
        let (id, session, protocol) = {
            let mut s = self.lock();
            s.next_id += 1;
            (s.next_id, s.id.clone(), s.protocol.clone())
        };
        let message = match out.kind {
            Kind::Request => jsonrpc::request(id, out.method, out.params.clone()),
            Kind::Notification => jsonrpc::notification(out.method, out.params.clone()),
        };
        let body = serde_json::to_vec(&message).map_err(|e| McpError::protocol(e.to_string()))?;
        if class == CallClass::Preflight {
            guard::verify_outgoing_preflight(&body).map_err(McpError::Guard)?;
        }
        let token = self
            .tokens
            .bearer(class.token_purpose(), cancel)
            .map_err(McpError::SignIn)?;
        let mut request = HttpRequest::post_json(self.endpoint.clone(), body, limits)
            .header("accept", "application/json, text/event-stream")
            .with_bearer(token);
        let late = (out.kind == Kind::Request && class != CallClass::Free)
            .then(|| self.late_route(out, id, class));
        if let Some(route) = &late {
            request = request.with_late(Arc::clone(&route.sink));
        }
        if !is_initialize {
            if let Some(version) = protocol {
                request = request.header("mcp-protocol-version", version);
            }
            if let Some(sid) = &session {
                request = request.header("mcp-session-id", sid.clone());
            }
        }
        tracing::trace!(target: WIRE_TARGET, method = out.method, body = %redact::body(&request.body), "mcp request");
        let started = Instant::now();
        // A cancel after the request may have left: tell the server (§4.4).
        let tell_cancelled = |err: &McpError| {
            let after_sending = matches!(
                err,
                McpError::Cancelled {
                    send: SendState::MaybeSent,
                    ..
                }
            ) || matches!(err, McpError::Transport(e) if e.kind == TransportErrorKind::Cancelled && e.send == SendState::MaybeSent);
            if after_sending && out.kind == Kind::Request && !is_initialize {
                self.notify_cancelled(id);
            }
        };
        let response = match self.transport.send(request, cancel) {
            Ok(response) => response,
            Err(e) => {
                let err = McpError::Transport(e);
                tell_cancelled(&err);
                return Err(err);
            }
        };
        tracing::debug!(
            method = out.method,
            status = response.status,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "mcp answer"
        );
        let session_header = response.header("mcp-session-id").map(str::to_owned);
        let www_authenticate = response.header("www-authenticate").map(str::to_owned);
        let challenge = || www_authenticate.as_deref().and_then(parse_bearer_challenge);
        match response.status {
            200..=299 => match out.kind {
                Kind::Notification => Ok(Answer {
                    result: Value::Null,
                    session: session_header,
                    job_ids: Vec::new(),
                }),
                Kind::Request => {
                    let resume = Resume {
                        limits,
                        late,
                        session: session_header.clone(),
                    };
                    let (result, job_ids) = self
                        .read_answer(response, id, resume, cancel)
                        .inspect_err(|e| tell_cancelled(e))?;
                    Ok(Answer {
                        result,
                        session: session_header,
                        job_ids,
                    })
                }
            },
            // On a paid call or cost check these bodies are read too: one
            // that names a job (not the request's own ids) makes the
            // answer an error that carries it, so the run is Outcome
            // unknown or a started run, never Rejected, and nothing is
            // renewed or resent (review R35, the owner's R12 rule).
            401 | 403 | 404
                if class != CallClass::Free && (response.status != 404 || session.is_some()) =>
            {
                let status = response.status;
                let (excerpt, found) = excerpt(response, cancel);
                let sent = out
                    .params
                    .as_ref()
                    .and_then(|p| p.get("arguments"))
                    .cloned()
                    .unwrap_or(Value::Null);
                let job_ids = guard::new_ids(found, &sent);
                Err(match status {
                    _ if !job_ids.is_empty() => McpError::Http {
                        status,
                        excerpt,
                        job_ids,
                    },
                    401 => McpError::Unauthorized {
                        challenge: challenge(),
                    },
                    403 => McpError::Forbidden {
                        challenge: challenge(),
                    },
                    _ => McpError::SessionExpired,
                })
            }
            401 => Err(McpError::Unauthorized {
                challenge: challenge(),
            }),
            403 => Err(McpError::Forbidden {
                challenge: challenge(),
            }),
            404 if session.is_some() => Err(McpError::SessionExpired),
            status => {
                let (excerpt, job_ids) = excerpt(response, cancel);
                Err(McpError::Http {
                    status,
                    excerpt,
                    job_ids,
                })
            }
        }
    }

    fn read_answer(
        &self,
        mut response: HttpResponse,
        id: i64,
        resume: Resume,
        cancel: &CancelToken,
    ) -> Result<(Value, Vec<String>), McpError> {
        // Every byte of the answer is scanned for job ids, whatever the
        // outcome (review R12).
        let ids = |read: &[u8]| guard::uuids_in_text(&String::from_utf8_lossy(read));
        match response.media_type().as_deref() {
            Some("application/json") => {
                let mut bytes = Vec::new();
                if let Err(e) =
                    read_body_into(&mut response.body, cancel, MAX_ANSWER_BYTES, &mut bytes)
                {
                    return Err(body_error(e).with_answer(&bytes));
                }
                tracing::trace!(target: WIRE_TARGET, body = %redact::body(&bytes), "mcp answer body");
                // Read to its end, so nothing in it can be reported late.
                let value = self
                    .response_in(&bytes, id, cancel)
                    .and_then(|outcome| outcome)
                    .map_err(|e| e.with_answer(&bytes))?;
                Ok((value, ids(&bytes)))
            }
            Some("text/event-stream") => {
                let mut read = Vec::new();
                let value = self
                    .read_stream(response.body, id, &resume, cancel, &mut read)
                    .map_err(|e| e.with_answer(&read))?;
                Ok((value, ids(&read)))
            }
            other => Err(McpError::protocol(format!(
                "unexpected content type {}",
                other.unwrap_or("(none)")
            ))),
        }
    }

    /// The response to `id` in a JSON body.
    #[allow(clippy::type_complexity)] // as for `handle_incoming`
    fn response_in(
        &self,
        bytes: &[u8],
        id: i64,
        cancel: &CancelToken,
    ) -> Result<Result<Value, McpError>, McpError> {
        let value: Value = serde_json::from_slice(bytes)
            .map_err(|e| McpError::protocol(format!("invalid JSON: {e}")))?;
        for message in jsonrpc::messages(value).map_err(McpError::protocol)? {
            if let Some(outcome) = self.handle_incoming(&message, id, cancel)? {
                return Ok(outcome);
            }
        }
        Err(McpError::protocol(
            "the answer has no response to the request",
        ))
    }

    /// Reads an event stream until the response to `id`. A stream that ends
    /// or breaks after an event id but before the response is resumed with
    /// `GET` and
    /// `Last-Event-ID`, after the server's `retry` delay, at most
    /// [`MAX_RESUMES`] times (MCP 2025-11-25, review F4). A `GET` never
    /// resends the request; if resuming fails, the answer is lost, which
    /// for a paid call is an unknown outcome. The bytes read, across
    /// resumes, are kept in `read` (up to the answer limit) so an unreadable
    /// answer still names its job ids (review R4).
    fn read_stream(
        &self,
        mut body: Box<dyn Read + Send>,
        id: i64,
        resume: &Resume,
        cancel: &CancelToken,
        read: &mut Vec<u8>,
    ) -> Result<Value, McpError> {
        let mut parser = SseParser::new(MAX_ANSWER_BYTES);
        let mut chunk = [0u8; 8 * 1024];
        let mut resumes = 0;
        loop {
            if cancel.is_cancelled() {
                return Err(McpError::Cancelled {
                    send: SendState::MaybeSent,
                    job_ids: Vec::new(),
                });
            }
            let n = match body.read(&mut chunk) {
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) if cancel.is_cancelled() => {
                    return Err(McpError::Cancelled {
                        send: SendState::MaybeSent,
                        job_ids: Vec::new(),
                    });
                }
                // A stream that broke (a cut chunked body, a reset) with an
                // event id to resume from resumes like one that ended: the
                // GET never resends the request (MCP 2025-11-25; review R16).
                Err(e) if parser.last_event_id().is_some() => {
                    tracing::debug!(error = %e, "the event stream broke; resuming");
                    0
                }
                Err(e) => return Err(McpError::protocol(format!("the event stream broke: {e}"))),
            };
            if n == 0 {
                let Some(last) = parser.last_event_id().map(str::to_owned) else {
                    return Err(McpError::protocol(
                        "the event stream ended before the response",
                    ));
                };
                if resumes == MAX_RESUMES {
                    return Err(McpError::protocol(
                        "the event stream ended again and again before the response",
                    ));
                }
                resumes += 1;
                let delay = Duration::from_millis(
                    parser
                        .retry_ms()
                        .unwrap_or(DEFAULT_RESUME_DELAY_MS)
                        .min(MAX_RESUME_DELAY_MS),
                );
                if !self.waiter.wait(delay, cancel) {
                    return Err(McpError::Cancelled {
                        send: SendState::MaybeSent,
                        job_ids: Vec::new(),
                    });
                }
                body = self.resume(&last, resume, cancel)?;
                parser.next_stream();
                continue;
            }
            if read.len() < MAX_ANSWER_BYTES {
                read.extend_from_slice(&chunk[..n.min(MAX_ANSWER_BYTES - read.len())]);
            }
            let events = parser
                .feed(&chunk[..n])
                .map_err(|e| McpError::protocol(e.to_string()))?;
            for event in events {
                // JSON-RPC messages come as `message` events; other types
                // are not ours to read (review F10).
                if event.event != "message" || event.data.trim().is_empty() {
                    continue; // another event type, or a priming/keep-alive event
                }
                tracing::trace!(target: WIRE_TARGET, data = %redact::body(event.data.as_bytes()), "mcp event");
                let value: Value = serde_json::from_str(&event.data)
                    .map_err(|e| McpError::protocol(format!("invalid JSON in an event: {e}")))?;
                for message in jsonrpc::messages(value).map_err(McpError::protocol)? {
                    if let Some(outcome) = self.handle_incoming(&message, id, cancel)? {
                        // Taken before the body is dropped, so the transport's
                        // copy of the rest is not reported as late.
                        resume.answered();
                        return outcome;
                    }
                }
            }
        }
    }

    /// Sends `notifications/cancelled` for request `id` after a local cancel
    /// (§4.4, review F8): best effort, on a fresh token, with the short
    /// limits of a session close. The server may already have acted; this
    /// changes nothing about how the call is classified.
    fn notify_cancelled(&self, id: i64) {
        let fresh = CancelToken::new();
        let Ok(token) = self.tokens.bearer(TokenPurpose::Ordinary, &fresh) else {
            return;
        };
        let message = jsonrpc::notification(
            "notifications/cancelled",
            Some(json!({ "requestId": id, "reason": "cancelled in FreeCut" })),
        );
        let (session, protocol) = {
            let s = self.lock();
            (s.id.clone(), s.protocol.clone())
        };
        let mut request = HttpRequest::post_json(
            self.endpoint.clone(),
            message.to_string().into_bytes(),
            CallLimits::CLOSE,
        )
        .header("accept", "application/json, text/event-stream")
        .with_bearer(token);
        if let Some(version) = protocol {
            request = request.header("mcp-protocol-version", version);
        }
        if let Some(sid) = session {
            request = request.header("mcp-session-id", sid);
        }
        if let Err(e) = self.transport.send(request, &fresh) {
            tracing::debug!(error = %e, "could not tell the server about a cancel");
        }
    }

    /// Opens the resumed stream after `last_event_id` (review F4).
    fn resume(
        &self,
        last_event_id: &str,
        resume: &Resume,
        cancel: &CancelToken,
    ) -> Result<Box<dyn Read + Send>, McpError> {
        let lost = |why: String| {
            McpError::protocol(format!("the event stream could not be resumed: {why}"))
        };
        let token = self
            .tokens
            .bearer(TokenPurpose::Ordinary, cancel)
            .map_err(|e| lost(e.to_string()))?;
        let (session, protocol) = {
            let s = self.lock();
            // The answer's own session id first: during `initialize` it is
            // not stored yet.
            (
                resume.session.clone().or_else(|| s.id.clone()),
                s.protocol.clone(),
            )
        };
        let mut request = HttpRequest::get(self.endpoint.clone(), resume.limits)
            .header("accept", "text/event-stream")
            .header("last-event-id", last_event_id)
            .with_bearer(token);
        if let Some(version) = protocol {
            request = request.header("mcp-protocol-version", version);
        }
        if let Some(sid) = session {
            request = request.header("mcp-session-id", sid);
        }
        if let Some(route) = &resume.late {
            request = request.with_late(Arc::clone(&route.sink));
        }
        tracing::debug!(%last_event_id, "resuming an event stream");
        match self.transport.send(request, cancel) {
            Ok(response)
                if response.status == 200
                    && response.media_type().as_deref() == Some("text/event-stream") =>
            {
                Ok(response.body)
            }
            Ok(response) => Err(lost(format!("HTTP {}", response.status))),
            Err(e) if e.kind == TransportErrorKind::Cancelled => Err(McpError::Cancelled {
                send: SendState::MaybeSent,
                job_ids: Vec::new(),
            }),
            Err(e) => Err(lost(e.to_string())),
        }
    }

    /// Handles one message; `Some` when it answers request `id`.
    #[allow(clippy::type_complexity)] // Option<Result<..>>: "not ours" vs "ours, maybe an error"
    fn handle_incoming(
        &self,
        message: &Value,
        id: i64,
        cancel: &CancelToken,
    ) -> Result<Option<Result<Value, McpError>>, McpError> {
        match jsonrpc::parse(message).map_err(McpError::protocol)? {
            Incoming::Response { id: rid, outcome } => {
                if rid == json!(id) {
                    Ok(Some(outcome.map_err(|error| McpError::Rpc {
                        error,
                        job_ids: Vec::new(),
                    })))
                } else {
                    tracing::debug!("ignored a response to another request");
                    Ok(None)
                }
            }
            Incoming::Request { id: rid, method } => {
                self.answer_server_request(&rid, &method, cancel);
                Ok(None)
            }
            Incoming::Notification { method } => {
                if method == "notifications/progress" || method == "notifications/message" {
                    tracing::debug!(%method, "server notification");
                }
                Ok(None)
            }
        }
    }

    /// FreeCut declares no client capabilities: `ping` gets the empty result
    /// MCP requires, anything else "method not found" (§4.1).
    fn answer_server_request(&self, id: &Value, method: &str, cancel: &CancelToken) {
        let reply = if method == "ping" {
            jsonrpc::result(id, json!({}))
        } else {
            jsonrpc::error(
                id,
                jsonrpc::METHOD_NOT_FOUND,
                "FreeCut does not serve requests",
            )
        };
        let Ok(token) = self.tokens.bearer(TokenPurpose::Ordinary, cancel) else {
            return;
        };
        let (session, protocol) = {
            let s = self.lock();
            (s.id.clone(), s.protocol.clone())
        };
        let mut request = HttpRequest::post_json(
            self.endpoint.clone(),
            reply.to_string().into_bytes(),
            CallLimits::READ,
        )
        .header("accept", "application/json, text/event-stream")
        .with_bearer(token);
        if let Some(version) = protocol {
            request = request.header("mcp-protocol-version", version);
        }
        if let Some(sid) = session {
            request = request.header("mcp-session-id", sid);
        }
        if let Err(e) = self.transport.send(request, cancel) {
            tracing::debug!(error = %e, %method, "could not answer a server request");
        }
    }
}

impl McpClient {
    /// Where a late answer to request `id` goes: parsed as far as possible
    /// and queued with every job id it names. Nothing is queued once the
    /// caller has the response, nor for a refusal status that names no job.
    fn late_route(&self, out: &Outgoing<'_>, id: i64, class: CallClass) -> LateRoute {
        let tool = out
            .params
            .as_ref()
            .and_then(|p| p.get("name"))
            .and_then(Value::as_str)
            .unwrap_or(out.method)
            .to_owned();
        // The request's own UUIDs (media it refers to) are not jobs (R14).
        let sent = out
            .params
            .as_ref()
            .and_then(|p| p.get("arguments"))
            .cloned()
            .unwrap_or(Value::Null);
        let run = out.run.cloned();
        let queue = Arc::clone(&self.late);
        let answered = Arc::new(AtomicBool::new(false));
        let taken = Arc::clone(&answered);
        let sink: LateSink = Arc::new(move |response: LateResponse| {
            if taken.load(Ordering::Acquire) {
                return;
            }
            let mut result = late_result(&tool, id, response);
            result.run.clone_from(&run);
            result.class = class;
            result.job_ids = guard::new_ids(result.job_ids, &sent);
            if result.job_ids.is_empty() && !(200..300).contains(&result.status) {
                tracing::debug!(
                    status = result.status,
                    "a refusal arrived after the caller stopped"
                );
                return;
            }
            tracing::warn!(
                tool = %result.tool,
                jobs = result.job_ids.len(),
                "an answer arrived after the caller stopped reading"
            );
            queue.push(result);
        });
        LateRoute { sink, answered }
    }
}

/// Reads a late answer: a JSON or SSE body with the response to `id`.
fn late_result(tool: &str, id: i64, response: LateResponse) -> LateToolResult {
    let text = String::from_utf8_lossy(&response.body).into_owned();
    let is_stream = response.headers.iter().any(|(k, v)| {
        k.eq_ignore_ascii_case("content-type")
            && v.to_ascii_lowercase().starts_with("text/event-stream")
    });
    let mut messages = Vec::new();
    if is_stream {
        let mut parser = SseParser::new(MAX_ANSWER_BYTES);
        let mut events = parser.feed(response.body.as_slice()).unwrap_or_default();
        // A cut-off stream may lack the final blank line.
        events.extend(parser.feed(b"\n\n").unwrap_or_default());
        for event in events {
            if let Ok(value) = serde_json::from_str::<Value>(&event.data) {
                messages.extend(jsonrpc::messages(value).unwrap_or_default());
            }
        }
    } else if let Ok(value) = serde_json::from_str::<Value>(&text) {
        messages.extend(jsonrpc::messages(value).unwrap_or_default());
    }
    let value = messages.iter().find_map(|m| match jsonrpc::parse(m) {
        Ok(Incoming::Response { id: rid, outcome }) if rid == json!(id) => match outcome {
            Ok(result) => match parse_tool_result(result) {
                Ok(output) => output.value(),
                Err(McpError::Tool { structured, .. }) => structured,
                Err(_) => None,
            },
            Err(e) => e.data,
        },
        _ => None,
    });
    LateToolResult {
        run: None,
        class: CallClass::Paid,
        tool: tool.to_owned(),
        request_id: id,
        status: response.status,
        complete: response.complete,
        value,
        job_ids: guard::uuids_in_text(&text),
    }
}

fn body_error(e: BodyError) -> McpError {
    match e {
        BodyError::Cancelled => McpError::Cancelled {
            send: SendState::MaybeSent,
            job_ids: Vec::new(),
        },
        other => McpError::protocol(other.to_string()),
    }
}

/// The start of an error body as quoted text, and the job ids in the raw
/// body, up to [`MAX_ERROR_SCAN_BYTES`] of it, before it is cut for the
/// message (review R12).
fn excerpt(mut response: HttpResponse, cancel: &CancelToken) -> (String, Vec<String>) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8 * 1024];
    while buf.len() < MAX_ERROR_SCAN_BYTES && !cancel.is_cancelled() {
        match response.body.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n.min(MAX_ERROR_SCAN_BYTES - buf.len())]),
        }
    }
    let ids = guard::uuids_in_text(&String::from_utf8_lossy(&buf));
    let start = &buf[..buf.len().min(MAX_EXCERPT_BYTES)];
    (crate::error::quoted(&String::from_utf8_lossy(start)), ids)
}

fn parse_initialize(result: &Value) -> Result<ServerInfo, McpError> {
    let version = result
        .get("protocolVersion")
        .and_then(Value::as_str)
        .ok_or_else(|| McpError::protocol("initialize without protocolVersion"))?;
    if !SUPPORTED_PROTOCOL_VERSIONS.contains(&version) {
        return Err(McpError::UnsupportedVersion(version.to_owned()));
    }
    let info = result.get("serverInfo");
    let text = |key: &str| {
        info.and_then(|i| i.get(key))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    Ok(ServerInfo {
        name: text("name"),
        version: text("version"),
        protocol_version: version.to_owned(),
        instructions: result
            .get("instructions")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

fn parse_tool_result(result: Value) -> Result<ToolOutput, McpError> {
    let obj = result
        .as_object()
        .ok_or_else(|| McpError::protocol("a tool result that is not an object"))?;
    let texts = obj
        .get("content")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|c| c.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|c| c.get("text").and_then(Value::as_str))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let output = ToolOutput {
        structured: obj.get("structuredContent").cloned(),
        texts,
        answer_job_ids: Vec::new(),
    };
    if obj.get("isError").and_then(Value::as_bool).unwrap_or(false) {
        return Err(McpError::Tool {
            message: output.texts.first().cloned().unwrap_or_default(),
            structured: output.value().filter(Value::is_object),
            texts: output.texts,
            job_ids: Vec::new(),
        });
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_values_prefer_structured_content() {
        let out = parse_tool_result(json!({
            "content": [{"type": "text", "text": "{\"cost\":{\"credits\":1}}"}],
            "structuredContent": {"cost": {"credits": 2}}
        }))
        .unwrap();
        assert_eq!(out.value(), Some(json!({"cost": {"credits": 2}})));
        let out =
            parse_tool_result(json!({"content": [{"type": "text", "text": "{\"a\":1}"}]})).unwrap();
        assert_eq!(out.value(), Some(json!({"a": 1})));
        let out =
            parse_tool_result(json!({"content": [{"type": "text", "text": "plain"}]})).unwrap();
        assert_eq!(out.value(), Some(json!("plain")));
        assert_eq!(
            parse_tool_result(json!({"content": []})).unwrap().value(),
            None
        );
    }

    /// R5: job ids count in every content item, not only the one read as
    /// the result.
    #[test]
    fn job_ids_are_found_in_every_content_item() {
        let job = "7c9e6679-7425-40de-944b-e07fc1f90ae7";
        let output = ToolOutput {
            structured: None,
            texts: vec![r#"{"cost":{"credits":2}}"#.into(), format!("queued {job}")],
            answer_job_ids: vec![],
        };
        assert_eq!(output.value(), Some(json!({"cost": {"credits": 2}})));
        assert_eq!(output.job_ids(), vec![job]);
        let rpc = McpError::Rpc {
            error: RpcError {
                code: -32602,
                message: "Invalid params".into(),
                data: Some(json!({"jobs": [{"id": job.to_uppercase()}]})),
            },
            job_ids: vec![],
        };
        assert_eq!(rpc.job_ids(), vec![job]);
        let gateway = McpError::Http {
            status: 502,
            excerpt: format!("upstream failed for {job}"),
            job_ids: vec![],
        };
        assert_eq!(gateway.job_ids(), vec![job]);
    }

    #[test]
    fn tool_errors_carry_the_message() {
        let err = parse_tool_result(json!({
            "content": [{"type": "text", "text": "Invalid request: unknown model \"x\""}],
            "isError": true
        }))
        .unwrap_err();
        assert!(
            matches!(err, McpError::Tool { ref message, structured: None, .. } if message.starts_with("Invalid request"))
        );
    }

    #[test]
    fn initialize_accepts_only_supported_revisions() {
        let ok = parse_initialize(
            &json!({"protocolVersion": "2025-06-18", "serverInfo": {"name": "s", "version": "1"}}),
        )
        .unwrap();
        assert_eq!(ok.protocol_version, "2025-06-18");
        assert_eq!(
            parse_initialize(&json!({"protocolVersion": "2024-11-05"})),
            Err(McpError::UnsupportedVersion("2024-11-05".into()))
        );
    }

    /// Answers initialize with JSON and everything else with a fixed SSE body.
    #[derive(Debug)]
    struct FixedStream(&'static str);

    impl HttpTransport for FixedStream {
        fn send(
            &self,
            request: HttpRequest,
            _: &CancelToken,
        ) -> Result<HttpResponse, TransportError> {
            let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
            Ok(match body["method"].as_str() {
                Some("initialize") => HttpResponse::new(
                    200,
                    vec![("content-type".into(), "application/json".into())],
                    json!({"jsonrpc": "2.0", "id": body["id"], "result": {"protocolVersion": "2025-11-25"}})
                        .to_string()
                        .into_bytes(),
                ),
                Some("notifications/initialized") => HttpResponse::new(202, Vec::new(), Vec::new()),
                _ => HttpResponse::new(
                    200,
                    vec![("content-type".into(), "text/event-stream".into())],
                    self.0.as_bytes().to_vec(),
                ),
            })
        }
    }

    /// F10: an event of another type carrying a response-shaped message is
    /// not read as the answer.
    #[test]
    fn events_of_other_types_are_skipped() {
        let stream = "event: endpoint\ndata: {\"jsonrpc\":\"2.0\",\"id\":3,\"result\":{\"content\":[{\"type\":\"text\",\"text\":\"decoy\"}]}}\n\n\
                      data: {\"jsonrpc\":\"2.0\",\"id\":3,\"result\":{\"content\":[{\"type\":\"text\",\"text\":\"real\"}]}}\n\n";
        let c = McpClient::new(
            "https://mcp.example.test/mcp",
            Arc::new(FixedStream(stream)),
            Arc::new(StaticToken(Secret::new("t"))),
            Arc::new(crate::transport::SystemWaiter),
            "0.1.0",
        );
        let out = c
            .call_tool(
                &crate::provider::higgsfield::allow::MODELS_EXPLORE,
                json!({}),
                &CancelToken::new(),
            )
            .unwrap();
        assert_eq!(out.texts, vec!["real"]);
    }

    #[test]
    fn only_free_calls_retry_after_an_answer() {
        let http = McpError::Http {
            status: 503,
            excerpt: String::new(),
            job_ids: vec![],
        };
        let not_sent = McpError::Transport(TransportError::not_sent(
            crate::transport::TransportErrorKind::Connect,
            "",
        ));
        let maybe = McpError::Transport(TransportError::maybe_sent(
            crate::transport::TransportErrorKind::Timeout,
            "",
        ));
        assert!(CallClass::Free.may_retry(&http));
        assert!(CallClass::Free.may_retry(&not_sent));
        assert!(!CallClass::Free.may_retry(&maybe));
        assert!(!CallClass::Preflight.may_retry(&http));
        assert!(CallClass::Preflight.may_retry(&not_sent));
        assert!(!CallClass::Preflight.may_retry(&maybe));
        assert!(!CallClass::Paid.may_retry(&not_sent));
        assert!(!CallClass::Paid.may_retry(&http));
    }
}
