//! Records FreeCut's own free exchanges with Higgsfield as test fixtures
//! (architecture §13, review round 2 "next steps"). Run by the owner, who
//! signs in once in the browser:
//!
//! ```text
//! cargo run -p freecut-cloud --features http --example record
//! ```
//!
//! **Free calls only**: sign-in discovery, one client registration, the
//! browser sign-in and its code exchange, then `initialize`, `tools/list`,
//! `models_explore` listings and the session `DELETE`. No cost check and no
//! paid tool: the recording transport refuses any `tools/call` whose tool is
//! not free on the allow list, and any request that carries `get_cost`,
//! before it leaves the computer.
//!
//! What is written (`tests/fixtures/higgsfield/recorded/free-calls.jsonl`,
//! one exchange per line) is scrubbed first: only the headers on
//! [`RECORDED_HEADERS`] are kept, request and response (no `Authorization`,
//! cookies, trace ids or CDN headers); token, code, client id and other
//! secret fields are replaced;
//! e-mail addresses, JWTs and UUIDs are replaced by stable fakes; session
//! ids become `fake-session-N`; signed URLs lose their query. The owner
//! reviews the file before committing it. The access token lives in memory
//! only and is dropped at exit; nothing is stored in the keychain.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use freecut_cloud::CancelToken;
use freecut_cloud::catalogue::OutputKind;
use freecut_cloud::guard::{PreflightGuard, is_uuid};
use freecut_cloud::http::{TransportConfig, UreqTransport};
use freecut_cloud::mcp::{CallClass, StaticToken};
use freecut_cloud::oauth::pkce::{Pkce, random_state};
use freecut_cloud::oauth::{OAuthClient, authorize_request, parse_callback};
use freecut_cloud::provider::GenerationProvider;
use freecut_cloud::provider::higgsfield::{ENDPOINT, HiggsfieldProvider, allow, auth_policy};
use freecut_cloud::redact;
use freecut_cloud::transport::{
    HttpRequest, HttpResponse, HttpTransport, Method, SystemWaiter, TransportError,
    TransportErrorKind, read_body,
};
use serde_json::{Value, json};

/// The only headers recorded, on requests and answers (default deny). Every
/// other header may tie the file to the account, the machine or the CDN
/// session: cookies, trace and ray ids, the caller's country, report
/// endpoints (review round 3).
const RECORDED_HEADERS: &[&str] = &[
    "content-type",
    "cache-control",
    "date",
    "mcp-session-id",
    "mcp-protocol-version",
    "www-authenticate",
    "x-higgsfield-build",
];

/// The longest answer recorded.
const MAX_RECORDED_BYTES: usize = 32 * 1024 * 1024;
/// How long the owner has to finish signing in.
const SIGN_IN_WAIT: Duration = Duration::from_secs(300);

fn main() -> anyhow_free::Result {
    let print_url = std::env::args().any(|a| a == "--print-url");
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/higgsfield/recorded/free-calls.jsonl");
    let cancel = CancelToken::new();
    let recorder = Arc::new(Recorder::new(Arc::new(UreqTransport::new(
        &TransportConfig::default(),
    ))));

    // Sign-in: discovery, registration, browser, code exchange.
    let oauth = OAuthClient::new(recorder.clone(), auth_policy());
    let found = oauth.discover(None, &cancel)?;
    let registration = oauth.register(&found.server, &cancel)?;
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    let request = authorize_request(
        &found.server,
        &registration.client_id,
        port,
        oauth.policy(),
        Pkce::generate()?,
        random_state()?,
    )?;
    println!("Sign in to Higgsfield in the browser window that opens (5 minutes).");
    if print_url || !open_in_browser(&request.url) {
        println!("Open this address to sign in:\n{}", request.url);
    }
    let code = wait_for_callback(&listener, &request.state, &found.server)?;
    let tokens = oauth.exchange_code(
        &found.server,
        &registration.client_id,
        &code,
        &request.redirect_uri,
        request.pkce.verifier(),
        &cancel,
    )?;
    println!("Signed in. Recording the free calls.");

    // The service: handshake, tool list, model listings, session end.
    let provider = HiggsfieldProvider::new(
        recorder.clone(),
        Arc::new(StaticToken(tokens.access)),
        Arc::new(SystemWaiter),
        env!("CARGO_PKG_VERSION"),
        Arc::new(PreflightGuard::new()),
    );
    let info = provider.connect(&cancel)?;
    let catalogue = provider.catalogue(&cancel)?;
    provider.close();

    // The client id can also sit inside other values (a registration URL).
    let lines = recorder.lines_without(&[(registration.client_id.as_str(), "FAKE_CLIENT_ID")]);
    drop(registration);
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = std::fs::File::create(&out)?;
    for line in &lines {
        writeln!(file, "{line}")?;
    }
    let count = |kind: OutputKind| {
        catalogue
            .models()
            .iter()
            .filter(|m| m.output == kind)
            .count()
    };
    println!(
        "Wrote {} exchanges to {}.\nServer {} {}, protocol {}; {} allowed tools offered, missing {:?}.\nModels listed: {} image, {} video, {} audio ({} shown after review).\nReview the file before committing it.",
        lines.len(),
        out.display(),
        info.server_name,
        info.server_version,
        info.protocol_version,
        info.offered.len(),
        info.missing,
        count(OutputKind::Image),
        count(OutputKind::Video),
        count(OutputKind::Audio),
        catalogue.visible().count(),
    );
    Ok(())
}

/// Opens `url` with the platform's opener; false if that failed.
fn open_in_browser(url: &str) -> bool {
    let mut command = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else if cfg!(windows) {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", ""]);
        c
    } else {
        std::process::Command::new("xdg-open")
    };
    command
        .arg(url)
        .status()
        .is_ok_and(|status| status.success())
}

/// Serves the loopback redirect until the sign-in answer arrives.
fn wait_for_callback(
    listener: &TcpListener,
    state: &freecut_cloud::secret::Secret,
    server: &freecut_cloud::oauth::discovery::AuthServer,
) -> anyhow_free::Result<freecut_cloud::secret::Secret> {
    let deadline = Instant::now() + SIGN_IN_WAIT;
    listener.set_nonblocking(true)?;
    while Instant::now() < deadline {
        let mut stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line)?;
        let target = line
            .split_whitespace()
            .nth(1)
            .unwrap_or_default()
            .to_owned();
        let result = parse_callback(&target, state, server);
        let page = match &result {
            Ok(_) => "Signed in. You can close this tab and go back to the terminal.",
            Err(_) => "This is not the sign-in answer FreeCut is waiting for.",
        };
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}",
            page.len()
        );
        match result {
            Ok(code) => return Ok(code),
            Err(freecut_cloud::oauth::CallbackError::NotTheCallback) => {}
            Err(e) => return Err(e.into()),
        }
    }
    Err("sign-in was not finished within 5 minutes".into())
}

/// A transport that refuses anything but free calls and keeps a scrubbed
/// copy of every exchange.
struct Recorder {
    inner: Arc<dyn HttpTransport>,
    lines: Mutex<Vec<Value>>,
    scrub: Mutex<Scrubber>,
}

impl Recorder {
    fn new(inner: Arc<dyn HttpTransport>) -> Recorder {
        Recorder {
            inner,
            lines: Mutex::new(Vec::new()),
            scrub: Mutex::new(Scrubber::default()),
        }
    }

    fn lines(&self) -> Vec<Value> {
        self.lines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The recorded lines with each known value replaced wherever it
    /// appears.
    fn lines_without(&self, known: &[(&str, &str)]) -> Vec<Value> {
        self.lines()
            .iter()
            .map(|line| replace_known(line, known))
            .collect()
    }

    fn record(&self, exchange: Value) {
        let scrubbed = self
            .scrub
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .value(&exchange);
        self.lines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(scrubbed);
    }
}

impl std::fmt::Debug for Recorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Recorder").finish_non_exhaustive()
    }
}

/// Why `body`, sent to the MCP endpoint, may not leave: a tool call that is
/// not free on the allow list, or anything carrying `get_cost`.
fn refusal(url: &str, body: &[u8]) -> Option<String> {
    if !url.starts_with(ENDPOINT) || body.is_empty() {
        return None;
    }
    let text = String::from_utf8_lossy(body);
    if text.contains("get_cost") {
        return Some("a cost check".into());
    }
    let message: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    if message.get("method").and_then(Value::as_str) != Some("tools/call") {
        return None;
    }
    let name = message
        .pointer("/params/name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match allow::lookup(name) {
        Some(tool) if tool.class() == CallClass::Free => None,
        _ => Some(format!("tool {name}, which is not a free call")),
    }
}

impl HttpTransport for Recorder {
    fn send(
        &self,
        request: HttpRequest,
        cancel: &CancelToken,
    ) -> Result<HttpResponse, TransportError> {
        if let Some(why) = refusal(&request.url, &request.body) {
            return Err(TransportError::not_sent(
                TransportErrorKind::Other,
                format!("the recorder sends free calls only; refused {why}"),
            ));
        }
        let method = match request.method {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Delete => "DELETE",
        };
        let sent = json!({
            "method": method,
            "url": request.url,
            // The bearer token travels outside `headers` and is never recorded.
            "headers": headers_json(&request.headers),
            "body": body_json(&request.body),
        });
        let mut response = match self.inner.send(request, cancel) {
            Ok(response) => response,
            Err(e) => {
                self.record(json!({"request": sent, "error": e.to_string()}));
                return Err(e);
            }
        };
        let body = read_body(&mut response.body, cancel, MAX_RECORDED_BYTES)
            .map_err(|e| TransportError::maybe_sent(TransportErrorKind::Io, e.to_string()))?;
        self.record(json!({
            "request": sent,
            "response": {
                "status": response.status,
                "headers": headers_json(&response.headers),
                "body": body_json(&body),
            },
        }));
        Ok(HttpResponse::new(response.status, response.headers, body))
    }
}

fn replace_known(value: &Value, known: &[(&str, &str)]) -> Value {
    match value {
        Value::String(s) => Value::String(
            known
                .iter()
                .filter(|(real, _)| !real.is_empty())
                .fold(s.clone(), |s, (real, fake)| s.replace(real, fake)),
        ),
        Value::Array(items) => {
            Value::Array(items.iter().map(|v| replace_known(v, known)).collect())
        }
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), replace_known(v, known)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn recorded_header(name: &str) -> bool {
    RECORDED_HEADERS
        .iter()
        .any(|h| h.eq_ignore_ascii_case(name))
}

/// The headers worth recording, names lower-cased; everything else dropped.
fn headers_json(headers: &[(String, String)]) -> Value {
    Value::Object(
        headers
            .iter()
            .filter(|(k, _)| recorded_header(k))
            .map(|(k, v)| (k.to_ascii_lowercase(), Value::String(v.clone())))
            .collect(),
    )
}

/// JSON stays JSON, forms and other text become strings; secret fields are
/// replaced either way.
fn body_json(bytes: &[u8]) -> Value {
    if bytes.is_empty() {
        return Value::Null;
    }
    if let Ok(value) = serde_json::from_slice::<Value>(bytes) {
        return redact::json(&value);
    }
    let text = String::from_utf8_lossy(bytes);
    if text.contains('=') && !text.contains(char::is_whitespace) {
        return Value::String(redact::form(&text));
    }
    // SSE streams and other text: JSON inside `data:` lines is redacted.
    Value::String(
        text.lines()
            .map(|line| match line.strip_prefix("data:") {
                Some(data) => match serde_json::from_str::<Value>(data.trim()) {
                    Ok(v) => format!("data: {}", redact::json(&v)),
                    Err(_) => line.to_owned(),
                },
                None => line.to_owned(),
            })
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// Replaces what is personal or secret in recorded text with stable fakes.
#[derive(Debug, Default)]
struct Scrubber {
    fakes: BTreeMap<String, String>,
}

impl Scrubber {
    fn value(&mut self, value: &Value) -> Value {
        match value {
            Value::String(s) => Value::String(self.text(s)),
            Value::Array(items) => Value::Array(items.iter().map(|v| self.value(v)).collect()),
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| {
                        let v = if k == "mcp-session-id" {
                            Value::String(self.fake(v.as_str().unwrap_or_default(), "fake-session"))
                        } else {
                            self.value(v)
                        };
                        (k.clone(), v)
                    })
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    fn fake(&mut self, real: &str, prefix: &str) -> String {
        let next = self.fakes.len() + 1;
        self.fakes
            .entry(real.to_owned())
            .or_insert_with(|| format!("{prefix}-{next}"))
            .clone()
    }

    /// Word by word: UUIDs, e-mail addresses, JWTs and signed URL queries.
    fn text(&mut self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut word = String::new();
        let flush = |word: &mut String, out: &mut String, this: &mut Scrubber| {
            if !word.is_empty() {
                out.push_str(&this.word(word));
                word.clear();
            }
        };
        for c in text.chars() {
            if c.is_whitespace() || matches!(c, '"' | '\'' | ',' | '(' | ')' | '<' | '>') {
                flush(&mut word, &mut out, self);
                out.push(c);
            } else {
                word.push(c);
            }
        }
        flush(&mut word, &mut out, self);
        out
    }

    fn word(&mut self, word: &str) -> String {
        if is_uuid(word) {
            let n = self.fakes.len() + 1;
            return self
                .fakes
                .entry(word.to_ascii_lowercase())
                .or_insert_with(|| format!("00000000-0000-4000-8000-{n:012}"))
                .clone();
        }
        if word.starts_with("eyJ") && word.matches('.').count() == 2 {
            return "<jwt>".into();
        }
        if let Some((user, domain)) = word.split_once('@')
            && !user.is_empty()
            && domain.contains('.')
            && !word.contains("://")
        {
            return "user@example.test".into();
        }
        if (word.starts_with("https://") || word.starts_with("http://"))
            && let Some((base, query)) = word.split_once('?')
        {
            let lower = query.to_ascii_lowercase();
            if ["signature", "x-amz-", "sig=", "token", "expires"]
                .iter()
                .any(|marker| lower.contains(marker))
            {
                return format!("{base}?<signed query removed>");
            }
        }
        word.to_owned()
    }
}

/// Errors as text: the tool reports and exits.
mod anyhow_free {
    pub type Error = Box<dyn std::error::Error + Send + Sync>;
    pub type Result<T = ()> = std::result::Result<T, Error>;
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)] // test code
    use super::*;
    use freecut_cloud::fake::{FakeService, ToolReply};
    use freecut_cloud::mcp::{McpClient, McpError};
    use freecut_cloud::secret::Secret;

    /// Retries without sleeping.
    #[derive(Debug)]
    struct NoWait;

    impl freecut_cloud::transport::Waiter for NoWait {
        fn wait(&self, _: Duration, cancel: &CancelToken) -> bool {
            !cancel.is_cancelled()
        }

        fn jitter(&self) -> u32 {
            0
        }
    }

    fn fake() -> FakeService {
        let fake = FakeService::new(ENDPOINT);
        fake.accept_token("t1");
        fake.set_tools(
            ["models_explore", "generate_image", "generate_video"]
                .iter()
                .map(|n| json!({"name": n, "description": "", "inputSchema": {"type": "object"}}))
                .collect(),
            10,
        );
        fake.on_tool("models_explore", |_| {
            ToolReply::Json(
                json!({"items": [], "has_more": false, "contact": "owner@example.com",
                "job": "0B4F6D2A-1111-4222-8333-944455556666"}),
            )
        });
        fake.on_tool("generate_image", |_| {
            ToolReply::Json(json!({"cost": {"credits": 1}}))
        });
        fake
    }

    #[test]
    fn free_calls_are_recorded_and_scrubbed() {
        let fake = fake();
        let recorder = Arc::new(Recorder::new(Arc::new(fake.clone())));
        let provider = HiggsfieldProvider::with_endpoint(
            ENDPOINT,
            recorder.clone(),
            Arc::new(StaticToken(Secret::new("t1"))),
            Arc::new(NoWait),
            "0.1.0",
            Arc::new(PreflightGuard::new()),
        );
        provider.connect(&CancelToken::new()).unwrap();
        provider.catalogue(&CancelToken::new()).unwrap();
        provider.close();
        let text = recorder
            .lines()
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("models_explore"));
        assert!(!text.contains("t1\""), "the token was recorded");
        assert!(!text.to_ascii_lowercase().contains("bearer"));
        assert!(!text.contains("owner@example.com"));
        assert!(!text.contains("0b4f6d2a") && !text.contains("0B4F6D2A"));
        assert!(text.contains("\"mcp-session-id\":\"fake-session-"));
    }

    #[test]
    fn cost_checks_and_paid_tools_never_leave() {
        let fake = fake();
        let recorder: Arc<dyn HttpTransport> = Arc::new(Recorder::new(Arc::new(fake.clone())));
        let client = McpClient::new(
            ENDPOINT,
            recorder,
            Arc::new(StaticToken(Secret::new("t1"))),
            Arc::new(NoWait),
            "0.1.0",
        );
        client.initialize(&CancelToken::new()).unwrap();
        let err = client
            .call_tool(
                &allow::GENERATE_IMAGE,
                json!({"params": {"model": "m"}}),
                &CancelToken::new(),
            )
            .unwrap_err();
        assert!(
            matches!(err, McpError::Transport(e) if e.send == freecut_cloud::transport::SendState::NotSent)
        );
        let quote = freecut_cloud::guard::QuoteRequest::new(
            "generate_image",
            json!({"params": {"model": "m"}}),
        )
        .unwrap();
        assert!(
            client
                .check_cost(&allow::GENERATE_IMAGE, &quote, &CancelToken::new())
                .is_err()
        );
        assert!(fake.calls_to("generate_image").is_empty());
        // Free calls still go.
        client
            .call_tool(
                &allow::MODELS_EXPLORE,
                json!({"action": "list", "type": "image"}),
                &CancelToken::new(),
            )
            .unwrap();
    }

    /// A transport that answers every request with the same headers.
    #[derive(Debug)]
    struct Canned;

    impl HttpTransport for Canned {
        fn send(&self, _: HttpRequest, _: &CancelToken) -> Result<HttpResponse, TransportError> {
            let headers = [
                ("Content-Type", "application/json"),
                ("Date", "Tue, 14 Oct 2026 10:00:00 GMT"),
                ("X-Higgsfield-Build", "b1"),
                ("Set-Cookie", "__cf_bm=abc; path=/"),
                ("x-country", "KZ"),
                ("cf-ray", "8c1f-ALA"),
                ("x-clerk-trace-id", "t-123"),
                ("x-cloud-trace-context", "c-456"),
                ("report-to", "{\"group\":\"cf-nel\"}"),
                ("nel", "{\"report_to\":\"cf-nel\"}"),
                ("server", "cloudflare"),
            ];
            Ok(HttpResponse::new(
                200,
                headers
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                    .collect(),
                b"{}".to_vec(),
            ))
        }
    }

    /// Round 3: only allowed headers reach the file, request and answer.
    #[test]
    fn only_allowed_headers_are_recorded() {
        let recorder = Recorder::new(Arc::new(Canned));
        let mut request = HttpRequest::get(
            "https://clerk.higgsfield.ai/.well-known/oauth-authorization-server",
            freecut_cloud::transport::CallLimits::READ,
        )
        .header("Accept", "application/json")
        .header("Cookie", "__cf_bm=abc")
        .header("MCP-Protocol-Version", "2025-11-25")
        .with_bearer(Secret::new("t1"));
        request
            .headers
            .push(("authorization".into(), "Bearer t1".into()));
        recorder.send(request, &CancelToken::new()).unwrap();
        let line = &recorder.lines()[0];
        let names = |part: &str| -> Vec<String> {
            line[part]["headers"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect()
        };
        assert_eq!(names("request"), vec!["mcp-protocol-version"]);
        assert_eq!(
            names("response"),
            vec!["content-type", "date", "x-higgsfield-build"]
        );
        let text = line.to_string();
        for leak in [
            "__cf_bm", "KZ", "8c1f", "t-123", "c-456", "cf-nel", "Bearer",
        ] {
            assert!(!text.contains(leak), "{leak} was recorded");
        }
    }

    #[test]
    fn a_known_value_is_replaced_wherever_it_appears() {
        let line = json!({"response": {"body": {
            "client_id": "<redacted>",
            "registration_client_uri": "https://clerk.example/oauth/register/client_abc123",
            "list": ["x client_abc123 y"],
        }}});
        let out = replace_known(&line, &[("client_abc123", "FAKE_CLIENT_ID")]);
        assert!(!out.to_string().contains("client_abc123"));
        assert_eq!(
            out["response"]["body"]["registration_client_uri"],
            "https://clerk.example/oauth/register/FAKE_CLIENT_ID"
        );
    }

    #[test]
    fn scrubbing_is_stable_and_leaves_public_text_alone() {
        let mut s = Scrubber::default();
        let id = "7c9e6679-7425-40de-944b-e07fc1f90ae7";
        let once = s.text(&format!("job {id} again {id}"));
        assert_eq!(once.matches("00000000-0000-4000-8000-").count(), 2);
        assert_eq!(
            s.text("https://cdn.example/x.png?X-Amz-Signature=abc&X-Amz-Expires=60"),
            "https://cdn.example/x.png?<signed query removed>"
        );
        assert_eq!(s.text("eyJhbGciOi.eyJzdWIiOi.c2ln"), "<jwt>");
        assert_eq!(
            s.text("https://clerk.higgsfield.ai/oauth/authorize"),
            "https://clerk.higgsfield.ai/oauth/authorize"
        );
        assert_eq!(
            s.text("Most advanced cinema-grade model"),
            "Most advanced cinema-grade model"
        );
        let scrubbed =
            s.value(&json!({"mcp-session-id": "real-session-abc", "x": "owner@example.com"}));
        assert!(
            scrubbed["mcp-session-id"]
                .as_str()
                .unwrap()
                .starts_with("fake-session-")
        );
        assert_eq!(scrubbed["x"], "user@example.test");
        // The same session id maps to the same fake.
        assert_eq!(
            s.value(&json!({"mcp-session-id": "real-session-abc"}))["mcp-session-id"],
            scrubbed["mcp-session-id"]
        );
    }
}
