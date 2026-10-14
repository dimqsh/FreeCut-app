//! The real transport (`ureq`) against a scripted HTTP/1.1 server on
//! `127.0.0.1` (architecture §13, feature `http`): JSON and SSE answers,
//! a refused connection (`NotSent`), a connection dropped after the request
//! arrived (`MaybeSent` → outcome unknown, exactly one request), and a
//! stalled answer abandoned on cancel.
#![cfg(feature = "http")]
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use common::NoWait;
use freecut_cloud::CancelToken;
use freecut_cloud::error::{CloudError, SubmitError, submit_disposition};
use freecut_cloud::http::{TransportConfig, UreqTransport};
use freecut_cloud::mcp::{CallClass, McpClient, StaticToken};
use freecut_cloud::secret::Secret;
use freecut_cloud::transport::{CallLimits, SendState};
use serde_json::{Value, json};

/// What the scripted server does with one request.
enum Reply {
    /// Status, extra headers, body; then close.
    Send(u16, Vec<(&'static str, String)>, Vec<u8>),
    /// SSE events written one at a time with pauses.
    Stream(Vec<String>),
    /// Close without answering.
    Drop,
    /// Wait this long, then close.
    Stall(Duration),
}

struct Server {
    url: String,
    requests: Arc<Mutex<Vec<(String, Value)>>>,
    count: Arc<AtomicUsize>,
}

fn read_request(stream: &mut TcpStream) -> Option<(String, Value)> {
    let mut reader = BufReader::new(stream);
    let mut head = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        if line == "\r\n" {
            break;
        }
        head.push(line);
    }
    let length = head
        .iter()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.eq_ignore_ascii_case("content-length")
                .then(|| v.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).ok()?;
    let headers = head.join("");
    Some((
        headers,
        serde_json::from_slice(&body).unwrap_or(Value::Null),
    ))
}

fn serve(script: impl Fn(&Value) -> Reply + Send + 'static) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let count = Arc::new(AtomicUsize::new(0));
    let (log, n) = (requests.clone(), count.clone());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let Some((headers, body)) = read_request(&mut stream) else {
                continue;
            };
            n.fetch_add(1, Ordering::SeqCst);
            log.lock().unwrap().push((headers, body.clone()));
            match script(&body) {
                Reply::Send(status, extra, payload) => {
                    let mut out = format!(
                        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n",
                        payload.len()
                    );
                    for (k, v) in extra {
                        out.push_str(&format!("{k}: {v}\r\n"));
                    }
                    out.push_str("\r\n");
                    let _ = stream.write_all(out.as_bytes());
                    let _ = stream.write_all(&payload);
                }
                Reply::Stream(events) => {
                    let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n");
                    for e in events {
                        let _ = stream.write_all(e.as_bytes());
                        let _ = stream.flush();
                        thread::sleep(Duration::from_millis(20));
                    }
                }
                Reply::Drop => {}
                Reply::Stall(d) => thread::sleep(d),
            }
        }
    });
    Server {
        url,
        requests,
        count,
    }
}

fn json_reply(id: &Value, result: Value, extra: Vec<(&'static str, String)>) -> Reply {
    let mut headers = vec![("Content-Type", "application/json".to_owned())];
    headers.extend(extra);
    Reply::Send(
        200,
        headers,
        json!({"jsonrpc": "2.0", "id": id, "result": result})
            .to_string()
            .into_bytes(),
    )
}

/// Answers the handshake; `call` answers tools/call.
fn mcp_script(
    call: impl Fn(&Value) -> Reply + Send + 'static,
) -> impl Fn(&Value) -> Reply + Send + 'static {
    move |body: &Value| match body.get("method").and_then(Value::as_str) {
        Some("initialize") => json_reply(
            &body["id"],
            json!({"protocolVersion": "2025-11-25", "capabilities": {"tools": {}}, "serverInfo": {"name": "loop", "version": "1"}}),
            vec![("Mcp-Session-Id", "loop-1".into())],
        ),
        Some("notifications/initialized") => Reply::Send(202, Vec::new(), Vec::new()),
        Some("tools/call") => call(body),
        _ => Reply::Send(400, Vec::new(), Vec::new()),
    }
}

fn client(url: &str) -> McpClient {
    let transport = UreqTransport::new(&TransportConfig {
        https_only: false,
        ..TransportConfig::default()
    });
    McpClient::new(
        url,
        Arc::new(transport),
        Arc::new(StaticToken(Secret::new("t1"))),
        Arc::new(NoWait::default()),
        "0.1.0",
    )
}

fn paid(client: &McpClient, cancel: &CancelToken) -> Result<Value, freecut_cloud::mcp::McpError> {
    client
        .call_tool(
            "generate_image",
            json!({"params": {"model": "m", "prompt": "x"}}),
            CallClass::Paid,
            CallLimits {
                connect: Duration::from_secs(2),
                total: Duration::from_secs(5),
                fresh_connection: true,
            },
            cancel,
        )
        .map(|o| o.value().unwrap_or(Value::Null))
}

#[test]
fn json_answers_over_real_http() {
    let server = serve(mcp_script(|body| {
        json_reply(
            &body["id"],
            json!({"content": [{"type": "text", "text": "{\"cost\":{\"credits\":2}}"}]}),
            Vec::new(),
        )
    }));
    let c = client(&server.url);
    let out = c
        .call_tool(
            "generate_image",
            json!({"params": {"model": "m", "get_cost": true}}),
            CallClass::Preflight,
            CallLimits::PREFLIGHT,
            &CancelToken::new(),
        )
        .unwrap();
    assert_eq!(out.value(), Some(json!({"cost": {"credits": 2}})));
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    let (headers, _) = &requests[2];
    let lower = headers.to_ascii_lowercase();
    assert!(lower.contains("authorization: bearer t1"), "{headers}");
    assert!(lower.contains("mcp-session-id: loop-1"), "{headers}");
    assert!(
        lower.contains("mcp-protocol-version: 2025-11-25"),
        "{headers}"
    );
    assert!(
        lower.contains("accept: application/json, text/event-stream"),
        "{headers}"
    );
}

#[test]
fn sse_answers_arrive_in_pieces() {
    let server = serve(mcp_script(|body| {
        let response = json!({"jsonrpc": "2.0", "id": body["id"], "result": {"content": [{"type": "text", "text": "{\"ok\":1}"}]}}).to_string();
        let (a, b) = response.split_at(response.len() / 2);
        Reply::Stream(vec![
            ": keep-alive\n\n".into(),
            "event: message\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{}}\n\n".into(),
            format!("event: message\ndata: {a}"),
            format!("{b}\n\n"),
        ])
    }));
    let c = client(&server.url);
    let out = c
        .call_tool(
            "models_explore",
            json!({"action": "list"}),
            CallClass::Free,
            CallLimits::READ,
            &CancelToken::new(),
        )
        .unwrap();
    assert_eq!(out.value(), Some(json!({"ok": 1})));
}

#[test]
fn a_refused_connection_is_not_sent() {
    let port = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let c = client(&format!("http://127.0.0.1:{port}/mcp"));
    let err = paid(&c, &CancelToken::new()).unwrap_err();
    assert_eq!(err.send_state(), SendState::NotSent, "{err:?}");
    assert!(
        matches!(
            submit_disposition(&err),
            SubmitError::NotSent(CloudError::Offline)
        ),
        "{err:?}"
    );
}

#[test]
fn a_connection_dropped_after_the_request_is_an_unknown_outcome_sent_once() {
    let server = serve(mcp_script(|_| Reply::Drop));
    let c = client(&server.url);
    let err = paid(&c, &CancelToken::new()).unwrap_err();
    assert!(
        matches!(submit_disposition(&err), SubmitError::OutcomeUnknown(_)),
        "{err:?}"
    );
    // initialize, initialized, and exactly one paid request.
    assert_eq!(server.count.load(Ordering::SeqCst), 3);
}

#[test]
fn a_stalled_answer_is_abandoned_on_cancel() {
    let server = serve(mcp_script(|_| Reply::Stall(Duration::from_secs(4))));
    let c = client(&server.url);
    let cancel = CancelToken::new();
    let trigger = cancel.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(500));
        trigger.cancel();
    });
    let started = Instant::now();
    let err = paid(&c, &cancel).unwrap_err();
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(err.send_state(), SendState::MaybeSent);
    assert!(
        matches!(submit_disposition(&err), SubmitError::OutcomeUnknown(_)),
        "{err:?}"
    );
}
