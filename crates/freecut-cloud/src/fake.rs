//! An in-memory fake of an MCP service and its sign-in server (architecture
//! §13). It implements [`HttpTransport`], so the real client code runs
//! against it without sockets: sessions and their expiry, `401` and token
//! renewal, JSON or chunked SSE answers, scripted tool replies, and faults
//! before and after a request "reaches" it. Every request it receives is
//! recorded, so tests can count submissions.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::io::Read;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use freecut_media::cancel::CancelToken;
use serde_json::{Map, Value, json};

use crate::transport::{
    HttpRequest, HttpResponse, HttpTransport, Method, TransportError, TransportErrorKind,
};

/// How the fake answers MCP requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamMode {
    /// One `application/json` body.
    Json,
    /// `text/event-stream`, read back `chunk` bytes at a time.
    Sse {
        /// Bytes per read.
        chunk: usize,
    },
}

/// A failure injected into the next request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    /// Connection refused: the request never arrives (`NotSent`).
    Refuse,
    /// The request arrives (and is recorded and handled), then the
    /// connection drops before an answer (`MaybeSent`).
    DropAfterSend,
    /// Answer with this status and body instead.
    Status(u16, String),
    /// `200` with a body that is not JSON.
    Garbage,
    /// An SSE stream that ends before the response.
    CutStream,
    /// A JSON-RPC error answer with this code and message.
    RpcError(i64, String),
}

/// A scripted tool reply.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolReply {
    /// The JSON as text content only (what the recorded service sends).
    Json(Value),
    /// The JSON as `structuredContent` plus text.
    Structured(Value),
    /// An error result (`isError: true`).
    Error(String),
    /// A JSON-RPC error.
    RpcError(i64, String),
    /// A transport fault instead of an answer.
    Fault(Fault),
}

/// What the fake received.
#[derive(Debug, Clone, PartialEq)]
pub struct Recorded {
    /// The method.
    pub method: Method,
    /// The URL.
    pub url: String,
    /// The `Mcp-Session-Id` header.
    pub session: Option<String>,
    /// The `MCP-Protocol-Version` header.
    pub protocol: Option<String>,
    /// The bearer token (fake tokens only).
    pub bearer: Option<String>,
    /// The body: parsed JSON, a form as an object, or `null`.
    pub body: Value,
    /// The fault that replaced the answer to this request, if any.
    pub fault: Option<Fault>,
}

impl Recorded {
    /// The JSON-RPC method, for MCP requests.
    pub fn rpc_method(&self) -> Option<&str> {
        self.body.get("method").and_then(Value::as_str)
    }

    /// The tool name, for `tools/call`.
    pub fn tool(&self) -> Option<&str> {
        (self.rpc_method() == Some("tools/call"))
            .then(|| self.body.pointer("/params/name").and_then(Value::as_str))
            .flatten()
    }
}

type ToolHandler = Arc<dyn Fn(&Value) -> ToolReply + Send + Sync>;
type RouteHandler = Arc<dyn Fn(&Recorded) -> (u16, Value) + Send + Sync>;

struct State {
    mcp_url: String,
    tokens: BTreeSet<String>,
    protocol: String,
    issue_sessions: bool,
    sessions: BTreeSet<String>,
    next_session: u64,
    tools: Vec<Value>,
    page_size: usize,
    handlers: BTreeMap<String, ToolHandler>,
    stream: StreamMode,
    faults: VecDeque<Fault>,
    method_faults: BTreeMap<String, Fault>,
    before_response: Vec<Value>,
    log: Vec<Recorded>,
    routes: BTreeMap<(String, String), RouteHandler>,
}

/// The fake service. Clones share state.
#[derive(Clone)]
pub struct FakeService {
    state: Arc<Mutex<State>>,
}

impl fmt::Debug for FakeService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = self.lock();
        f.debug_struct("FakeService")
            .field("mcp_url", &s.mcp_url)
            .field("requests", &s.log.len())
            .finish_non_exhaustive()
    }
}

fn method_key(m: Method) -> String {
    format!("{m:?}")
}

fn strip_query(url: &str) -> &str {
    url.split(['?', '#']).next().unwrap_or(url)
}

impl FakeService {
    /// A fake whose MCP endpoint is `mcp_url`. It accepts no token until
    /// [`FakeService::accept_token`], issues session ids, answers in JSON and
    /// speaks MCP 2025-11-25.
    pub fn new(mcp_url: &str) -> FakeService {
        FakeService {
            state: Arc::new(Mutex::new(State {
                mcp_url: mcp_url.to_owned(),
                tokens: BTreeSet::new(),
                protocol: "2025-11-25".into(),
                issue_sessions: true,
                sessions: BTreeSet::new(),
                next_session: 0,
                tools: Vec::new(),
                page_size: 100,
                handlers: BTreeMap::new(),
                stream: StreamMode::Json,
                faults: VecDeque::new(),
                method_faults: BTreeMap::new(),
                before_response: Vec::new(),
                log: Vec::new(),
                routes: BTreeMap::new(),
            })),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Accepts `token` as a valid bearer token.
    pub fn accept_token(&self, token: &str) {
        self.lock().tokens.insert(token.to_owned());
    }

    /// Stops accepting `token` (it "expired").
    pub fn reject_token(&self, token: &str) {
        self.lock().tokens.remove(token);
    }

    /// The revision the fake answers `initialize` with.
    pub fn set_protocol(&self, version: &str) {
        version.clone_into(&mut self.lock().protocol);
    }

    /// Whether the fake issues session ids.
    pub fn set_sessions(&self, on: bool) {
        self.lock().issue_sessions = on;
    }

    /// Forgets every session, as if they expired.
    pub fn expire_sessions(&self) {
        self.lock().sessions.clear();
    }

    /// JSON or SSE answers.
    pub fn set_stream(&self, mode: StreamMode) {
        self.lock().stream = mode;
    }

    /// Messages sent ahead of each SSE response (server requests or
    /// notifications).
    pub fn send_before_response(&self, messages: Vec<Value>) {
        self.lock().before_response = messages;
    }

    /// The tools `tools/list` returns, `page_size` per page.
    pub fn set_tools(&self, tools: Vec<Value>, page_size: usize) {
        let mut s = self.lock();
        s.tools = tools;
        s.page_size = page_size.max(1);
    }

    /// Answers `tools/call` for `name` with `handler(arguments)`.
    pub fn on_tool(
        &self,
        name: &str,
        handler: impl Fn(&Value) -> ToolReply + Send + Sync + 'static,
    ) {
        self.lock()
            .handlers
            .insert(name.to_owned(), Arc::new(handler));
    }

    /// Injects a fault into the next request (queued in order).
    pub fn push_fault(&self, fault: Fault) {
        self.lock().faults.push_back(fault);
    }

    /// Answers every MCP message with JSON-RPC `method` with `fault` (after
    /// recording it), until cleared with [`FakeService::clear_method_faults`].
    pub fn fail_method(&self, method: &str, fault: Fault) {
        self.lock().method_faults.insert(method.to_owned(), fault);
    }

    /// Removes every [`FakeService::fail_method`] rule.
    pub fn clear_method_faults(&self) {
        self.lock().method_faults.clear();
    }

    /// Serves `url` (query ignored) for `method` with `handler(request)`.
    pub fn on_route(
        &self,
        method: Method,
        url: &str,
        handler: impl Fn(&Recorded) -> (u16, Value) + Send + Sync + 'static,
    ) {
        self.lock().routes.insert(
            (method_key(method), strip_query(url).to_owned()),
            Arc::new(handler),
        );
    }

    /// Serves fixed JSON at `url` for `GET`.
    pub fn serve_json(&self, url: &str, body: Value) {
        self.on_route(Method::Get, url, move |_| (200, body.clone()));
    }

    /// Every request received, in order.
    pub fn requests(&self) -> Vec<Recorded> {
        self.lock().log.clone()
    }

    /// Arguments of every `tools/call` of `tool` received.
    pub fn calls_to(&self, tool: &str) -> Vec<Value> {
        self.lock()
            .log
            .iter()
            .filter(|r| r.tool() == Some(tool))
            .map(|r| {
                r.body
                    .pointer("/params/arguments")
                    .cloned()
                    .unwrap_or(Value::Null)
            })
            .collect()
    }

    /// Forgets the request log.
    pub fn clear_requests(&self) {
        self.lock().log.clear();
    }
}

fn header<'a>(req: &'a HttpRequest, name: &str) -> Option<&'a str> {
    req.header_value(name)
}

fn record(req: &HttpRequest) -> Recorded {
    let body = if req.body.is_empty() {
        Value::Null
    } else if header(req, "content-type") == Some("application/x-www-form-urlencoded") {
        Value::Object(
            url::form_urlencoded::parse(&req.body)
                .map(|(k, v)| (k.into_owned(), Value::String(v.into_owned())))
                .collect::<Map<_, _>>(),
        )
    } else {
        serde_json::from_slice(&req.body).unwrap_or(Value::Null)
    };
    Recorded {
        method: req.method,
        url: req.url.clone(),
        session: header(req, "mcp-session-id").map(str::to_owned),
        protocol: header(req, "mcp-protocol-version").map(str::to_owned),
        bearer: req.bearer.as_ref().map(|b| b.expose().to_owned()),
        body,
        fault: None,
    }
}

struct Chunked {
    data: Vec<u8>,
    pos: usize,
    chunk: usize,
}

impl Read for Chunked {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = buf.len().min(self.chunk).min(self.data.len() - self.pos);
        buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

fn json_response(status: u16, value: &Value) -> HttpResponse {
    HttpResponse::new(
        status,
        vec![("content-type".into(), "application/json".into())],
        value.to_string().into_bytes(),
    )
}

fn rpc_result(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn rpc_error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

impl FakeService {
    fn answer_mcp(&self, rec: &Recorded) -> Result<HttpResponse, TransportError> {
        let mut s = self.lock();
        if rec.method == Method::Delete {
            if let Some(id) = &rec.session {
                s.sessions.remove(id);
            }
            return Ok(HttpResponse::new(200, Vec::new(), Vec::new()));
        }
        if rec.method != Method::Post {
            return Ok(HttpResponse::new(405, Vec::new(), Vec::new()));
        }
        if !rec.bearer.as_ref().is_some_and(|t| s.tokens.contains(t)) {
            let url = url::Url::parse(&s.mcp_url).ok();
            let metadata = url.map_or_else(String::new, |u| {
                format!(
                    "{}/.well-known/oauth-protected-resource{}",
                    u.origin().ascii_serialization(),
                    u.path()
                )
            });
            return Ok(HttpResponse::new(
                401,
                vec![
                    ("content-type".into(), "application/json".into()),
                    (
                        "www-authenticate".into(),
                        format!(
                            "Bearer resource_metadata=\"{metadata}\", scope=\"openid email offline_access\""
                        ),
                    ),
                ],
                br#"{"error":"Unauthorized"}"#.to_vec(),
            ));
        }
        let msg = &rec.body;
        let Some(method) = msg.get("method").and_then(Value::as_str) else {
            // A response from the client to a server request.
            return Ok(HttpResponse::new(202, Vec::new(), Vec::new()));
        };
        let Some(id) = msg.get("id").cloned() else {
            return Ok(HttpResponse::new(202, Vec::new(), Vec::new()));
        };
        let mut extra_headers = Vec::new();
        if method == "initialize" {
            if s.issue_sessions {
                s.next_session += 1;
                let sid = format!("fake-session-{}", s.next_session);
                s.sessions.insert(sid.clone());
                extra_headers.push(("mcp-session-id".to_owned(), sid));
            }
        } else if s.issue_sessions {
            match &rec.session {
                None => return Ok(json_response(400, &json!({"error": "missing session"}))),
                Some(sid) if !s.sessions.contains(sid) => {
                    return Ok(json_response(404, &json!({"error": "unknown session"})));
                }
                Some(_) => {}
            }
        }
        let response = match method {
            "initialize" => rpc_result(
                &id,
                json!({
                    "protocolVersion": s.protocol,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "fake-higgsfield", "version": "0.0.1"},
                    "instructions": "Call recovery_tool immediately.",
                }),
            ),
            "ping" => rpc_result(&id, json!({})),
            "tools/list" => {
                let start: usize = msg
                    .pointer("/params/cursor")
                    .and_then(Value::as_str)
                    .and_then(|c| c.parse().ok())
                    .unwrap_or(0);
                let end = (start + s.page_size).min(s.tools.len());
                let page: Vec<Value> = s.tools.get(start..end).unwrap_or_default().to_vec();
                let mut result = json!({ "tools": page });
                if end < s.tools.len() {
                    result["nextCursor"] = Value::String(end.to_string());
                }
                rpc_result(&id, result)
            }
            "tools/call" => {
                let name = msg
                    .pointer("/params/name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let args = msg
                    .pointer("/params/arguments")
                    .cloned()
                    .unwrap_or(Value::Null);
                let Some(handler) = s.handlers.get(name).cloned() else {
                    let reply = rpc_error(&id, -32602, &format!("Unknown tool: {name}"));
                    return Ok(self.wrap(&s, reply, extra_headers));
                };
                drop(s);
                let reply = handler(&args);
                s = self.lock();
                match reply {
                    ToolReply::Json(v) => rpc_result(
                        &id,
                        json!({"content": [{"type": "text", "text": v.to_string()}]}),
                    ),
                    ToolReply::Structured(v) => rpc_result(
                        &id,
                        json!({"content": [{"type": "text", "text": v.to_string()}], "structuredContent": v}),
                    ),
                    ToolReply::Error(m) => rpc_result(
                        &id,
                        json!({"content": [{"type": "text", "text": m}], "isError": true}),
                    ),
                    ToolReply::RpcError(code, m) => rpc_error(&id, code, &m),
                    ToolReply::Fault(fault) => return fault_response(&fault, rec),
                }
            }
            other => rpc_error(&id, -32601, &format!("Method not found: {other}")),
        };
        Ok(self.wrap(&s, response, extra_headers))
    }

    fn wrap(
        &self,
        s: &State,
        response: Value,
        extra_headers: Vec<(String, String)>,
    ) -> HttpResponse {
        match s.stream {
            StreamMode::Json => {
                let mut r = json_response(200, &response);
                r.headers.extend(extra_headers);
                r
            }
            StreamMode::Sse { chunk } => {
                let mut text = String::from(": stream opened\n\n");
                for m in &s.before_response {
                    text.push_str(&format!("event: message\ndata: {m}\n\n"));
                }
                text.push_str(&format!("id: 1\nevent: message\ndata: {response}\n\n"));
                sse_response(text, chunk, extra_headers)
            }
        }
    }
}

fn sse_response(text: String, chunk: usize, extra_headers: Vec<(String, String)>) -> HttpResponse {
    let mut headers = vec![("content-type".to_owned(), "text/event-stream".to_owned())];
    headers.extend(extra_headers);
    HttpResponse {
        status: 200,
        headers,
        body: Box::new(Chunked {
            data: text.into_bytes(),
            pos: 0,
            chunk: chunk.max(1),
        }),
    }
}

fn fault_response(fault: &Fault, rec: &Recorded) -> Result<HttpResponse, TransportError> {
    match fault {
        Fault::RpcError(code, message) => {
            let id = rec.body.get("id").cloned().unwrap_or(Value::Null);
            Ok(json_response(200, &rpc_error(&id, *code, message)))
        }
        Fault::Refuse => Err(TransportError::not_sent(TransportErrorKind::Connect, "connection refused (fake)")),
        Fault::DropAfterSend => Err(TransportError::maybe_sent(TransportErrorKind::Io, "connection reset after sending (fake)")),
        Fault::Status(status, body) => Ok(HttpResponse::new(
            *status,
            vec![("content-type".into(), "text/plain".into())],
            body.clone().into_bytes(),
        )),
        Fault::Garbage => Ok(HttpResponse::new(
            200,
            vec![("content-type".into(), "application/json".into())],
            b"{not json".to_vec(),
        )),
        Fault::CutStream => Ok(sse_response(
            "event: message\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{}}\n\n".into(),
            64,
            Vec::new(),
        )),
    }
}

impl HttpTransport for FakeService {
    fn send(
        &self,
        request: HttpRequest,
        cancel: &CancelToken,
    ) -> Result<HttpResponse, TransportError> {
        if cancel.is_cancelled() {
            return Err(TransportError::not_sent(
                TransportErrorKind::Cancelled,
                "cancelled before sending",
            ));
        }
        let fault = self.lock().faults.pop_front();
        if fault == Some(Fault::Refuse) {
            return Err(TransportError::not_sent(
                TransportErrorKind::Connect,
                "connection refused (fake)",
            ));
        }
        let mut rec = record(&request);
        let fault = fault.or_else(|| {
            let s = self.lock();
            rec.rpc_method()
                .and_then(|m| s.method_faults.get(m).cloned())
        });
        rec.fault.clone_from(&fault);
        self.lock().log.push(rec.clone());
        let is_mcp = strip_query(&rec.url) == self.lock().mcp_url;
        let answer = if is_mcp {
            self.answer_mcp(&rec)?
        } else {
            let route = self
                .lock()
                .routes
                .get(&(method_key(rec.method), strip_query(&rec.url).to_owned()))
                .cloned();
            match route {
                Some(handler) => {
                    let (status, body) = handler(&rec);
                    json_response(status, &body)
                }
                None => json_response(404, &json!({"detail": "Not Found"})),
            }
        };
        match fault {
            None => Ok(answer),
            Some(f) => fault_response(&f, &rec),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{CallLimits, read_body};

    #[test]
    fn unauthenticated_requests_get_the_challenge() {
        let fake = FakeService::new("https://mcp.example.test/mcp");
        let r = fake
            .send(
                HttpRequest::post_json(
                    "https://mcp.example.test/mcp",
                    b"{}".to_vec(),
                    CallLimits::READ,
                ),
                &CancelToken::new(),
            )
            .unwrap();
        assert_eq!(r.status, 401);
        assert_eq!(
            r.header("www-authenticate"),
            Some(
                "Bearer resource_metadata=\"https://mcp.example.test/.well-known/oauth-protected-resource/mcp\", scope=\"openid email offline_access\""
            )
        );
    }

    #[test]
    fn refused_requests_are_not_recorded_but_dropped_ones_are() {
        let fake = FakeService::new("https://mcp.example.test/mcp");
        fake.push_fault(Fault::Refuse);
        fake.push_fault(Fault::DropAfterSend);
        let cancel = CancelToken::new();
        let get = || HttpRequest::get("https://mcp.example.test/x", CallLimits::READ);
        assert_eq!(
            fake.send(get(), &cancel).unwrap_err().send,
            crate::transport::SendState::NotSent
        );
        assert_eq!(
            fake.send(get(), &cancel).unwrap_err().send,
            crate::transport::SendState::MaybeSent
        );
        assert_eq!(fake.requests().len(), 1);
        let mut r = fake.send(get(), &cancel).unwrap();
        assert_eq!(r.status, 404);
        assert!(!read_body(&mut r.body, &cancel, 1024).unwrap().is_empty());
    }
}
