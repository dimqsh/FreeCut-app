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
//! `models_explore` listings and the session `DELETE`. Before anything
//! leaves the computer, the recording transport refuses every body but the
//! few the tool sends (see [`refusal`]): no other tool, free or paid, no
//! other action, no batch, nothing that names `get_cost`.
//!
//! What is written (`tests/fixtures/higgsfield/recorded/free-calls.jsonl`,
//! one exchange per line) is scrubbed first. Every secret the run holds (the
//! client id, state, PKCE verifier, code, tokens, and the e-mail and subject
//! in the ID token) is replaced wherever it appears, raw, percent-encoded or
//! JSON-escaped, and the file is not written if any is left. The account's
//! allowance (`unlim`, and its prose) gets fixed values. Only the headers on
//! [`RECORDED_HEADERS`] are kept, request and response (no `Authorization`,
//! cookies, trace ids or CDN headers); token, code, client id and other
//! secret fields are replaced;
//! e-mail addresses, JWTs and UUIDs are replaced by stable fakes; session
//! ids become `fake-session-N`; signed URLs lose their query. The owner
//! reviews the file before committing it. The access token lives in memory
//! only and is dropped at exit; nothing is stored in the keychain.
//!
//! `--filter-headers <file>` applies the header allow list to a file recorded
//! before the list existed, without signing in or calling the service.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use freecut_cloud::CancelToken;
use freecut_cloud::catalogue::OutputKind;
use freecut_cloud::guard::{PreflightGuard, uuid_spans};
use freecut_cloud::http::{TransportConfig, UreqTransport};
use freecut_cloud::mcp::{LateQueue, StaticToken};
use freecut_cloud::oauth::pkce::{Pkce, random_state};
use freecut_cloud::oauth::{OAuthClient, authorize_request, parse_callback};
use freecut_cloud::provider::GenerationProvider;
use freecut_cloud::provider::higgsfield::{ENDPOINT, HiggsfieldProvider, auth_policy};
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
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--filter-headers") {
        let path = args.get(i + 1).ok_or("--filter-headers needs a file")?;
        return filter_file(Path::new(path));
    }
    let print_url = args.iter().any(|a| a == "--print-url");
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
    // Every secret this run holds, replaced before writing and checked
    // after (review R22). Taken before the access token moves below.
    let known = known_secrets(
        &registration.client_id,
        &request.state,
        request.pkce.verifier(),
        &code,
        &tokens,
    );
    drop(registration);
    println!("Signed in. Recording the free calls.");

    // The service: handshake, tool list, model listings, session end.
    let provider = HiggsfieldProvider::new(
        recorder.clone(),
        Arc::new(StaticToken(tokens.access)),
        Arc::new(SystemWaiter),
        env!("CARGO_PKG_VERSION"),
        Arc::new(PreflightGuard::new()),
        Arc::new(LateQueue::new()),
    );
    let info = provider.connect(&cancel)?;
    let catalogue = provider.catalogue(&cancel)?;
    provider.close();

    let lines = recorder.lines_without(&known);
    let left = leaks(&lines, &known);
    if !left.is_empty() {
        return Err(format!(
            "refusing to write the recording: it still holds {}",
            left.join(", ")
        )
        .into());
    }
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

/// Rewrites a recorded file keeping only [`RECORDED_HEADERS`] (review round
/// 3): no network, no sign-in. Everything but the dropped headers stays as
/// it was.
fn filter_file(path: &Path) -> anyhow_free::Result {
    let text = std::fs::read_to_string(path)?;
    let mut dropped = BTreeSet::new();
    let mut out = String::with_capacity(text.len());
    let mut lines = 0;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let value: Value = serde_json::from_str(line)?;
        out.push_str(&filter_headers(&value, &mut dropped).to_string());
        out.push('\n');
        lines += 1;
    }
    std::fs::write(path, out)?;
    println!(
        "Kept only the allowed headers in {lines} exchanges of {}.\nDropped: {}",
        path.display(),
        dropped.into_iter().collect::<Vec<_>>().join(", ")
    );
    Ok(())
}

/// `line` with every header off [`RECORDED_HEADERS`] removed from its request
/// and answer; the names removed are added to `dropped`.
fn filter_headers(line: &Value, dropped: &mut BTreeSet<String>) -> Value {
    let mut line = line.clone();
    for part in ["request", "response"] {
        if let Some(headers) = line
            .get_mut(part)
            .and_then(|p| p.get_mut("headers"))
            .and_then(Value::as_object_mut)
        {
            headers.retain(|name, _| {
                let keep = recorded_header(name);
                if !keep {
                    dropped.insert(name.clone());
                }
                keep
            });
        }
    }
    line
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

/// A transport that refuses anything but free calls and keeps every
/// exchange (headers filtered, secret fields redacted) in memory until the
/// file is written; the rest of the scrubbing happens then, once every
/// secret of the run is known.
struct Recorder {
    inner: Arc<dyn HttpTransport>,
    raw: Mutex<Vec<Value>>,
}

impl Recorder {
    fn new(inner: Arc<dyn HttpTransport>) -> Recorder {
        Recorder {
            inner,
            raw: Mutex::new(Vec::new()),
        }
    }

    /// The scrubbed lines, with no known secrets (tests).
    #[cfg(test)]
    fn lines(&self) -> Vec<Value> {
        self.lines_without(&[])
    }

    /// The lines with each known secret replaced wherever it appears, then
    /// scrubbed, with fakes that are stable across the file.
    fn lines_without(&self, known: &[Known]) -> Vec<Value> {
        let mut scrub = Scrubber::default();
        self.raw
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|line| scrub.value(&replace_known(line, known)))
            .collect()
    }

    fn record(&self, exchange: Value) {
        self.raw
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(exchange);
    }
}

/// A secret the run holds, and what replaces it in the recording.
struct Known {
    value: freecut_cloud::secret::Secret,
    label: &'static str,
}

impl Known {
    fn new(value: &str, label: &'static str) -> Known {
        Known {
            value: freecut_cloud::secret::Secret::new(value.to_owned()),
            label,
        }
    }

    /// The forms the value can take in a recording: raw, percent-encoded
    /// (with `+` or `%20` for spaces) and JSON-escaped; longest first.
    fn forms(&self) -> Vec<String> {
        let raw = self.value.expose();
        let form: String = url::form_urlencoded::byte_serialize(raw.as_bytes()).collect();
        let escaped = serde_json::to_string(raw).unwrap_or_default();
        let mut forms = vec![
            raw.to_owned(),
            form.replace('+', "%20"),
            form,
            escaped.trim_matches('"').to_owned(),
        ];
        forms.retain(|f| !f.is_empty());
        forms.sort_by_key(|f| std::cmp::Reverse(f.len()));
        forms.dedup();
        forms
    }
}

/// The secrets of a signed-in run (review R22): the client id, the state,
/// the PKCE verifier, the code, the tokens, and the e-mail and subject the
/// ID token names.
fn known_secrets(
    client_id: &str,
    state: &freecut_cloud::secret::Secret,
    verifier: &freecut_cloud::secret::Secret,
    code: &freecut_cloud::secret::Secret,
    tokens: &freecut_cloud::oauth::token::TokenSet,
) -> Vec<Known> {
    let mut known = vec![
        Known::new(client_id, "FAKE_CLIENT_ID"),
        Known::new(state.expose(), "<state>"),
        Known::new(verifier.expose(), "<code-verifier>"),
        Known::new(code.expose(), "<code>"),
        Known::new(tokens.access.expose(), "<access-token>"),
    ];
    if let Some(refresh) = &tokens.refresh {
        known.push(Known::new(refresh.expose(), "<refresh-token>"));
    }
    if let Some(id) = &tokens.id_token {
        known.push(Known::new(id.expose(), "<id-token>"));
        if let Some(claims) = freecut_cloud::oauth::token::id_token_claims(id.expose()) {
            if let Some(email) = &claims.email {
                known.push(Known::new(email, "user@example.test"));
            }
            if let Some(sub) = &claims.subject {
                known.push(Known::new(sub, "<sub>"));
            }
        }
    }
    known
}

/// The labels of the known secrets still present, in any form, in `lines`.
fn leaks(lines: &[Value], known: &[Known]) -> Vec<&'static str> {
    let text: String = lines.iter().map(Value::to_string).collect();
    known
        .iter()
        .filter(|k| k.forms().iter().any(|f| text.contains(f.as_str())))
        .map(|k| k.label)
        .collect()
}

impl std::fmt::Debug for Recorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Recorder").finish_non_exhaustive()
    }
}

/// The JSON-RPC methods the tool sends to the service, besides
/// `tools/call` for `models_explore` with action `list` (review R25).
const ALLOWED_METHODS: &[&str] = &[
    "initialize",
    "notifications/initialized",
    "tools/list",
    "ping",
    "notifications/cancelled",
];

/// Why `body`, sent to `url`, may not leave (review R25). The service gets
/// exactly what `main` sends: the handshake, `tools/list`, `ping`,
/// `notifications/cancelled`, `models_explore` with action `list`, and
/// replies to the server's requests (an `id` with a `result` or `error`).
/// Everything else is refused: another tool, free ones included
/// (`media_upload` creates upload slots, `balance` would record credit
/// figures), another action, a method that is not exactly one of those
/// strings, a batch, a body that is not JSON. At any URL, a body that names
/// `get_cost`, or a JSON-RPC message to another host, is refused.
fn refusal(url: &str, body: &[u8]) -> Option<String> {
    if body.is_empty() {
        return None;
    }
    if String::from_utf8_lossy(body).contains("get_cost") {
        return Some("a cost check".into());
    }
    let parsed = serde_json::from_slice::<Value>(body);
    if !url.starts_with(ENDPOINT) {
        // Sign-in: a form or a registration document, never JSON-RPC.
        return match parsed {
            Ok(Value::Object(m)) if m.contains_key("method") || m.contains_key("jsonrpc") => {
                Some("a JSON-RPC message to another host".into())
            }
            Ok(Value::Array(_)) => Some("a JSON array to another host".into()),
            _ => None,
        };
    }
    let message = match parsed {
        Ok(message @ Value::Object(_)) => message,
        Ok(Value::Array(_)) => return Some("a JSON-RPC batch".into()),
        _ => return Some("a body that is not one JSON-RPC object".into()),
    };
    match message.get("method") {
        None => {
            let reply = message.get("id").is_some()
                && (message.get("result").is_some() || message.get("error").is_some());
            (!reply).then(|| "a message that is neither a request nor a reply".into())
        }
        Some(Value::String(m)) if ALLOWED_METHODS.contains(&m.as_str()) => None,
        Some(Value::String(m)) if m == "tools/call" => {
            let name = message.pointer("/params/name").and_then(Value::as_str);
            let action = message
                .pointer("/params/arguments/action")
                .and_then(Value::as_str);
            match (name, action) {
                (Some("models_explore"), Some("list")) => None,
                _ => Some(format!(
                    "tool {name:?} with action {action:?}, which the recorder does not send"
                )),
            }
        }
        Some(other) => Some(format!("method {other}, which the recorder does not send")),
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

/// `value` with every form of every known secret replaced by its label, in
/// strings and in object keys.
fn replace_known(value: &Value, known: &[Known]) -> Value {
    let text = |s: &str| {
        known.iter().fold(s.to_owned(), |s, k| {
            k.forms()
                .iter()
                .fold(s, |s, form| s.replace(form.as_str(), k.label))
        })
    };
    match value {
        Value::String(s) => Value::String(text(s)),
        Value::Array(items) => {
            Value::Array(items.iter().map(|v| replace_known(v, known)).collect())
        }
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (text(k), replace_known(v, known)))
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

/// JSON stays JSON, a form has its secret fields replaced, other text (an
/// event stream) stays text. The scrubber redacts secret fields in JSON and
/// in every event's data at write time.
fn body_json(bytes: &[u8]) -> Value {
    if bytes.is_empty() {
        return Value::Null;
    }
    if let Ok(value) = serde_json::from_slice::<Value>(bytes) {
        return value;
    }
    let text = String::from_utf8_lossy(bytes);
    if text.contains('=') && !text.contains(char::is_whitespace) {
        return Value::String(redact::form(&text));
    }
    Value::String(text.into_owned())
}

/// What replaces the allowance prose a listing carries (review R24).
const ALLOWANCE_PROSE: &str =
    "Allowance: not recorded (the FreeCut recorder removes the account's allowance).";

/// The `unlim` allowance with fixed values of the same shape: false, 0,
/// `<removed>`, null and empty lists (review R24).
fn fake_allowance(value: &Value) -> Value {
    match value {
        Value::Bool(_) => Value::Bool(false),
        Value::Number(_) => json!(0),
        Value::String(_) => Value::String("<removed>".into()),
        Value::Array(_) => json!([]),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), fake_allowance(v)))
                .collect(),
        ),
        Value::Null => Value::Null,
    }
}

/// A text item that describes the allowance in prose (it names `unlim` and
/// is not JSON).
fn allowance_prose(item: &serde_json::Map<String, Value>) -> bool {
    item.get("type").and_then(Value::as_str) == Some("text")
        && item.get("text").and_then(Value::as_str).is_some_and(|t| {
            t.to_ascii_lowercase().contains("unlim") && serde_json::from_str::<Value>(t).is_err()
        })
}

/// Whether `key` names a secret field (`redact::SECRET_FIELDS`).
fn secret_key(key: &str) -> bool {
    redact::SECRET_FIELDS
        .iter()
        .any(|f| f.eq_ignore_ascii_case(key))
}

/// The start of every fake UUID the scrubber writes.
const FAKE_UUID_PREFIX: &str = "00000000-0000-4000-8000-";

/// UUIDs the scrubber leaves alone: the nil and max values (they appear in
/// the service's public schemas) and its own fakes.
fn keeps(uuid: &str) -> bool {
    let digits = uuid.bytes().filter(|b| *b != b'-');
    uuid.starts_with(FAKE_UUID_PREFIX)
        || digits.clone().all(|b| b == b'0')
        || digits.into_iter().all(|b| b.eq_ignore_ascii_case(&b'f'))
}

/// Replaces what is personal or secret in recorded text with stable fakes.
#[derive(Debug, Default)]
struct Scrubber {
    fakes: BTreeMap<String, String>,
}

impl Scrubber {
    fn value(&mut self, value: &Value) -> Value {
        match value {
            Value::String(s) => Value::String(self.string(s)),
            Value::Array(items) => Value::Array(items.iter().map(|v| self.value(v)).collect()),
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| {
                        let v = if k == "mcp-session-id" {
                            Value::String(self.fake(v.as_str().unwrap_or_default(), "fake-session"))
                        } else if k == "unlim" {
                            fake_allowance(v)
                        } else if k == "text" && allowance_prose(map) {
                            Value::String(ALLOWANCE_PROSE.into())
                        } else if secret_key(k)
                            && matches!(v, Value::String(_) | Value::Number(_) | Value::Bool(_))
                        {
                            // Only a scalar is a secret: a JSON Schema object
                            // under `code` or `token` stays (review R23).
                            Value::String("<redacted>".into())
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

    /// A string value: an event stream is scrubbed line by line, each
    /// `data:` line as parsed JSON; a string that holds JSON (a tool result's
    /// text item) is parsed, scrubbed and written back; anything else is
    /// scrubbed as text. Scrubbing parsed JSON keeps it valid: replacing
    /// inside escaped JSON could drop an escaping backslash (review R21).
    fn string(&mut self, s: &str) -> String {
        if s.lines().any(|l| l.starts_with("data:")) {
            return s
                .lines()
                .map(|line| match line.strip_prefix("data:") {
                    Some(data) => match serde_json::from_str::<Value>(data.trim()) {
                        Ok(v) => format!("data: {}", self.value(&v)),
                        Err(_) => self.text(line),
                    },
                    None => self.text(line),
                })
                .collect::<Vec<_>>()
                .join("\n");
        }
        let trimmed = s.trim_start();
        if (trimmed.starts_with('{') || trimmed.starts_with('['))
            && let Ok(v) = serde_json::from_str::<Value>(s)
        {
            return self.value(&v).to_string();
        }
        self.text(s)
    }

    /// UUIDs, e-mail addresses and JWTs wherever they sit (review R21), then
    /// word by word: signed URL queries.
    fn text(&mut self, text: &str) -> String {
        let text = self.uuids(text);
        let text = self.emails(&text);
        let text = jwts(&text);
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

    /// Replaces every UUID-shaped run of 36 characters, wherever it sits
    /// (the guard's scan, with no token boundaries): a whole word, inside
    /// escaped JSON, in a URL path, after a prefix (`job-<id>`), right after
    /// an escape that ends in a hex digit (`%2F<id>`, `\\b<id>`; review R28).
    /// Replacing too much is the safe direction. The nil and max UUIDs are
    /// values, not identifiers, and stay; so do the fakes.
    fn uuids(&mut self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        for (start, end) in uuid_spans(text) {
            let uuid = &text[start..end];
            if keeps(uuid) {
                continue;
            }
            out.push_str(&text[last..start]);
            let n = self.fakes.len() + 1;
            out.push_str(
                self.fakes
                    .entry(uuid.to_ascii_lowercase())
                    .or_insert_with(|| format!("{FAKE_UUID_PREFIX}{n:012}")),
            );
            last = end;
        }
        out.push_str(&text[last..]);
        out
    }

    /// Every e-mail address, written with `@` or `%40`: only the address is
    /// replaced, so surrounding escapes and URLs stay intact. The local part
    /// never takes a character that an escaping backslash precedes.
    fn emails(&mut self, text: &str) -> String {
        let b = text.as_bytes();
        let local = |c: u8| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'+' | b'-');
        let domain = |c: u8| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-');
        let mut out = String::with_capacity(text.len());
        let (mut last, mut i) = (0, 0);
        while i < b.len() {
            let at = if b[i] == b'@' {
                1
            } else if b[i..].starts_with(b"%40") {
                3
            } else {
                0
            };
            if at == 0 {
                i += 1;
                continue;
            }
            let mut l = i;
            while l > last && local(b[l - 1]) && !(l >= 2 && b[l - 2] == b'\\') {
                l -= 1;
            }
            let mut r = i + at;
            while r < b.len() && domain(b[r]) {
                r += 1;
            }
            while r > i + at && matches!(b[r - 1], b'.' | b'-') {
                r -= 1;
            }
            let host = &text[i + at..r];
            let tld_ok = host
                .rsplit('.')
                .next()
                .is_some_and(|t| t.len() >= 2 && t.bytes().all(|c| c.is_ascii_alphabetic()));
            if l < i && host.contains('.') && tld_ok {
                out.push_str(&text[last..l]);
                out.push_str("user");
                out.push_str(&text[i..i + at]);
                out.push_str("example.test");
                last = r;
                i = r;
            } else {
                i += 1;
            }
        }
        out.push_str(&text[last..]);
        out
    }

    fn word(&mut self, word: &str) -> String {
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

/// Every JWT (`eyJ…` with at least three dot-separated base64url parts, JWS
/// or JWE), wherever it sits: after `=` or `;`, in brackets, before a full
/// stop (review R21).
fn jwts(text: &str) -> String {
    let b = text.as_bytes();
    let part = |c: u8| c.is_ascii_alphanumeric() || c == b'-' || c == b'_';
    let mut out = String::with_capacity(text.len());
    let (mut last, mut i) = (0, 0);
    while i + 3 <= b.len() {
        if &b[i..i + 3] == b"eyJ" {
            let (mut j, mut end, mut parts) = (i, i, 0);
            loop {
                let start = j;
                while j < b.len() && part(b[j]) {
                    j += 1;
                }
                if j == start {
                    break;
                }
                parts += 1;
                end = j;
                if j < b.len() && b[j] == b'.' {
                    j += 1;
                } else {
                    break;
                }
            }
            if parts >= 3 {
                out.push_str(&text[last..i]);
                out.push_str("<jwt>");
                last = end;
                i = end;
                continue;
            }
        }
        i += 1;
    }
    out.push_str(&text[last..]);
    out
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
    use freecut_cloud::fake::{FakeService, StreamMode, ToolReply};
    use freecut_cloud::mcp::{McpClient, McpError, RunTag};
    use freecut_cloud::provider::higgsfield::allow;
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
            Arc::new(LateQueue::new()),
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

    /// Every `data:` line of a recorded event stream, parsed.
    fn data_lines(lines: &[Value]) -> Vec<Value> {
        lines
            .iter()
            .filter_map(|l| l["response"]["body"].as_str())
            .flat_map(|body| body.lines().filter_map(|l| l.strip_prefix("data:")))
            .filter(|d| !d.trim().is_empty())
            .map(|d| serde_json::from_str(d.trim()).unwrap_or_else(|e| panic!("{e}: {d}")))
            .collect()
    }

    /// R21: over SSE, as the service answers, the e-mail and the id inside
    /// the escaped tool result are scrubbed, e-mails and JWTs are found
    /// anywhere, and every data line still parses.
    #[test]
    fn sse_answers_are_scrubbed_and_stay_valid_json() {
        let fake = fake();
        fake.set_stream(StreamMode::Sse { chunk: 64 });
        fake.on_tool("models_explore", |_| {
            ToolReply::Json(json!({"items": [], "has_more": false,
                "contact": "owner@example.com",
                "link": "https://x.example/u?who=owner%40example.com&next=1",
                "auth": "Bearer=eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2lnbmF0dXJl;",
                "job": "0B4F6D2A-1111-4222-8333-944455556666"}))
        });
        let recorder = Arc::new(Recorder::new(Arc::new(fake.clone())));
        let provider = HiggsfieldProvider::with_endpoint(
            ENDPOINT,
            recorder.clone(),
            Arc::new(StaticToken(Secret::new("t1"))),
            Arc::new(NoWait),
            "0.1.0",
            Arc::new(PreflightGuard::new()),
            Arc::new(LateQueue::new()),
        );
        provider.catalogue(&CancelToken::new()).unwrap();
        let lines = recorder.lines();
        let data = data_lines(&lines);
        assert!(!data.is_empty());
        let text = lines.iter().map(Value::to_string).collect::<String>();
        for leak in [
            "owner@example.com",
            "owner%40example.com",
            "0b4f6d2a",
            "0B4F6D2A",
            "eyJzdWIiOiIxIn0",
        ] {
            assert!(!text.contains(leak), "{leak} was recorded");
        }
        assert!(text.contains("example.test"));
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
            .call_paid(
                &allow::GENERATE_IMAGE,
                json!({"params": {"model": "m"}}),
                &RunTag::new("record-test"),
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
                .check_cost(
                    &allow::GENERATE_IMAGE,
                    &quote,
                    &RunTag::new("record-test"),
                    &CancelToken::new()
                )
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

    /// Round 3: only a single JSON-RPC object goes to the service. A batch is
    /// refused even when every call in it is free; so is a body that is not
    /// JSON.
    #[test]
    fn batches_and_other_bodies_never_leave() {
        let fake = fake();
        let recorder = Recorder::new(Arc::new(fake.clone()));
        let call = |name: &str, id: i64| {
            json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
                "params": {"name": name, "arguments": {"params": {"model": "m"}}}})
        };
        for body in [
            json!([call("generate_image", 1)]).to_string().into_bytes(),
            json!([call("models_explore", 1), call("models_explore", 2)])
                .to_string()
                .into_bytes(),
            b"{not json".to_vec(),
            b"\"tools/call\"".to_vec(),
        ] {
            let request = HttpRequest::post_json(
                ENDPOINT,
                body.clone(),
                freecut_cloud::transport::CallLimits::READ,
            )
            .with_bearer(Secret::new("t1"));
            let err = recorder.send(request, &CancelToken::new()).unwrap_err();
            assert_eq!(
                err.send,
                freecut_cloud::transport::SendState::NotSent,
                "{}",
                String::from_utf8_lossy(&body)
            );
        }
        assert!(fake.requests().is_empty());
        // One object still goes.
        let one = HttpRequest::post_json(
            ENDPOINT,
            json!({"jsonrpc": "2.0", "id": 1, "method": "ping"})
                .to_string()
                .into_bytes(),
            freecut_cloud::transport::CallLimits::READ,
        )
        .with_bearer(Secret::new("t1"));
        assert!(recorder.send(one, &CancelToken::new()).is_ok());
        assert_eq!(fake.requests().len(), 1);
    }

    /// R25: the gate lets through exactly what `main` sends: the handshake,
    /// `tools/list`, `ping`, `notifications/cancelled`, `models_explore`
    /// with action `list`, and replies. Every other call, free tools
    /// included, is refused before sending, at any URL.
    #[test]
    fn the_gate_allows_only_the_calls_the_tool_makes() {
        let call = |name: &str, args: Value| {
            json!({"jsonrpc": "2.0", "id": 9, "method": "tools/call",
                "params": {"name": name, "arguments": args}})
        };
        let refused = [
            (
                ENDPOINT,
                call("media_upload", json!({"files": [{"name": "a.png"}]})),
            ),
            (ENDPOINT, call("balance", json!({}))),
            (ENDPOINT, call("jobs_wait", json!({"job_ids": []}))),
            (
                ENDPOINT,
                call("models_explore", json!({"action": "recommend"})),
            ),
            (
                ENDPOINT,
                call(
                    "models_explore",
                    json!({"action": "list", "type": "image", "get_cost": true}),
                ),
            ),
            (
                ENDPOINT,
                json!({"jsonrpc": "2.0", "id": 9, "method": "Tools/Call",
                "params": {"name": "generate_image"}}),
            ),
            (
                ENDPOINT,
                json!({"jsonrpc": "2.0", "id": 9, "method": ["tools/call"],
                "params": {"name": "generate_image"}}),
            ),
            (
                ENDPOINT,
                json!({"jsonrpc": "2.0", "method": "resources/list"}),
            ),
            (ENDPOINT, json!({"jsonrpc": "2.0", "id": 9})),
            (
                "https://clerk.example/oauth/token",
                call("models_explore", json!({"action": "list"})),
            ),
        ];
        for (url, body) in &refused {
            assert!(
                refusal(url, body.to_string().as_bytes()).is_some(),
                "{url} {body}"
            );
        }
        let allowed = [
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
            json!({"jsonrpc": "2.0", "id": 3, "method": "ping"}),
            json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 3}}),
            call(
                "models_explore",
                json!({"action": "list", "type": "video", "limit": 100}),
            ),
            json!({"jsonrpc": "2.0", "id": "s1", "result": {}}),
            json!({"jsonrpc": "2.0", "id": "s2", "error": {"code": -32601, "message": "x"}}),
        ];
        for body in &allowed {
            assert_eq!(
                refusal(ENDPOINT, body.to_string().as_bytes()),
                None,
                "{body}"
            );
        }
        // OAuth requests: a form and a registration body go.
        assert_eq!(
            refusal(
                "https://clerk.example/oauth/token",
                b"grant_type=authorization_code&code=x"
            ),
            None
        );
        assert_eq!(
            refusal(
                "https://clerk.example/oauth/register",
                json!({"client_name": "FreeCut", "redirect_uris": ["http://127.0.0.1/callback"]})
                    .to_string()
                    .as_bytes()
            ),
            None
        );
    }

    /// Round 3: the one-off pass over an older recording keeps the same
    /// headers the recorder keeps, and nothing else changes.
    #[test]
    fn the_one_off_pass_keeps_only_allowed_headers() {
        let line = json!({
            "request": {"method": "POST", "url": ENDPOINT, "body": {"method": "initialize"},
                "headers": {"accept": "application/json", "mcp-protocol-version": "2025-11-25"}},
            "response": {"status": 200, "body": "event: message",
                "headers": {"content-type": "text/event-stream", "set-cookie": "__cf_bm=abc",
                    "x-country": "KZ", "cf-ray": "8c1f-ALA", "x-higgsfield-build": "b1"}},
        });
        let mut dropped = BTreeSet::new();
        let out = filter_headers(&line, &mut dropped);
        assert_eq!(
            out["request"]["headers"],
            json!({"mcp-protocol-version": "2025-11-25"})
        );
        assert_eq!(
            out["response"]["headers"],
            json!({"content-type": "text/event-stream", "x-higgsfield-build": "b1"})
        );
        assert_eq!(
            dropped.into_iter().collect::<Vec<_>>(),
            vec!["accept", "cf-ray", "set-cookie", "x-country"]
        );
        let mut rest = out.clone();
        let mut original = line.clone();
        for v in [&mut rest, &mut original] {
            v["request"]["headers"] = Value::Null;
            v["response"]["headers"] = Value::Null;
        }
        assert_eq!(rest, original);
        // An exchange that ended in an error has no answer; it passes as is.
        let failed = json!({"request": {"headers": {}}, "error": "x"});
        assert_eq!(filter_headers(&failed, &mut BTreeSet::new()), failed);
    }

    /// Round 3 check: a UUID is replaced wherever it sits, not only as a
    /// whole word; the nil and max UUIDs of a public schema stay.
    #[test]
    fn uuids_are_replaced_anywhere_but_nil_and_max_stay() {
        let id = "7c9e6679-7425-40de-944b-e07fc1f90ae7";
        let mut s = Scrubber::default();
        let escaped = s.text(&format!(r#"data: {{"text":"{{\"job_id\":\"{id}\"}}"}}"#));
        let in_path = s.text(&format!("https://cdn.example/media/{id}.png"));
        let prefixed = s.text(&format!("job-{}", id.to_uppercase()));
        for out in [&escaped, &in_path, &prefixed] {
            assert!(!out.to_ascii_lowercase().contains(id), "{out}");
            assert!(
                out.contains("00000000-0000-4000-8000-000000000001"),
                "{out}"
            );
        }
        let pattern = "^([0-9a-f]{8}-…|00000000-0000-0000-0000-000000000000|ffffffff-ffff-ffff-ffff-ffffffffffff)$";
        assert_eq!(s.text(pattern), pattern);
        // R28: right after an escape that ends in a hex digit (percent, \b,
        // \f, \u00XX), and glued to more hex digits (the safe direction).
        for text in [
            format!("next=%2Fjobs%2F{id}"),
            format!("id%3D{id}"),
            format!("%22{id}%22"),
            format!("https://x/_next/image?url=https%3A%2F%2Fcdn.example%2F{id}.webp&w=640"),
            format!("\\b{id}"),
            format!("\\f{id}"),
            format!("\\u0022{id}"),
            format!("{id}0"),
        ] {
            let out = s.text(&text);
            assert!(!out.to_ascii_lowercase().contains(id), "{text} -> {out}");
        }
    }

    /// Answers with every planted secret in its body (raw, percent-encoded,
    /// inside escaped JSON) and in an allowed header; fails the request whose
    /// URL ends in `/fail` with a secret in the error.
    #[derive(Debug)]
    struct Plants(Vec<String>);

    impl HttpTransport for Plants {
        fn send(
            &self,
            request: HttpRequest,
            _: &CancelToken,
        ) -> Result<HttpResponse, TransportError> {
            if request.url.ends_with("/fail") {
                return Err(TransportError::maybe_sent(
                    TransportErrorKind::Io,
                    format!("reset while sending {}", self.0[0]),
                ));
            }
            let pct: Vec<String> = self
                .0
                .iter()
                .map(|v| url::form_urlencoded::byte_serialize(v.as_bytes()).collect())
                .collect();
            let body = json!({
                "raw": self.0,
                "link": format!("https://x.example/cb?v={}", pct.join("&w=")),
                "text": json!({"nested": self.0}).to_string(),
            });
            Ok(HttpResponse::new(
                200,
                vec![
                    ("content-type".into(), "application/json".into()),
                    (
                        "www-authenticate".into(),
                        format!("Bearer error={}", self.0[1]),
                    ),
                ],
                body.to_string().into_bytes(),
            ))
        }
    }

    /// R22: every secret the tool holds is replaced wherever it appears, in
    /// raw, percent-encoded and JSON-escaped form, before the file is
    /// written; one that is left makes the write refuse.
    #[test]
    fn no_secret_held_in_memory_is_written() {
        let values = [
            "at-RAW+access/token==",
            "rt.refresh\"token\"",
            "code value 1",
            "verifier_ABC-123",
            "st4te",
            "owner+tag@example.com",
            "user_2xYzSub",
            "client_abc123",
        ];
        let known: Vec<Known> = values
            .iter()
            .zip([
                "<access-token>",
                "<refresh-token>",
                "<code>",
                "<code-verifier>",
                "<state>",
                "user@example.test",
                "<sub>",
                "FAKE_CLIENT_ID",
            ])
            .map(|(v, label)| Known::new(v, label))
            .collect();
        let recorder = Recorder::new(Arc::new(Plants(
            values.iter().map(|v| (*v).to_owned()).collect(),
        )));
        let limits = freecut_cloud::transport::CallLimits::READ;
        recorder
            .send(
                HttpRequest::get("https://clerk.example/a", limits),
                &CancelToken::new(),
            )
            .unwrap();
        assert!(
            recorder
                .send(
                    HttpRequest::get("https://clerk.example/fail", limits),
                    &CancelToken::new()
                )
                .is_err()
        );
        let lines = recorder.lines_without(&known);
        assert_eq!(leaks(&lines, &known), Vec::<&str>::new());
        let text = lines.iter().map(Value::to_string).collect::<String>();
        for v in values {
            assert!(!text.contains(v), "{v} was written");
        }
        // In what was recorded, before any replacement, the check finds
        // them all.
        let raw = recorder.raw.lock().unwrap().clone();
        assert_eq!(leaks(&raw, &known).len(), values.len());
    }

    /// R23: a secret field name redacts only a scalar value. A JSON Schema
    /// property named `code` or `token` (an object) survives, in a JSON body
    /// and inside an event stream; a token string or number does not.
    #[test]
    fn only_scalar_secret_values_are_redacted() {
        let schema = json!({"type": "object", "properties": {
            "code": {"type": "string", "description": "Python code to run"},
            "lease": {"type": "object", "properties": {"token": {"type": "string"}}}}});
        let answer = json!({"jsonrpc": "2.0", "id": 2, "result": {"tools": [
            {"name": "scene_builder_3d_run_python", "inputSchema": schema}]}});
        let tokens = json!({"access_token": "at-1", "code": "c-1", "expires_in": 3600,
            "token": 12345, "refresh_token": null});
        let mut s = Scrubber::default();
        let in_json = s.value(&answer);
        assert_eq!(in_json["result"]["tools"][0]["inputSchema"], schema);
        let sse = Value::String(format!("event: message\ndata: {answer}\n\n"));
        let in_stream = s.value(&sse);
        let data = in_stream
            .as_str()
            .unwrap()
            .lines()
            .find_map(|l| l.strip_prefix("data: "))
            .unwrap();
        let parsed: Value = serde_json::from_str(data).unwrap();
        assert_eq!(parsed["result"]["tools"][0]["inputSchema"], schema);
        assert_eq!(
            s.value(&tokens),
            json!({"access_token": "<redacted>", "code": "<redacted>", "expires_in": 3600,
                "token": "<redacted>", "refresh_token": null})
        );
        // Through the recorder: a JSON answer keeps its schema too.
        assert_eq!(
            s.value(&body_json(answer.to_string().as_bytes()))["result"]["tools"][0]["inputSchema"],
            schema
        );
    }

    /// R24: the account's allowance is not recorded: the `unlim` object in
    /// every copy gets fixed values of the same shape, and the allowance
    /// prose fixed text. Other text items stay.
    #[test]
    fn the_accounts_allowance_is_not_recorded() {
        let fake = fake();
        fake.set_stream(StreamMode::Sse { chunk: 64 });
        fake.on_tool("models_explore", |_| {
            let listing = json!({"items": [], "has_more": false, "unlim":
                {"available": true, "remaining": 4242, "expires_at": "2031-07-15T00:00:00Z"}});
            ToolReply::Raw(json!({"structuredContent": listing, "content": [
                {"type": "text", "text": listing.to_string()},
                {"type": "text", "text": "Free-trial unlim: 4242 generations left until 2031-07-15."},
                {"type": "text", "text": "Models are listed by type."}]}))
        });
        let recorder = Arc::new(Recorder::new(Arc::new(fake.clone())));
        let provider = HiggsfieldProvider::with_endpoint(
            ENDPOINT,
            recorder.clone(),
            Arc::new(StaticToken(Secret::new("t1"))),
            Arc::new(NoWait),
            "0.1.0",
            Arc::new(PreflightGuard::new()),
            Arc::new(LateQueue::new()),
        );
        provider.catalogue(&CancelToken::new()).unwrap();
        let lines = recorder.lines();
        let bodies: String = lines
            .iter()
            .map(|l| l["response"]["body"].to_string())
            .collect();
        assert!(
            !bodies.contains("4242") && !bodies.contains("2031-07-15"),
            "{bodies}"
        );
        assert!(bodies.contains("Models are listed by type."));
        let data = data_lines(&lines);
        let result = &data.last().unwrap()["result"];
        let fake_unlim = json!({"available": false, "remaining": 0, "expires_at": "<removed>"});
        assert_eq!(result["structuredContent"]["unlim"], fake_unlim);
        let text: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(text["unlim"], fake_unlim);
        assert_eq!(result["content"][1]["text"], ALLOWANCE_PROSE);
    }

    #[test]
    fn a_known_value_is_replaced_wherever_it_appears() {
        let line = json!({"response": {"body": {
            "client_id": "<redacted>",
            "registration_client_uri": "https://clerk.example/oauth/register/client_abc123",
            "list": ["x client_abc123 y"],
        }}});
        let out = replace_known(&line, &[Known::new("client_abc123", "FAKE_CLIENT_ID")]);
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
