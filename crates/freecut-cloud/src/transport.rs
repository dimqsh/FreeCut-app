//! The HTTP seam between the sans-I/O core and the network.
//!
//! [`HttpTransport`] sends one request and returns the status, headers and a
//! streaming body. The real implementation is `http::UreqTransport` (feature
//! `http`); tests use [`crate::fake::FakeService`]. A failed send reports a
//! [`SendState`], the only input to the unknown-outcome rule (§4.5).

use std::fmt;
use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

use freecut_media::cancel::CancelToken;

use crate::secret::Secret;

/// HTTP methods FreeCut uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// `GET`
    Get,
    /// `POST`
    Post,
    /// `DELETE`
    Delete,
}

/// Connect and whole-call limits of one request (§4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallLimits {
    /// Time to resolve and connect, TLS included.
    pub connect: Duration,
    /// Time for the whole call, body included.
    pub total: Duration,
    /// Use a new connection instead of a pooled one. Paid submissions do, so a
    /// stale pooled socket cannot turn a refusal into an unknown outcome.
    pub fresh_connection: bool,
}

impl CallLimits {
    /// `initialize`, `tools/list`, the catalogue and other read-only calls.
    pub const READ: CallLimits = CallLimits {
        connect: Duration::from_secs(10),
        total: Duration::from_secs(30),
        fresh_connection: false,
    };
    /// A cost preflight (`get_cost: true`).
    pub const PREFLIGHT: CallLimits = CallLimits {
        connect: Duration::from_secs(10),
        total: Duration::from_secs(60),
        fresh_connection: false,
    };
    /// A paid submission.
    pub const PAID: CallLimits = CallLimits {
        connect: Duration::from_secs(10),
        total: Duration::from_secs(120),
        fresh_connection: true,
    };
    /// Sign-in requests (discovery, registration, tokens, revocation).
    pub const OAUTH: CallLimits = CallLimits {
        connect: Duration::from_secs(10),
        total: Duration::from_secs(30),
        fresh_connection: false,
    };
    /// Closing an MCP session on disconnect or quit (best effort).
    pub const CLOSE: CallLimits = CallLimits {
        connect: Duration::from_secs(2),
        total: Duration::from_secs(2),
        fresh_connection: false,
    };
}

/// An answer that arrived after the caller stopped waiting (§4.4, §7.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LateResponse {
    /// The status code.
    pub status: u16,
    /// Header names (lower-case) and values.
    pub headers: Vec<(String, String)>,
    /// The body, as much as arrived.
    pub body: Vec<u8>,
    /// Whether the body was read to its end.
    pub complete: bool,
}

/// Receives a [`LateResponse`]. Set on paid calls and cost checks, whose
/// answers may carry job ids that must reach the ledger even after a cancel.
pub type LateSink = Arc<dyn Fn(LateResponse) + Send + Sync>;

/// One HTTP request.
///
/// The bearer token is kept apart from the headers so that it is added by the
/// transport at the last moment and never appears in `Debug` output.
pub struct HttpRequest {
    /// The method.
    pub method: Method,
    /// The absolute URL.
    pub url: String,
    /// Header names (lower-case) and values, without `Authorization`.
    pub headers: Vec<(String, String)>,
    /// `Authorization: Bearer <token>` when set.
    pub bearer: Option<Secret>,
    /// The body; empty for `GET` and `DELETE`.
    pub body: Vec<u8>,
    /// Timeouts and connection reuse.
    pub limits: CallLimits,
    /// Where an answer goes that arrives after the caller cancelled. A
    /// transport that can abandon a request in flight must deliver it there.
    pub late: Option<LateSink>,
}

impl HttpRequest {
    /// A `GET` with no headers.
    pub fn get(url: impl Into<String>, limits: CallLimits) -> HttpRequest {
        HttpRequest {
            method: Method::Get,
            url: url.into(),
            headers: Vec::new(),
            bearer: None,
            body: Vec::new(),
            limits,
            late: None,
        }
    }

    /// A `POST` of a JSON body.
    pub fn post_json(url: impl Into<String>, body: Vec<u8>, limits: CallLimits) -> HttpRequest {
        HttpRequest {
            method: Method::Post,
            url: url.into(),
            headers: vec![("content-type".into(), "application/json".into())],
            bearer: None,
            body,
            limits,
            late: None,
        }
    }

    /// A `POST` of `application/x-www-form-urlencoded` pairs.
    pub fn post_form(
        url: impl Into<String>,
        pairs: &[(&str, &str)],
        limits: CallLimits,
    ) -> HttpRequest {
        let mut form = url::form_urlencoded::Serializer::new(String::new());
        for (k, v) in pairs {
            form.append_pair(k, v);
        }
        HttpRequest {
            method: Method::Post,
            url: url.into(),
            headers: vec![(
                "content-type".into(),
                "application/x-www-form-urlencoded".into(),
            )],
            bearer: None,
            body: form.finish().into_bytes(),
            limits,
            late: None,
        }
    }

    /// Adds a header (names are stored lower-case).
    #[must_use]
    pub fn header(mut self, name: &str, value: impl Into<String>) -> HttpRequest {
        self.headers.push((name.to_ascii_lowercase(), value.into()));
        self
    }

    /// Sets the bearer token.
    #[must_use]
    pub fn with_bearer(mut self, token: Secret) -> HttpRequest {
        self.bearer = Some(token);
        self
    }

    /// Sets where a late answer goes.
    #[must_use]
    pub fn with_late(mut self, sink: LateSink) -> HttpRequest {
        self.late = Some(sink);
        self
    }

    /// The first value of a header, by case-insensitive name.
    pub fn header_value(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

impl fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("headers", &self.headers)
            .field("bearer", &self.bearer.as_ref().map(|_| "<redacted>"))
            .field("body", &format_args!("<{} bytes>", self.body.len()))
            .field("limits", &self.limits)
            .field("late", &self.late.as_ref().map(|_| "<sink>"))
            .finish()
    }
}

/// A response whose body is still to be read.
pub struct HttpResponse {
    /// The status code.
    pub status: u16,
    /// Header names (lower-case) and values.
    pub headers: Vec<(String, String)>,
    /// The body, read in chunks so that cancellation is checked between them.
    pub body: Box<dyn Read + Send>,
}

impl HttpResponse {
    /// A response with an in-memory body.
    pub fn new(status: u16, headers: Vec<(String, String)>, body: Vec<u8>) -> HttpResponse {
        HttpResponse {
            status,
            headers: headers
                .into_iter()
                .map(|(k, v)| (k.to_ascii_lowercase(), v))
                .collect(),
            body: Box::new(std::io::Cursor::new(body)),
        }
    }

    /// The first value of a header, by case-insensitive name.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// The media type without parameters, lower-case (`application/json`).
    pub fn media_type(&self) -> Option<String> {
        self.header("content-type").map(|v| {
            v.split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
        })
    }
}

impl fmt::Debug for HttpResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpResponse")
            .field("status", &self.status)
            .field("headers", &self.headers)
            .finish_non_exhaustive()
    }
}

/// Whether a failed request may have reached the server (§4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendState {
    /// The failure happened before the first request byte could be written:
    /// the server cannot have acted on it.
    NotSent,
    /// The request may have been written, in part or in full.
    MaybeSent,
}

/// What kind of transport failure happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportErrorKind {
    /// The host name did not resolve.
    HostNotFound,
    /// The connection could not be opened.
    Connect,
    /// TLS failed.
    Tls,
    /// A phase timed out.
    Timeout,
    /// The connection broke while sending or receiving.
    Io,
    /// The caller cancelled while the request was in flight.
    Cancelled,
    /// Anything else.
    Other,
}

/// A request that got no HTTP answer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{kind:?} ({send:?}): {detail}")]
pub struct TransportError {
    /// What failed.
    pub kind: TransportErrorKind,
    /// Whether the server may have received the request.
    pub send: SendState,
    /// A description for the log (never contains tokens).
    pub detail: String,
}

impl TransportError {
    /// A failure before anything was sent.
    pub fn not_sent(kind: TransportErrorKind, detail: impl Into<String>) -> TransportError {
        TransportError {
            kind,
            send: SendState::NotSent,
            detail: detail.into(),
        }
    }

    /// A failure after the request may have been sent.
    pub fn maybe_sent(kind: TransportErrorKind, detail: impl Into<String>) -> TransportError {
        TransportError {
            kind,
            send: SendState::MaybeSent,
            detail: detail.into(),
        }
    }
}

/// Sends HTTP requests. Implementations block; they must stop waiting soon
/// after `cancel` fires (§4.4) and must never retry a request on their own.
pub trait HttpTransport: Send + Sync + fmt::Debug {
    /// Sends one request.
    ///
    /// # Errors
    /// [`TransportError`] when no HTTP answer arrived; any status code,
    /// including 4xx and 5xx, is an `Ok` response.
    fn send(
        &self,
        request: HttpRequest,
        cancel: &CancelToken,
    ) -> Result<HttpResponse, TransportError>;
}

/// Why a body could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BodyError {
    /// The caller cancelled.
    #[error("cancelled while reading the answer")]
    Cancelled,
    /// The body was longer than allowed.
    #[error("the answer is larger than {0} bytes")]
    TooLarge(usize),
    /// The connection broke.
    #[error("the answer was cut off: {0}")]
    Io(String),
}

/// Reads a whole body, at most `max` bytes, checking `cancel` between chunks.
///
/// # Errors
/// [`BodyError`] on cancellation, an oversized body or a broken connection.
pub fn read_body(
    body: &mut dyn Read,
    cancel: &CancelToken,
    max: usize,
) -> Result<Vec<u8>, BodyError> {
    let mut out = Vec::new();
    read_body_into(body, cancel, max, &mut out)?;
    Ok(out)
}

/// Like [`read_body`], but into `out`, which keeps what arrived when the
/// read fails (review R12: a broken answer may still name its jobs).
///
/// # Errors
/// As [`read_body`].
pub fn read_body_into(
    body: &mut dyn Read,
    cancel: &CancelToken,
    max: usize,
    out: &mut Vec<u8>,
) -> Result<(), BodyError> {
    let mut chunk = [0u8; 8 * 1024];
    loop {
        if cancel.is_cancelled() {
            return Err(BodyError::Cancelled);
        }
        match body.read(&mut chunk) {
            Ok(0) => return Ok(()),
            Ok(n) => {
                if out.len() + n > max {
                    return Err(BodyError::TooLarge(max));
                }
                out.extend_from_slice(&chunk[..n]);
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) if cancel.is_cancelled() => return Err(BodyError::Cancelled),
            Err(e) => return Err(BodyError::Io(e.to_string())),
        }
    }
}

/// Waits between retries; injectable so tests run without sleeping.
pub trait Waiter: Send + Sync + fmt::Debug {
    /// Waits `duration`, returning `false` early if `cancel` fires.
    fn wait(&self, duration: Duration, cancel: &CancelToken) -> bool;

    /// A random number for retry jitter.
    fn jitter(&self) -> u32;
}

/// Sleeps in short slices, checking the cancel token between them.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemWaiter;

impl Waiter for SystemWaiter {
    fn wait(&self, duration: Duration, cancel: &CancelToken) -> bool {
        let slice = Duration::from_millis(50);
        let mut left = duration;
        while !left.is_zero() {
            if cancel.is_cancelled() {
                return false;
            }
            let step = left.min(slice);
            std::thread::sleep(step);
            left = left.saturating_sub(step);
        }
        !cancel.is_cancelled()
    }

    fn jitter(&self) -> u32 {
        let mut bytes = [0u8; 4];
        // No randomness only means no jitter.
        getrandom::fill(&mut bytes).map_or(0, |()| u32::from_le_bytes(bytes))
    }
}

/// The delay before retry number `retry` (0-based): 1 s, 2 s, 4 s, …, each
/// ±25 % from `random` (§4.5).
pub fn backoff_delay(retry: u32, random: u32) -> Duration {
    let base_ms = 1_000u64 << retry.min(6);
    let spread = base_ms / 4;
    let offset = u64::from(random) % (2 * spread + 1);
    Duration::from_millis(base_ms - spread + offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_hides_the_bearer_token_and_body() {
        let req = HttpRequest::post_json(
            "https://x.test/mcp",
            b"{\"secret\":1}".to_vec(),
            CallLimits::READ,
        )
        .with_bearer(Secret::new("tok-123"));
        let shown = format!("{req:?}");
        assert!(!shown.contains("tok-123"), "{shown}");
        assert!(!shown.contains("secret"), "{shown}");
    }

    #[test]
    fn form_bodies_are_url_encoded() {
        let req = HttpRequest::post_form(
            "https://x.test/token",
            &[
                ("grant_type", "authorization_code"),
                ("redirect_uri", "http://127.0.0.1:5/callback"),
            ],
            CallLimits::OAUTH,
        );
        assert_eq!(
            String::from_utf8(req.body).unwrap(),
            "grant_type=authorization_code&redirect_uri=http%3A%2F%2F127.0.0.1%3A5%2Fcallback"
        );
    }

    #[test]
    fn backoff_is_one_two_four_seconds_within_a_quarter() {
        for (retry, base) in [(0u32, 1_000u64), (1, 2_000), (2, 4_000)] {
            for random in [0u32, 1, 499, 500, u32::MAX] {
                let d = backoff_delay(retry, random).as_millis() as u64;
                assert!(
                    d >= base * 3 / 4 && d <= base * 5 / 4,
                    "{retry} {random} {d}"
                );
            }
        }
    }

    #[test]
    fn read_body_stops_at_the_limit_and_on_cancel() {
        let cancel = CancelToken::new();
        let mut small = std::io::Cursor::new(vec![1u8; 10]);
        assert_eq!(read_body(&mut small, &cancel, 10).unwrap().len(), 10);
        let mut big = std::io::Cursor::new(vec![1u8; 11]);
        assert_eq!(
            read_body(&mut big, &cancel, 10),
            Err(BodyError::TooLarge(10))
        );
        cancel.cancel();
        let mut any = std::io::Cursor::new(vec![1u8; 1]);
        assert_eq!(read_body(&mut any, &cancel, 10), Err(BodyError::Cancelled));
    }

    #[test]
    fn media_type_drops_parameters() {
        let r = HttpResponse::new(
            200,
            vec![(
                "Content-Type".into(),
                "Text/Event-Stream; charset=utf-8".into(),
            )],
            Vec::new(),
        );
        assert_eq!(r.media_type().as_deref(), Some("text/event-stream"));
    }
}
