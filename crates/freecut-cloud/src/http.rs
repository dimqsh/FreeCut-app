//! The real HTTPS transport (feature `http`): `ureq` with rustls, the
//! platform's certificate roots, and, when the caller asks for it, the
//! `HTTPS_PROXY` from the environment (architecture §4.5; see
//! [`TransportConfig::proxy_from_env`]).
//!
//! `ureq` has phase timeouts but no way to wake a blocked read, so each call
//! runs on a short-lived helper thread (§4.4). The caller waits on channels
//! in 250 ms slices and checks its [`CancelToken`] between them; on cancel it
//! stops waiting and the helper ends on its own timeouts. Body bytes travel
//! over a bounded channel, so a streamed answer is also abandoned between
//! chunks.
//!
//! Two more rules hold for paid calls and cost checks (review M3):
//! - a paid request goes out on a fresh connection whose connector chain
//!   ends in a cancel gate: once the connection (TLS included) is up, the
//!   gate checks the token, so a cancel that fires before the first request
//!   byte stops the request there and it is reported as *not sent*;
//! - for a request with a [`LateSink`](crate::transport::LateSink), the
//!   helper keeps a copy of the answer; if the caller's token fired before
//!   it had the whole answer, the helper reads it to its end and hands it
//!   to the sink, so job ids that arrive late are not lost. (If the token
//!   fires just as a complete answer was read, the sink may see that answer
//!   too: consumers record job ids idempotently.)

use std::io::{self, Read};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::thread;
use std::time::Duration;

use freecut_media::cancel::CancelToken;
use ureq::config::Config;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
use ureq::unversioned::resolver::DefaultResolver;
use ureq::unversioned::transport::{ConnectionDetails, Connector, DefaultConnector, Transport};
use ureq::{Agent, Proxy, ProxyProtocol};

use crate::secret::Secret;
use crate::transport::{
    HttpRequest, HttpResponse, HttpTransport, LateResponse, Method, SendState, TransportError,
    TransportErrorKind,
};

const WAIT_SLICE: Duration = Duration::from_millis(250);
const CHUNK_BYTES: usize = 16 * 1024;
const CHUNKS_IN_FLIGHT: usize = 8;
/// The most of a late answer kept for the sink.
const MAX_LATE_BYTES: usize = 16 * 1024 * 1024;

/// Transport settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportConfig {
    /// Refuse plain `http://` URLs. Always on in the app; off only for
    /// loopback tests.
    pub https_only: bool,
    /// The `User-Agent` header.
    pub user_agent: String,
    /// Use the proxy in `HTTPS_PROXY` (or `https_proxy`). On in the app;
    /// off in tests, so a runner's proxy variables cannot reroute loopback
    /// calls. Only that variable is read: every call is HTTPS, so
    /// `ALL_PROXY` and `HTTP_PROXY` are ignored. Loopback hosts and the
    /// hosts in `NO_PROXY` (or `no_proxy`) always connect directly. Only
    /// `http://` and `https://` (CONNECT) proxies are used; `ureq` is built
    /// without SOCKS, so another scheme, or a value that does not parse,
    /// means a direct connection.
    pub proxy_from_env: bool,
}

impl Default for TransportConfig {
    fn default() -> TransportConfig {
        TransportConfig {
            https_only: true,
            user_agent: format!("FreeCut/{}", env!("CARGO_PKG_VERSION")),
            proxy_from_env: true,
        }
    }
}

/// Hosts that never go through a proxy, as `ureq` sees them (an IPv6
/// literal keeps its brackets).
const LOOPBACK: [&str; 3] = ["localhost", "127.0.0.1", "[::1]"];

/// The proxy for `config`, reading variables through `var` (review M12; the
/// rules are on [`TransportConfig::proxy_from_env`]).
fn proxy_for(config: &TransportConfig, var: impl Fn(&str) -> Option<String>) -> Option<Proxy> {
    if !config.proxy_from_env {
        return None;
    }
    let value = var("HTTPS_PROXY").or_else(|| var("https_proxy"))?;
    let parsed = Proxy::new(value.trim()).ok()?;
    if !matches!(
        parsed.protocol(),
        ProxyProtocol::Http | ProxyProtocol::Https
    ) {
        return None;
    }
    // Rebuilt, because `Proxy::new` takes no bypass list.
    let mut builder = Proxy::builder(parsed.protocol())
        .host(parsed.host())
        .port(parsed.port())
        .resolve_target(parsed.resolve_target());
    if let Some(user) = parsed.username() {
        builder = builder.username(user);
        if let Some(password) = parsed.password() {
            builder = builder.password(password);
        }
    }
    let no_proxy = var("NO_PROXY")
        .or_else(|| var("no_proxy"))
        .unwrap_or_default();
    let bypass = LOOPBACK
        .into_iter()
        .chain(no_proxy.split(',').map(str::trim))
        .filter(|host| !host.is_empty());
    for host in bypass {
        builder = builder.no_proxy(host);
    }
    builder.build().ok()
}

/// Blocking HTTPS over `ureq`.
#[derive(Debug, Clone)]
pub struct UreqTransport {
    pooled: Agent,
    /// Settings for paid requests, which get an agent of their own per
    /// request (fresh connection, cancel gate).
    fresh: Config,
}

fn agent_config(config: &TransportConfig, idle: usize) -> Config {
    Agent::config_builder()
        .http_status_as_error(false)
        .https_only(config.https_only)
        .max_redirects(0)
        // A redirect comes back as its own 3xx answer, not an error.
        .max_redirects_will_error(false)
        .user_agent(config.user_agent.as_str())
        .proxy(proxy_for(config, |name| std::env::var(name).ok()))
        .tls_config(
            TlsConfig::builder()
                .provider(TlsProvider::Rustls)
                // Passed explicitly: ureq is built without a provider of its
                // own (no bundled roots).
                .unversioned_rustls_crypto_provider(Arc::new(
                    rustls::crypto::ring::default_provider(),
                ))
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .max_idle_connections(idle)
        .build()
}

impl UreqTransport {
    /// A transport with `config`.
    pub fn new(config: &TransportConfig) -> UreqTransport {
        UreqTransport {
            pooled: agent_config(config, 4).new_agent(),
            // Paid submissions: no pooled socket that may be stale (§4.5).
            fresh: agent_config(config, 0),
        }
    }

    /// An agent for one paid request: a new connection, and a cancel gate
    /// after the connector chain.
    fn gated_agent(&self, cancel: &CancelToken) -> Agent {
        Agent::with_parts(
            self.fresh.clone(),
            DefaultConnector::new().chain(CancelGate(cancel.clone())),
            DefaultResolver::default(),
        )
    }
}

/// Stops a request whose token fired while its connection was being made:
/// it runs after TCP and TLS, right before ureq writes the request.
#[derive(Debug)]
struct CancelGate(CancelToken);

/// The cancel gate refused to let a request be written.
#[derive(Debug)]
struct CancelledBeforeSend;

impl std::fmt::Display for CancelledBeforeSend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cancelled before the request was written")
    }
}

impl std::error::Error for CancelledBeforeSend {}

impl Connector<Box<dyn Transport>> for CancelGate {
    type Out = Box<dyn Transport>;

    fn connect(
        &self,
        _details: &ConnectionDetails<'_>,
        chained: Option<Box<dyn Transport>>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        if self.0.is_cancelled() {
            return Err(ureq::Error::Other(Box::new(CancelledBeforeSend)));
        }
        Ok(chained)
    }
}

fn is_gate_refusal(err: &ureq::Error) -> bool {
    matches!(err, ureq::Error::Other(e) if e.downcast_ref::<CancelledBeforeSend>().is_some())
}

/// Whether a ureq error can only have happened before the first request
/// byte (§4.5). `ureq::Error` is `#[non_exhaustive]`, so variants added by a
/// later version fall into the conservative `MaybeSent` arm; the version is
/// pinned exactly and the table below is re-checked when it changes.
///
/// The `io::ErrorKind`s counted as not sent are the ones `connect(2)`
/// reports; once a connection is up, a broken one shows as a reset, a broken
/// pipe or a timeout instead, which count as maybe sent (review F7).
pub fn send_state(err: &ureq::Error) -> SendState {
    use ureq::Error;
    match err {
        Error::HostNotFound | Error::ConnectionFailed => SendState::NotSent,
        Error::Timeout(ureq::Timeout::Resolve | ureq::Timeout::Connect) => SendState::NotSent,
        Error::Rustls(
            rustls::Error::InvalidCertificate(_) | rustls::Error::NoCertificatesPresented,
        ) => SendState::NotSent,
        Error::Io(e)
            if matches!(
                e.kind(),
                io::ErrorKind::ConnectionRefused
                    | io::ErrorKind::NetworkUnreachable
                    | io::ErrorKind::HostUnreachable
                    | io::ErrorKind::AddrNotAvailable
            ) =>
        {
            SendState::NotSent
        }
        e if is_gate_refusal(e) => SendState::NotSent,
        _ => SendState::MaybeSent,
    }
}

fn kind_of(err: &ureq::Error) -> TransportErrorKind {
    use ureq::Error;
    match err {
        e if is_gate_refusal(e) => TransportErrorKind::Cancelled,
        Error::HostNotFound => TransportErrorKind::HostNotFound,
        Error::ConnectionFailed | Error::ConnectProxyFailed(_) => TransportErrorKind::Connect,
        Error::Timeout(_) => TransportErrorKind::Timeout,
        Error::Rustls(_) | Error::Tls(_) | Error::Pem(_) => TransportErrorKind::Tls,
        Error::Io(_) => TransportErrorKind::Io,
        _ => TransportErrorKind::Other,
    }
}

fn to_transport_error(err: &ureq::Error) -> TransportError {
    TransportError {
        kind: kind_of(err),
        send: send_state(err),
        detail: err.to_string(),
    }
}

type Head = Result<(u16, Vec<(String, String)>), ureq::Error>;

/// What the helper thread shares with the caller's side.
struct Shared {
    /// Set when the caller stops waiting because its token fired; only then
    /// is the rest of the answer a late answer.
    abandoned: AtomicBool,
}

fn run(
    agent: &Agent,
    request: HttpRequest,
    cancel: &CancelToken,
    shared: &Shared,
    head_tx: &SyncSender<Head>,
    body_tx: &SyncSender<io::Result<Vec<u8>>>,
) {
    let HttpRequest {
        method,
        url,
        headers,
        bearer,
        mut body,
        limits,
        late,
    } = request;
    // Cheap first check; the gate covers the time spent connecting.
    if cancel.is_cancelled() {
        let _ = head_tx.send(Err(ureq::Error::Other(Box::new(CancelledBeforeSend))));
        return;
    }
    let mut builder = ureq::http::Request::builder()
        .method(match method {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Delete => "DELETE",
        })
        .uri(url.as_str());
    for (name, value) in &headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    if let Some(token) = &bearer {
        // Cleared on drop; the copy inside the request is ureq's (best
        // effort, §5.6).
        let value = Secret::new(format!("Bearer {}", token.expose()));
        builder = builder.header("authorization", value.expose());
    }
    drop(bearer);
    let result = match builder.body(&body[..]) {
        Ok(request) => {
            let request = agent
                .configure_request(request)
                .timeout_connect(Some(limits.connect))
                .timeout_global(Some(limits.total))
                .build();
            agent.run(request)
        }
        Err(e) => Err(ureq::Error::from(e)),
    };
    // Form bodies hold codes and refresh tokens: clear them once sent.
    body.fill(0);
    let response = match result {
        Ok(r) => r,
        Err(e) => {
            let _ = head_tx.send(Err(e));
            return;
        }
    };
    let status = response.status().as_u16();
    let headers: Vec<(String, String)> = response
        .headers()
        .iter()
        .filter_map(|(k, v)| {
            v.to_str()
                .ok()
                .map(|v| (k.as_str().to_ascii_lowercase(), v.to_owned()))
        })
        .collect();
    let mut caller_gone = head_tx.send(Ok((status, headers.clone()))).is_err();
    // With a late sink, keep a copy of the body until the caller has read it
    // all: if it stops waiting, the whole answer goes to the sink.
    let mut kept: Option<Vec<u8>> = late.as_ref().map(|_| Vec::new());
    let mut reader = response.into_body().into_reader();
    let mut buf = vec![0u8; CHUNK_BYTES];
    let complete = loop {
        match reader.read(&mut buf) {
            Ok(0) => break true,
            Ok(n) => {
                if let Some(k) = kept.as_mut()
                    && k.len() + n <= MAX_LATE_BYTES
                {
                    k.extend_from_slice(&buf[..n]);
                }
                if !caller_gone && body_tx.send(Ok(buf[..n].to_vec())).is_err() {
                    caller_gone = true;
                }
                if caller_gone && kept.is_none() {
                    return;
                }
            }
            Err(e) => {
                if !caller_gone {
                    let _ = body_tx.send(Err(e));
                }
                break false;
            }
        }
    };
    let abandoned = shared.abandoned.load(Ordering::Acquire) || cancel.is_cancelled();
    if let (Some(sink), Some(body)) = (late, kept)
        && abandoned
    {
        tracing::debug!(
            status,
            bytes = body.len(),
            complete,
            "an answer arrived after a cancel"
        );
        sink(LateResponse {
            status,
            headers,
            body,
            complete,
        });
    }
}

/// A body read from the helper thread, abandoned when the token fires.
struct ChannelReader {
    chunks: Receiver<io::Result<Vec<u8>>>,
    pending: Vec<u8>,
    pos: usize,
    cancel: CancelToken,
    shared: Arc<Shared>,
}

impl Read for ChannelReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.pos < self.pending.len() {
                let n = buf.len().min(self.pending.len() - self.pos);
                buf[..n].copy_from_slice(&self.pending[self.pos..self.pos + n]);
                self.pos += n;
                return Ok(n);
            }
            if self.cancel.is_cancelled() {
                self.shared.abandoned.store(true, Ordering::Release);
                return Err(io::Error::other("cancelled"));
            }
            match self.chunks.recv_timeout(WAIT_SLICE) {
                Ok(Ok(chunk)) => {
                    self.pending = chunk;
                    self.pos = 0;
                }
                Ok(Err(e)) => return Err(e),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Ok(0),
            }
        }
    }
}

impl HttpTransport for UreqTransport {
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
        let agent = if request.limits.fresh_connection {
            self.gated_agent(cancel)
        } else {
            self.pooled.clone()
        };
        let shared = Arc::new(Shared {
            abandoned: AtomicBool::new(false),
        });
        let (head_tx, head_rx) = mpsc::sync_channel::<Head>(1);
        let (body_tx, body_rx) = mpsc::sync_channel(CHUNKS_IN_FLIGHT);
        let helper_cancel = cancel.clone();
        let helper_shared = Arc::clone(&shared);
        thread::Builder::new()
            .name("freecut-cloud-http".into())
            .spawn(move || {
                run(
                    &agent,
                    request,
                    &helper_cancel,
                    &helper_shared,
                    &head_tx,
                    &body_tx,
                );
            })
            .map_err(|e| {
                TransportError::not_sent(
                    TransportErrorKind::Other,
                    format!("no helper thread: {e}"),
                )
            })?;
        loop {
            if cancel.is_cancelled() {
                // The helper may already have written the request; if an
                // answer still comes, it goes to the late sink.
                shared.abandoned.store(true, Ordering::Release);
                return Err(TransportError::maybe_sent(
                    TransportErrorKind::Cancelled,
                    "cancelled while waiting",
                ));
            }
            match head_rx.recv_timeout(WAIT_SLICE) {
                Ok(Ok((status, headers))) => {
                    return Ok(HttpResponse {
                        status,
                        headers,
                        body: Box::new(ChannelReader {
                            chunks: body_rx,
                            pending: Vec::new(),
                            pos: 0,
                            cancel: cancel.clone(),
                            shared,
                        }),
                    });
                }
                Ok(Err(e)) => return Err(to_transport_error(&e)),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(TransportError::maybe_sent(
                        TransportErrorKind::Other,
                        "the request thread ended",
                    ));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// HTTPS without the runner's proxy variables.
    fn no_proxy() -> TransportConfig {
        TransportConfig {
            proxy_from_env: false,
            ..TransportConfig::default()
        }
    }

    /// Plain HTTP to a loopback server, without the runner's proxy variables.
    fn loopback() -> TransportConfig {
        TransportConfig {
            https_only: false,
            ..no_proxy()
        }
    }

    fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        }
    }

    /// M12: only `HTTPS_PROXY`, CONNECT proxies only, and loopback and
    /// `NO_PROXY` hosts go direct.
    #[test]
    fn the_proxy_comes_from_https_proxy_only_and_skips_loopback() {
        let on = TransportConfig::default();
        assert!(on.proxy_from_env);
        let other_vars = [
            ("ALL_PROXY", "http://all.corp:1"),
            ("all_proxy", "http://all.corp:1"),
            ("HTTP_PROXY", "http://plain.corp:2"),
            ("http_proxy", "http://plain.corp:2"),
        ];
        assert!(proxy_for(&on, vars(&other_vars)).is_none());

        let mut with_https = other_vars.to_vec();
        with_https.push(("HTTPS_PROXY", "http://proxy.corp:3128"));
        let p = proxy_for(&on, vars(&with_https)).unwrap();
        assert_eq!((p.host(), p.port()), ("proxy.corp", 3128));

        let p = proxy_for(
            &on,
            vars(&[
                ("https_proxy", "proxy.corp:8080"),
                ("no_proxy", " corp.example , .internal"),
            ]),
        )
        .unwrap();
        assert_eq!((p.protocol(), p.port()), (ProxyProtocol::Http, 8080));
        let uri = |s: &str| s.parse::<ureq::http::Uri>().unwrap();
        for direct in [
            "https://127.0.0.1:9/mcp",
            "https://localhost/",
            "https://[::1]:9/",
            "https://corp.example/",
            "https://api.internal/",
        ] {
            assert!(p.is_no_proxy(&uri(direct)), "{direct}");
        }
        assert!(!p.is_no_proxy(&uri("https://mcp.higgsfield.ai/mcp")));

        // No SOCKS (not built into ureq), nothing empty or unparseable.
        for bad in ["socks5://proxy.corp:1080", "", "   ", "http://"] {
            assert!(
                proxy_for(&on, vars(&[("HTTPS_PROXY", bad)])).is_none(),
                "{bad:?}"
            );
        }

        // Off: the variables are not even read.
        assert!(proxy_for(&no_proxy(), |name| panic!("read {name}")).is_none());
        assert!(agent_config(&no_proxy(), 0).proxy().is_none());
    }

    #[test]
    fn proxy_credentials_are_kept_but_not_printed() {
        let p = proxy_for(
            &TransportConfig::default(),
            vars(&[("HTTPS_PROXY", "http://alice:s3cr3t-pw@proxy.corp:3128")]),
        )
        .unwrap();
        assert_eq!(
            (p.username(), p.password()),
            (Some("alice"), Some("s3cr3t-pw"))
        );
        let shown = format!("{p:?}");
        assert!(
            !shown.contains("s3cr3t-pw") && !shown.contains("alice"),
            "{shown}"
        );
    }

    /// The send-state table of §4.5, pinned against ureq 3.4.2's variants:
    /// every variant built with FreeCut's features (no cookies, charset,
    /// compression, json, multipart or native-tls) and every `Timeout`.
    /// `Error::Protocol` holds a ureq-proto type, so the next test makes one
    /// with a loopback server.
    #[test]
    fn send_state_table() {
        use ureq::Error;
        let io = |k| Error::Io(io::Error::from(k));
        let not_sent = [
            Error::HostNotFound,
            Error::ConnectionFailed,
            Error::Timeout(ureq::Timeout::Resolve),
            Error::Timeout(ureq::Timeout::Connect),
            Error::Rustls(rustls::Error::NoCertificatesPresented),
            Error::Rustls(rustls::Error::InvalidCertificate(
                rustls::CertificateError::Expired,
            )),
            io(io::ErrorKind::ConnectionRefused),
            io(io::ErrorKind::NetworkUnreachable),
            io(io::ErrorKind::HostUnreachable),
            io(io::ErrorKind::AddrNotAvailable),
        ];
        for e in &not_sent {
            assert_eq!(send_state(e), SendState::NotSent, "{e:?}");
        }
        let maybe_sent = [
            Error::StatusCode(500),
            // Raised while building a request, but kept on the safe side.
            Error::Http(ureq::http::Error::from(
                ureq::http::StatusCode::from_u16(1000).unwrap_err(),
            )),
            Error::BadUri("x".into()),
            Error::Timeout(ureq::Timeout::Global),
            Error::Timeout(ureq::Timeout::PerCall),
            Error::Timeout(ureq::Timeout::SendRequest),
            Error::Timeout(ureq::Timeout::Await100),
            Error::Timeout(ureq::Timeout::SendBody),
            Error::Timeout(ureq::Timeout::RecvResponse),
            Error::Timeout(ureq::Timeout::RecvBody),
            Error::RedirectFailed,
            Error::InvalidProxyUrl,
            Error::BodyExceedsLimit(1),
            Error::TooManyRedirects,
            Error::Tls("x"),
            Error::Rustls(rustls::Error::DecryptError),
            Error::Pem(rustls::pki_types::pem::Error::NoItemsFound),
            Error::RequireHttpsOnly("x".into()),
            Error::LargeResponseHeader(1, 1),
            Error::ConnectProxyFailed("x".into()),
            Error::TlsRequired,
            Error::Other(Box::new(io::Error::other("x"))),
            Error::BodyStalled,
            io(io::ErrorKind::ConnectionReset),
            io(io::ErrorKind::BrokenPipe),
            io(io::ErrorKind::TimedOut),
            io(io::ErrorKind::UnexpectedEof),
        ];
        for e in &maybe_sent {
            assert_eq!(send_state(e), SendState::MaybeSent, "{e:?}");
        }
    }

    /// Building the TLS configuration happens on the first HTTPS request;
    /// without a provider ureq would panic there. A handshake against a
    /// loopback port that speaks no TLS must fail as an error instead.
    #[test]
    fn https_uses_the_ring_provider_without_panicking() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = listener.accept() {
                use std::io::Write;
                let _ = s.write_all(b"HTTP/1.1 200 OK\r\n\r\nnot tls");
            }
        });
        let t = UreqTransport::new(&no_proxy());
        let err = t
            .send(
                HttpRequest::get(
                    format!("https://127.0.0.1:{port}/"),
                    crate::transport::CallLimits::READ,
                ),
                &CancelToken::new(),
            )
            .unwrap_err();
        assert!(
            matches!(err.kind, TransportErrorKind::Tls | TransportErrorKind::Io),
            "{err:?}"
        );
    }

    /// A malformed answer is `Error::Protocol`: the request went out.
    #[test]
    fn a_malformed_answer_is_a_protocol_error_that_may_have_been_sent() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = listener.accept() {
                use std::io::{Read as _, Write as _};
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf);
                let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: many\r\n\r\n");
            }
        });
        let err = agent_config(&loopback(), 0)
            .new_agent()
            .get(format!("http://127.0.0.1:{port}/mcp"))
            .call()
            .unwrap_err();
        assert!(matches!(err, ureq::Error::Protocol(_)), "{err:?}");
        assert_eq!(send_state(&err), SendState::MaybeSent);
    }

    /// M3: the gate runs after the connection is up and stops the request
    /// before its first byte; the server sees a connection and no data.
    #[test]
    fn the_cancel_gate_stops_a_paid_request_before_it_is_written() {
        use std::io::Read as _;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = listener.accept() {
                let _ = s.set_read_timeout(Some(Duration::from_secs(3)));
                let mut buf = Vec::new();
                let _ = s.read_to_end(&mut buf);
                let _ = tx.send(buf.len());
            }
        });
        let t = UreqTransport::new(&loopback());
        let cancel = CancelToken::new();
        cancel.cancel();
        let err = t
            .gated_agent(&cancel)
            .get(format!("http://127.0.0.1:{port}/mcp"))
            .call()
            .unwrap_err();
        assert!(is_gate_refusal(&err), "{err:?}");
        assert_eq!(send_state(&err), SendState::NotSent);
        assert_eq!(kind_of(&err), TransportErrorKind::Cancelled);
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), 0);
    }

    #[test]
    fn cancelling_before_sending_sends_nothing() {
        let t = UreqTransport::new(&no_proxy());
        let cancel = CancelToken::new();
        cancel.cancel();
        let err = t
            .send(
                HttpRequest::get(
                    "https://mcp.higgsfield.ai/mcp",
                    crate::transport::CallLimits::READ,
                ),
                &cancel,
            )
            .unwrap_err();
        assert_eq!(err.send, SendState::NotSent);
    }
}
