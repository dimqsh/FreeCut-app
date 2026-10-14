//! The real HTTPS transport (feature `http`): `ureq` with rustls, the
//! platform's certificate roots, and `HTTPS_PROXY` from the environment
//! (architecture §4.5).
//!
//! `ureq` has phase timeouts but no way to wake a blocked read, so each call
//! runs on a short-lived helper thread (§4.4). The caller waits on channels
//! in 250 ms slices and checks its [`CancelToken`] between them; on cancel it
//! stops waiting and the helper ends on its own timeouts. Body bytes travel
//! over a bounded channel, so a streamed answer is also abandoned between
//! chunks.

use std::io::{self, Read};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::thread;
use std::time::Duration;

use freecut_media::cancel::CancelToken;
use ureq::Agent;
use ureq::tls::{RootCerts, TlsConfig};

use crate::transport::{
    HttpRequest, HttpResponse, HttpTransport, Method, SendState, TransportError, TransportErrorKind,
};

const WAIT_SLICE: Duration = Duration::from_millis(250);
const CHUNK_BYTES: usize = 16 * 1024;
const CHUNKS_IN_FLIGHT: usize = 8;

/// Transport settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportConfig {
    /// Refuse plain `http://` URLs. Always on in the app; off only for
    /// loopback tests.
    pub https_only: bool,
    /// The `User-Agent` header.
    pub user_agent: String,
}

impl Default for TransportConfig {
    fn default() -> TransportConfig {
        TransportConfig {
            https_only: true,
            user_agent: format!("FreeCut/{}", env!("CARGO_PKG_VERSION")),
        }
    }
}

/// Blocking HTTPS over `ureq`.
#[derive(Debug, Clone)]
pub struct UreqTransport {
    pooled: Agent,
    fresh: Agent,
}

impl UreqTransport {
    /// A transport with `config`.
    pub fn new(config: &TransportConfig) -> UreqTransport {
        let agent = |idle: usize| {
            Agent::config_builder()
                .http_status_as_error(false)
                .https_only(config.https_only)
                .max_redirects(0)
                // A redirect comes back as its own 3xx answer, not an error.
                .max_redirects_will_error(false)
                .user_agent(config.user_agent.as_str())
                .proxy(ureq::Proxy::try_from_env())
                .tls_config(
                    TlsConfig::builder()
                        .root_certs(RootCerts::PlatformVerifier)
                        .build(),
                )
                .max_idle_connections(idle)
                .build()
                .new_agent()
        };
        UreqTransport {
            pooled: agent(4),
            // Paid submissions: no pooled socket that may be stale (§4.5).
            fresh: agent(0),
        }
    }
}

/// Whether a ureq error can only have happened before the first request
/// byte (§4.5). `ureq::Error` is `#[non_exhaustive]`, so variants added by a
/// later version fall into the conservative `MaybeSent` arm; the version is
/// pinned exactly and the table below is re-checked when it changes.
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
        _ => SendState::MaybeSent,
    }
}

fn kind_of(err: &ureq::Error) -> TransportErrorKind {
    use ureq::Error;
    match err {
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

fn run(
    agent: &Agent,
    request: HttpRequest,
    head_tx: &SyncSender<Head>,
    body_tx: &SyncSender<io::Result<Vec<u8>>>,
) {
    let HttpRequest {
        method,
        url,
        headers,
        bearer,
        body,
        limits,
    } = request;
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
        builder = builder.header("authorization", format!("Bearer {}", token.expose()));
    }
    drop(bearer);
    let request = match builder.body(body) {
        Ok(r) => r,
        Err(e) => {
            let _ = head_tx.send(Err(ureq::Error::from(e)));
            return;
        }
    };
    let request = agent
        .configure_request(request)
        .timeout_connect(Some(limits.connect))
        .timeout_global(Some(limits.total))
        .build();
    let response = match agent.run(request) {
        Ok(r) => r,
        Err(e) => {
            let _ = head_tx.send(Err(e));
            return;
        }
    };
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .filter_map(|(k, v)| {
            v.to_str()
                .ok()
                .map(|v| (k.as_str().to_ascii_lowercase(), v.to_owned()))
        })
        .collect();
    if head_tx.send(Ok((status, headers))).is_err() {
        return; // the caller stopped waiting
    }
    let mut reader = response.into_body().into_reader();
    let mut buf = vec![0u8; CHUNK_BYTES];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => {
                if body_tx.send(Ok(buf[..n].to_vec())).is_err() {
                    return;
                }
            }
            Err(e) => {
                let _ = body_tx.send(Err(e));
                return;
            }
        }
    }
}

/// A body read from the helper thread, abandoned when the token fires.
struct ChannelReader {
    chunks: Receiver<io::Result<Vec<u8>>>,
    pending: Vec<u8>,
    pos: usize,
    cancel: CancelToken,
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
            self.fresh.clone()
        } else {
            self.pooled.clone()
        };
        let (head_tx, head_rx) = mpsc::sync_channel::<Head>(1);
        let (body_tx, body_rx) = mpsc::sync_channel(CHUNKS_IN_FLIGHT);
        thread::Builder::new()
            .name("freecut-cloud-http".into())
            .spawn(move || run(&agent, request, &head_tx, &body_tx))
            .map_err(|e| {
                TransportError::not_sent(
                    TransportErrorKind::Other,
                    format!("no helper thread: {e}"),
                )
            })?;
        loop {
            if cancel.is_cancelled() {
                // The helper may already have written the request.
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

    /// The send-state table of §4.5, pinned against ureq 3.4.2's variants.
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
            Error::BadUri("x".into()),
            Error::Timeout(ureq::Timeout::Global),
            Error::Timeout(ureq::Timeout::PerCall),
            Error::Timeout(ureq::Timeout::SendRequest),
            Error::Timeout(ureq::Timeout::SendBody),
            Error::Timeout(ureq::Timeout::RecvResponse),
            Error::Timeout(ureq::Timeout::RecvBody),
            Error::RedirectFailed,
            Error::InvalidProxyUrl,
            Error::BodyExceedsLimit(1),
            Error::TooManyRedirects,
            Error::Tls("x"),
            Error::Rustls(rustls::Error::DecryptError),
            Error::RequireHttpsOnly("x".into()),
            Error::LargeResponseHeader(1, 1),
            Error::ConnectProxyFailed("x".into()),
            Error::TlsRequired,
            Error::Other(Box::new(io::Error::other("x"))),
            io(io::ErrorKind::ConnectionReset),
            io(io::ErrorKind::BrokenPipe),
            io(io::ErrorKind::TimedOut),
            io(io::ErrorKind::UnexpectedEof),
        ];
        for e in &maybe_sent {
            assert_eq!(send_state(e), SendState::MaybeSent, "{e:?}");
        }
    }

    #[test]
    fn cancelling_before_sending_sends_nothing() {
        let t = UreqTransport::new(&TransportConfig::default());
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
