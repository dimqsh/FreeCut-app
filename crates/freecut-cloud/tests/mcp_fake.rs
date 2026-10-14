//! The MCP client, provider and guard against the in-memory fake service:
//! handshake, SSE, session expiry, `401` and renewal, retries, the preflight
//! guard and the unknown-outcome rule. No sockets.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use std::sync::Arc;

use common::{
    SwapToken, fake_higgsfield, fixture, fixture_json, golden, image_op, mcp_client, provider,
    provider_on, provider_with_guard, run_tag, video_op,
};
use freecut_cloud::CancelToken;
use freecut_cloud::credits::Credits;
use freecut_cloud::error::{CloudError, NotAcceptedReason, SubmitError, submit_disposition};
use freecut_cloud::fake::{Fault, ResumeMode, StreamMode, ToolReply};
use freecut_cloud::guard::{PreflightGuard, QuoteRequest};
use freecut_cloud::mcp::{McpClient, McpError, StaticToken};
use freecut_cloud::provider::higgsfield::allow;
use freecut_cloud::provider::{GenerationProvider, Operation, QuoteOutcome, Unavailable};
use freecut_cloud::secret::Secret;
use freecut_cloud::transport::{
    CallLimits, HttpRequest, HttpResponse, HttpTransport, LateResponse, TransportError,
    TransportErrorKind, read_body,
};
use pretty_assertions::assert_eq;
use proptest::prelude::*;
use serde_json::{Value, json};

fn token(t: &str) -> Arc<StaticToken> {
    Arc::new(StaticToken(Secret::new(t)))
}

fn methods(fake: &freecut_cloud::fake::FakeService) -> Vec<String> {
    fake.requests()
        .iter()
        .map(|r| {
            r.tool().map_or_else(
                || r.rpc_method().unwrap_or("-").to_owned(),
                |t| format!("call:{t}"),
            )
        })
        .collect()
}

#[test]
fn constructing_a_provider_sends_nothing() {
    let fake = fake_higgsfield();
    let (_provider, _) = provider(&fake, token("t1"));
    assert!(fake.requests().is_empty());
}

#[test]
fn connect_runs_the_handshake_and_keeps_only_allowed_tools() {
    let fake = fake_higgsfield();
    let (p, _) = provider(&fake, token("t1"));
    let info = p.connect(&CancelToken::new()).unwrap();
    assert_eq!(info.protocol_version, "2025-11-25");
    assert_eq!(
        info.offered,
        vec![
            "generate_image",
            "generate_video",
            "jobs_wait",
            "models_explore"
        ]
    );
    assert!(info.missing.is_empty());
    // Pages of two: three tools/list calls for five tools.
    assert_eq!(
        methods(&fake),
        vec![
            "initialize",
            "notifications/initialized",
            "tools/list",
            "tools/list",
            "tools/list"
        ]
    );
    let reqs = fake.requests();
    assert_eq!(reqs[0].session, None);
    assert_eq!(reqs[0].protocol, None);
    assert_eq!(reqs[0].body["params"]["clientInfo"]["name"], "FreeCut");
    assert_eq!(reqs[0].body["params"]["capabilities"], json!({}));
    for r in &reqs[1..] {
        assert_eq!(r.session.as_deref(), Some("fake-session-1"));
        assert_eq!(r.protocol.as_deref(), Some("2025-11-25"));
        assert_eq!(r.bearer.as_deref(), Some("t1"));
    }
    // The server's instructions are kept for the debug view only.
    assert!(p.server_info().unwrap().instructions.is_some());
}

#[test]
fn missing_tools_are_reported() {
    let fake = fake_higgsfield();
    fake.set_tools(
        vec![json!({"name": "models_explore", "inputSchema": {}})],
        10,
    );
    let (p, _) = provider(&fake, token("t1"));
    let info = p.connect(&CancelToken::new()).unwrap();
    assert_eq!(info.missing, vec!["generate_image", "generate_video"]);
    assert_eq!(
        p.quote(&image_op(), &run_tag(), &CancelToken::new())
            .unwrap(),
        QuoteOutcome::NotAvailable(Unavailable::ToolMissing {
            tool: "generate_image".into()
        })
    );
    assert!(fake.calls_to("generate_image").is_empty());
}

#[test]
fn an_older_supported_revision_is_accepted_and_an_unknown_one_refused() {
    let fake = fake_higgsfield();
    fake.set_protocol("2025-06-18");
    let (p, _) = provider(&fake, token("t1"));
    assert_eq!(
        p.connect(&CancelToken::new()).unwrap().protocol_version,
        "2025-06-18"
    );
    fake.set_protocol("2024-11-05");
    let (p, _) = provider(&fake, token("t1"));
    assert_eq!(
        p.connect(&CancelToken::new()),
        Err(CloudError::UnsupportedServiceVersion {
            version: "2024-11-05".into()
        })
    );
}

#[test]
fn the_catalogue_loads_every_output_type() {
    let fake = fake_higgsfield();
    let (p, _) = provider(&fake, token("t1"));
    let c = p.catalogue(&CancelToken::new()).unwrap();
    assert_eq!(c.models().len(), 36);
    let kinds: Vec<Value> = fake
        .calls_to("models_explore")
        .iter()
        .map(|a| a["type"].clone())
        .collect();
    assert_eq!(kinds, vec![json!("image"), json!("video"), json!("audio")]);
    for args in fake.calls_to("models_explore") {
        assert_eq!(args["action"], "list");
        assert!(args.get("context").is_none());
    }
}

#[test]
fn catalogue_pages_are_followed() {
    let fake = fake_higgsfield();
    fake.on_tool("models_explore", |args| {
        let kind = args["type"].as_str().unwrap_or_default();
        if kind != "image" {
            return ToolReply::Json(json!({"items": [], "has_more": false}));
        }
        match args.get("after").and_then(Value::as_str) {
            None => ToolReply::Json(json!({"items": [{"id": "a", "output_type": "image"}], "has_more": true, "next_page_token": "p2"})),
            Some("p2") => ToolReply::Json(json!({"items": [{"id": "b", "output_type": "image"}], "has_more": false})),
            Some(other) => panic!("unexpected cursor {other}"),
        }
    });
    let (p, _) = provider(&fake, token("t1"));
    let c = p.catalogue(&CancelToken::new()).unwrap();
    assert_eq!(
        c.models().iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec!["a", "b"]
    );
}

#[test]
fn quotes_send_the_golden_arguments_and_read_the_recorded_answers() {
    let fake = fake_higgsfield();
    let (p, _) = provider(&fake, token("t1"));
    let cancel = CancelToken::new();
    let QuoteOutcome::Quote(q) = p.quote(&image_op(), &run_tag(), &cancel).unwrap() else {
        panic!("expected a quote");
    };
    assert_eq!(q.credits, Credits::from_millis(250));
    assert_eq!(q.arguments, golden("get_cost-image-gpt_image_2_5.json"));
    assert_eq!(
        fake.calls_to("generate_image"),
        vec![golden("get_cost-image-gpt_image_2_5.json")]
    );

    let QuoteOutcome::Quote(q) = p.quote(&video_op(), &run_tag(), &cancel).unwrap() else {
        panic!("expected a quote");
    };
    assert_eq!(q.credits, Credits::from_millis(35_000));
    assert_eq!(
        fake.calls_to("generate_video"),
        vec![golden("get_cost-video-seedance_2_5-t2v.json")]
    );
}

#[test]
fn quotes_work_over_sse_split_into_any_chunk_size() {
    for chunk in [1, 7, 64, 4096] {
        let fake = fake_higgsfield();
        fake.set_stream(StreamMode::Sse { chunk });
        fake.send_before_response(vec![
            json!({"jsonrpc": "2.0", "method": "notifications/progress", "params": {"progress": 1}}),
            json!({"jsonrpc": "2.0", "id": "srv-1", "method": "ping"}),
            json!({"jsonrpc": "2.0", "id": "srv-2", "method": "sampling/createMessage", "params": {}}),
        ]);
        let (p, _) = provider(&fake, token("t1"));
        let QuoteOutcome::Quote(q) = p
            .quote(&image_op(), &run_tag(), &CancelToken::new())
            .unwrap()
        else {
            panic!("expected a quote (chunk {chunk})");
        };
        assert_eq!(q.credits, Credits::from_millis(250));
        let replies: Vec<Value> = fake
            .requests()
            .into_iter()
            .filter(|r| r.body.get("method").is_none() && r.body.get("id").is_some())
            .map(|r| r.body)
            .collect();
        assert!(
            replies.contains(&json!({"jsonrpc": "2.0", "id": "srv-1", "result": {}})),
            "{replies:?}"
        );
        assert!(
            replies
                .iter()
                .any(|r| r["id"] == "srv-2" && r["error"]["code"] == -32601),
            "{replies:?}"
        );
    }
}

#[test]
fn quotes_also_read_structured_content() {
    let fake = fake_higgsfield();
    fake.on_tool("generate_image", |_| {
        ToolReply::Structured(json!({"cost": {"credits": 1.5}}))
    });
    let (p, _) = provider(&fake, token("t1"));
    let QuoteOutcome::Quote(q) = p
        .quote(&image_op(), &run_tag(), &CancelToken::new())
        .unwrap()
    else {
        panic!("expected a quote");
    };
    assert_eq!(q.credits, Credits::from_millis(1_500));
}

#[test]
fn more_variants_than_an_estimate_covers_are_not_quoted() {
    let fake = fake_higgsfield();
    let (p, _) = provider(&fake, token("t1"));
    let Operation::Image(mut r) = image_op() else {
        unreachable!()
    };
    r.variants = 2;
    assert_eq!(
        p.quote(&Operation::Image(r), &run_tag(), &CancelToken::new())
            .unwrap(),
        QuoteOutcome::NotAvailable(Unavailable::VariantsNotCovered { max: 1 })
    );
    assert!(fake.requests().is_empty());
}

#[test]
fn a_session_404_on_a_free_call_reopens_the_session_and_resends_once() {
    let fake = fake_higgsfield();
    let (p, _) = provider(&fake, token("t1"));
    let cancel = CancelToken::new();
    p.connect(&cancel).unwrap();
    fake.expire_sessions();
    fake.clear_requests();
    p.catalogue(&cancel).unwrap();
    let m = methods(&fake);
    assert_eq!(
        &m[..4],
        [
            "call:models_explore",
            "initialize",
            "notifications/initialized",
            "call:models_explore"
        ]
    );
    assert_eq!(
        fake.requests()[3].session.as_deref(),
        Some("fake-session-2")
    );
}

#[test]
fn a_session_404_on_a_cost_check_reopens_the_session_but_does_not_resend() {
    let fake = fake_higgsfield();
    let (p, _) = provider(&fake, token("t1"));
    let cancel = CancelToken::new();
    p.catalogue(&cancel).unwrap();
    p.connect(&cancel).unwrap();
    fake.expire_sessions();
    assert_eq!(
        p.quote(&image_op(), &run_tag(), &cancel),
        Err(CloudError::NotAccepted {
            reason: NotAcceptedReason::SessionExpired
        })
    );
    assert_eq!(fake.calls_to("generate_image").len(), 1);
    // The user's next Check cost works on the new session.
    assert!(matches!(
        p.quote(&image_op(), &run_tag(), &cancel),
        Ok(QuoteOutcome::Quote(_))
    ));
    assert_eq!(fake.calls_to("generate_image").len(), 2);
    let checks: Vec<_> = fake
        .requests()
        .into_iter()
        .filter(|r| r.tool() == Some("generate_image"))
        .collect();
    assert_eq!(checks[1].session.as_deref(), Some("fake-session-3"));
}

#[test]
fn a_401_on_a_free_call_renews_the_sign_in_and_resends_once() {
    let fake = fake_higgsfield();
    let tokens = SwapToken::new("t1", Some("t2"));
    let (p, _) = provider(&fake, tokens.clone());
    let cancel = CancelToken::new();
    p.connect(&cancel).unwrap();
    fake.reject_token("t1");
    fake.accept_token("t2");
    fake.clear_requests();
    p.catalogue(&cancel).unwrap();
    assert_eq!(tokens.renewals(), 1);
    let first_two: Vec<_> = fake
        .requests()
        .into_iter()
        .take(2)
        .map(|r| r.bearer)
        .collect();
    assert_eq!(first_two, vec![Some("t1".into()), Some("t2".into())]);
}

#[test]
fn a_401_after_renewal_is_sign_in_expired() {
    let fake = fake_higgsfield();
    let tokens = SwapToken::new("t1", None);
    let (p, _) = provider(&fake, tokens.clone());
    fake.reject_token("t1");
    assert_eq!(
        p.connect(&CancelToken::new()),
        Err(CloudError::SignInExpired)
    );
    assert_eq!(tokens.renewals(), 1);
    assert_eq!(fake.requests().len(), 1);
}

#[test]
fn a_401_on_a_cost_check_renews_but_never_resends() {
    let fake = fake_higgsfield();
    let tokens = SwapToken::new("t1", Some("t2"));
    let (p, _) = provider(&fake, tokens.clone());
    let cancel = CancelToken::new();
    p.catalogue(&cancel).unwrap();
    p.connect(&cancel).unwrap();
    fake.reject_token("t1");
    fake.accept_token("t2");
    assert_eq!(
        p.quote(&image_op(), &run_tag(), &cancel),
        Err(CloudError::NotAccepted {
            reason: NotAcceptedReason::Unauthorized
        })
    );
    assert_eq!(tokens.renewals(), 1);
    assert_eq!(fake.calls_to("generate_image").len(), 1);
}

#[test]
fn free_calls_retry_connect_failures_and_5xx_but_not_lost_answers() {
    let cancel = CancelToken::new();
    // Two refused connections, then success: nothing reached the server twice.
    let fake = fake_higgsfield();
    let (p, waits) = provider(&fake, token("t1"));
    p.connect(&cancel).unwrap();
    fake.clear_requests();
    fake.push_fault(Fault::Refuse);
    fake.push_fault(Fault::Refuse);
    p.catalogue(&cancel).unwrap();
    assert_eq!(waits.waits.lock().unwrap().len(), 2);
    assert_eq!(fake.calls_to("models_explore").len(), 3);

    // A 503 is retried.
    let fake = fake_higgsfield();
    let (p, _) = provider(&fake, token("t1"));
    p.connect(&cancel).unwrap();
    fake.clear_requests();
    fake.push_fault(Fault::Status(503, "busy".into()));
    p.catalogue(&cancel).unwrap();
    assert_eq!(fake.calls_to("models_explore").len(), 4);

    // An answer lost after sending is not retried.
    let fake = fake_higgsfield();
    let (p, _) = provider(&fake, token("t1"));
    p.connect(&cancel).unwrap();
    fake.clear_requests();
    fake.push_fault(Fault::DropAfterSend);
    assert_eq!(p.catalogue(&cancel), Err(CloudError::NoAnswer));
    assert_eq!(fake.calls_to("models_explore").len(), 1);

    // Retries stop after three.
    let fake = fake_higgsfield();
    let (p, waits) = provider(&fake, token("t1"));
    p.connect(&cancel).unwrap();
    for _ in 0..4 {
        fake.push_fault(Fault::Refuse);
    }
    assert_eq!(p.catalogue(&cancel), Err(CloudError::Offline));
    assert_eq!(waits.waits.lock().unwrap().len(), 3);
}

#[test]
fn cost_checks_retry_only_while_nothing_was_sent() {
    // How each failure of the check ends: retried and fine, refused (no run
    // can have started), or possibly started (fail closed, review M1).
    #[derive(Debug, PartialEq)]
    enum End {
        Quoted,
        Refused,
        PossiblyStarted,
    }
    let cancel = CancelToken::new();
    for (fault, end) in [
        (Fault::Refuse, End::Quoted),
        (Fault::DropAfterSend, End::PossiblyStarted),
        (
            Fault::Status(503, "unavailable".into()),
            End::PossiblyStarted,
        ),
        (Fault::Status(429, "slow down".into()), End::Refused),
        (Fault::Garbage, End::PossiblyStarted),
        (Fault::CutStream, End::PossiblyStarted),
    ] {
        let fake = fake_higgsfield();
        let (p, _) = provider(&fake, token("t1"));
        p.catalogue(&cancel).unwrap();
        p.connect(&cancel).unwrap();
        fake.push_fault(fault.clone());
        let got = match p.quote(&image_op(), &run_tag(), &cancel) {
            Ok(QuoteOutcome::Quote(_)) => End::Quoted,
            Ok(QuoteOutcome::PossiblyStarted(_)) => End::PossiblyStarted,
            Err(_) => End::Refused,
            Ok(other) => panic!("{fault:?}: {other:?}"),
        };
        assert_eq!(got, end, "{fault:?}");
        assert_eq!(fake.calls_to("generate_image").len(), 1, "{fault:?}");
        assert_eq!(
            p.guard().is_blocked("generate_image"),
            end == End::PossiblyStarted,
            "{fault:?}"
        );
    }
}

/// M1: an answer of an unknown shape may hide a paid run, so the check fails
/// closed: the tool's checks stop until restart, other tools keep theirs.
#[test]
fn an_unreadable_cost_answer_is_possibly_started_and_stops_further_checks() {
    for answer in [
        json!({"status": "queued"}),
        json!({"cost": {"credits": "a lot"}}),
        json!("Your generation is on its way"),
        json!(null),
    ] {
        let fake = fake_higgsfield();
        let reply = answer.clone();
        fake.on_tool("generate_image", move |_| ToolReply::Json(reply.clone()));
        let (p, _) = provider(&fake, token("t1"));
        let cancel = CancelToken::new();
        let QuoteOutcome::PossiblyStarted(started) =
            p.quote(&image_op(), &run_tag(), &cancel).unwrap()
        else {
            panic!("expected possibly started for {answer}");
        };
        assert_eq!(started.tool, "generate_image");
        assert_eq!(
            started.arguments,
            golden("get_cost-image-gpt_image_2_5.json")
        );
        assert_eq!(
            p.quote(&image_op(), &run_tag(), &cancel).unwrap(),
            QuoteOutcome::NotAvailable(Unavailable::PreflightStopped)
        );
        assert_eq!(fake.calls_to("generate_image").len(), 1, "{answer}");
        assert!(matches!(
            p.quote(&video_op(), &run_tag(), &cancel).unwrap(),
            QuoteOutcome::Quote(_)
        ));
    }
}

#[test]
fn a_cost_check_answered_with_job_ids_is_a_started_run_and_stops_further_checks() {
    let fake = fake_higgsfield();
    // A service that ignores get_cost and starts a run.
    fake.on_tool("generate_image", |_| {
        ToolReply::Json(
            json!({"jobs": [{"index": 0, "job_id": "5c6f0d1e-2b3a-4c4d-8e5f-6a7b8c9d0e1f"}]}),
        )
    });
    let (p, _) = provider(&fake, token("t1"));
    let cancel = CancelToken::new();
    let QuoteOutcome::StartedRun(run) = p.quote(&image_op(), &run_tag(), &cancel).unwrap() else {
        panic!("expected a started run");
    };
    assert_eq!(run.job_ids, vec!["5c6f0d1e-2b3a-4c4d-8e5f-6a7b8c9d0e1f"]);
    assert_eq!(run.tool, "generate_image");
    assert!(p.guard().is_blocked("generate_image"));
    // Exactly one submission per Check cost, and none after that.
    for _ in 0..3 {
        assert_eq!(
            p.quote(&image_op(), &run_tag(), &cancel).unwrap(),
            QuoteOutcome::NotAvailable(Unavailable::PreflightStopped)
        );
    }
    assert_eq!(fake.calls_to("generate_image").len(), 1);
    // Other tools keep their checks.
    assert!(matches!(
        p.quote(&video_op(), &run_tag(), &cancel).unwrap(),
        QuoteOutcome::Quote(_)
    ));
}

#[test]
fn no_check_is_sent_when_the_live_schema_drops_get_cost() {
    let fake = fake_higgsfield();
    let mut tools = common::tools();
    for t in &mut tools {
        if t["name"] == "generate_image" {
            t["inputSchema"]["properties"]["params"]["anyOf"][0]["properties"]
                .as_object_mut()
                .unwrap()
                .remove("get_cost");
        }
    }
    fake.set_tools(tools, 10);
    let (p, _) = provider(&fake, token("t1"));
    assert_eq!(
        p.quote(&image_op(), &run_tag(), &CancelToken::new())
            .unwrap(),
        QuoteOutcome::NotAvailable(Unavailable::NoEstimate)
    );
    assert!(fake.calls_to("generate_image").is_empty());
}

#[test]
fn schema_drift_disables_the_check_instead_of_guessing() {
    let fake = fake_higgsfield();
    let mut tools = common::tools();
    for t in &mut tools {
        if t["name"] == "generate_image" {
            t["inputSchema"]["properties"]["params"]["anyOf"][0]["properties"]["count"]["maximum"] =
                json!(0);
        }
    }
    fake.set_tools(tools, 10);
    let (p, _) = provider(&fake, token("t1"));
    assert!(matches!(
        p.quote(&image_op(), &run_tag(), &CancelToken::new()),
        Err(CloudError::SchemaDrift { ref tool, .. }) if tool == "generate_image"
    ));
    assert!(fake.calls_to("generate_image").is_empty());
}

#[test]
fn service_errors_from_a_cost_check_are_classified() {
    let message = fixture("tool-error-unknown-model.txt");
    let fake = fake_higgsfield();
    fake.on_tool("generate_image", move |_| ToolReply::Error(message.clone()));
    let (p, _) = provider(&fake, token("t1"));
    assert!(matches!(
        p.quote(&image_op(), &run_tag(), &CancelToken::new()),
        Err(CloudError::ModelUnavailable { .. })
    ));
    let fake = fake_higgsfield();
    fake.on_tool("generate_video", |_| {
        ToolReply::Error("Insufficient credits".into())
    });
    let (p, _) = provider(&fake, token("t1"));
    assert_eq!(
        p.quote(&video_op(), &run_tag(), &CancelToken::new()),
        Err(CloudError::InsufficientCredits)
    );
    // A refusal the service explains cannot have started a run.
    assert!(!p.guard().is_blocked("generate_video"));
}

#[test]
fn a_cancelled_call_sends_nothing() {
    let fake = fake_higgsfield();
    let (p, _) = provider(&fake, token("t1"));
    let cancel = CancelToken::new();
    cancel.cancel();
    assert_eq!(p.connect(&cancel), Err(CloudError::Cancelled));
    assert!(fake.requests().is_empty());
}

#[test]
fn closing_deletes_the_session() {
    let fake = fake_higgsfield();
    let (p, _) = provider(&fake, token("t1"));
    p.connect(&CancelToken::new()).unwrap();
    p.close();
    let last = fake.requests().pop().unwrap();
    assert_eq!(last.method, freecut_cloud::transport::Method::Delete);
    assert_eq!(last.session.as_deref(), Some("fake-session-1"));
    // F10: the close names the negotiated revision too.
    assert_eq!(last.protocol.as_deref(), Some("2025-11-25"));
    assert_eq!(p.server_info(), None);
}

// ---- Paid calls: never more than one submission (CG-010) ----

/// A fake whose generate tools "start runs".
fn paid_fake() -> freecut_cloud::fake::FakeService {
    let fake = fake_higgsfield();
    fake.on_tool("generate_image", |_| {
        ToolReply::Json(
            json!({"jobs": [{"index": 0, "job_id": "0b4f6d2a-1111-4222-8333-944455556666"}]}),
        )
    });
    fake
}

fn image_args() -> Value {
    json!({"params": {"model": "gpt_image_2_5", "prompt": "x", "count": 1}})
}

fn submit(c: &McpClient) -> Result<(), McpError> {
    c.call_paid(
        &allow::GENERATE_IMAGE,
        image_args(),
        &run_tag(),
        &CancelToken::new(),
    )
    .map(|_| ())
}

#[test]
fn paid_failure_points_each_end_in_their_state_with_at_most_one_submission() {
    use SubmitError::*;
    // The table of §13: each scripted failure at the paid request itself.
    let at_call = |fault: Fault| {
        let fake = paid_fake();
        let c = mcp_client(&fake, token("t1"));
        c.initialize(&CancelToken::new()).unwrap();
        fake.push_fault(fault);
        let err = submit(&c).unwrap_err();
        (
            submit_disposition(&err, &Value::Null),
            fake.calls_to("generate_image").len(),
        )
    };
    let (d, n) = at_call(Fault::Refuse);
    assert!(
        matches!(d, NotSent(CloudError::Offline)) && n == 0,
        "{d:?} {n}"
    );
    let (d, n) = at_call(Fault::DropAfterSend);
    assert!(
        matches!(
            d,
            OutcomeUnknown {
                cause: CloudError::NoAnswer,
                ..
            }
        ) && n == 1,
        "{d:?} {n}"
    );
    let (d, n) = at_call(Fault::Status(502, "bad gateway".into()));
    assert!(matches!(d, OutcomeUnknown { .. }) && n == 1, "{d:?} {n}");
    let (d, n) = at_call(Fault::Status(429, "too many concurrent jobs".into()));
    assert!(
        matches!(d, Rejected(CloudError::Busy)) && n == 1,
        "{d:?} {n}"
    );
    let (d, n) = at_call(Fault::Garbage);
    assert!(
        matches!(
            d,
            OutcomeUnknown {
                cause: CloudError::Protocol { .. },
                ..
            }
        ) && n == 1,
        "{d:?} {n}"
    );
    let (d, n) = at_call(Fault::CutStream);
    assert!(matches!(d, OutcomeUnknown { .. }) && n == 1, "{d:?} {n}");
    let (d, n) = at_call(Fault::RpcError(-32603, "Internal error".into()));
    assert!(matches!(d, OutcomeUnknown { .. }) && n == 1, "{d:?} {n}");
    let (d, n) = at_call(Fault::RpcError(-32602, "Invalid params".into()));
    assert!(matches!(d, Rejected(_)) && n == 1, "{d:?} {n}");

    // 401: renewed, not resent.
    let fake = paid_fake();
    let tokens = SwapToken::new("t1", Some("t2"));
    let c = mcp_client(&fake, tokens.clone());
    c.initialize(&CancelToken::new()).unwrap();
    fake.reject_token("t1");
    fake.accept_token("t2");
    let err = submit(&c).unwrap_err();
    assert!(matches!(
        submit_disposition(&err, &Value::Null),
        Rejected(CloudError::NotAccepted {
            reason: NotAcceptedReason::Unauthorized
        })
    ));
    assert_eq!(fake.calls_to("generate_image").len(), 1);
    assert_eq!(tokens.renewals(), 1);

    // Session 404: a new session, not resent.
    let fake = paid_fake();
    let c = mcp_client(&fake, token("t1"));
    c.initialize(&CancelToken::new()).unwrap();
    fake.expire_sessions();
    let err = submit(&c).unwrap_err();
    assert!(matches!(
        submit_disposition(&err, &Value::Null),
        Rejected(CloudError::NotAccepted {
            reason: NotAcceptedReason::SessionExpired
        })
    ));
    assert_eq!(fake.calls_to("generate_image").len(), 1);

    // A failed handshake before the call: the call was never sent.
    let fake = paid_fake();
    fake.push_fault(Fault::DropAfterSend); // initialize's answer is lost
    let err = submit(&mcp_client(&fake, token("t1"))).unwrap_err();
    assert!(
        matches!(
            submit_disposition(&err, &Value::Null),
            NotSent(CloudError::NoAnswer)
        ),
        "{err:?}"
    );
    assert!(fake.calls_to("generate_image").is_empty());
}

/// M2: a JSON-RPC error on a paid call is "rejected" only for the codes that
/// prove the method never ran; anything else is an unknown outcome.
#[test]
fn json_rpc_errors_on_paid_calls_are_unknown_unless_the_code_proves_nothing_ran() {
    use SubmitError::*;
    for (code, message, rejected) in [
        (-32603, "Internal error", false),
        (-32000, "Insufficient credits", false),
        (-32001, "Upstream timeout", false),
        (-32602, "Invalid params: count", true),
        (-32601, "Method not found", true),
    ] {
        let fake = fake_higgsfield();
        fake.on_tool("generate_image", move |_| {
            ToolReply::RpcError(code, message.into())
        });
        let c = mcp_client(&fake, token("t1"));
        let err = submit(&c).unwrap_err();
        let d = submit_disposition(&err, &Value::Null);
        if rejected {
            assert!(matches!(d, Rejected(_)), "{code}: {d:?}");
        } else {
            assert!(matches!(d, OutcomeUnknown { .. }), "{code}: {d:?}");
        }
        assert_eq!(fake.calls_to("generate_image").len(), 1, "{code}");
    }
    // The reason survives inside the unknown outcome.
    let fake = fake_higgsfield();
    fake.on_tool("generate_image", |_| {
        ToolReply::RpcError(-32000, "Insufficient credits".into())
    });
    let err = submit(&mcp_client(&fake, token("t1"))).unwrap_err();
    assert_eq!(
        submit_disposition(&err, &Value::Null),
        OutcomeUnknown {
            cause: CloudError::InsufficientCredits,
            job_ids: vec![]
        }
    );

    // And a cost check answered with such an error fails closed (M1).
    let fake = fake_higgsfield();
    fake.on_tool("generate_image", |_| {
        ToolReply::RpcError(-32603, "Internal error".into())
    });
    let (p, _) = provider(&fake, token("t1"));
    assert!(matches!(
        p.quote(&image_op(), &run_tag(), &CancelToken::new())
            .unwrap(),
        QuoteOutcome::PossiblyStarted(_)
    ));
}

/// Hands every answer to a call that carries a late sink to that sink and
/// tells the caller it was cancelled while waiting: the server got the call,
/// the caller gave up, the answer came later (review M3).
#[derive(Debug)]
struct CancelsAfterSending(freecut_cloud::fake::FakeService);

impl HttpTransport for CancelsAfterSending {
    fn send(
        &self,
        request: HttpRequest,
        cancel: &CancelToken,
    ) -> Result<HttpResponse, TransportError> {
        let Some(sink) = request.late.clone() else {
            return self.0.send(request, cancel);
        };
        let mut answer = self.0.send(request, cancel)?;
        let body = read_body(&mut answer.body, &CancelToken::new(), 1 << 20).unwrap();
        sink(LateResponse {
            status: answer.status,
            headers: answer.headers,
            body,
            complete: true,
        });
        Err(TransportError::maybe_sent(
            TransportErrorKind::Cancelled,
            "cancelled while waiting",
        ))
    }
}

/// Holds the answers to requests that carry a late sink and reports the
/// call as cancelled; `deliver` hands them to their sinks later, as a helper
/// thread does after the caller gave up.
#[derive(Default)]
struct DeliversLater {
    fake: Option<freecut_cloud::fake::FakeService>,
    held: std::sync::Mutex<Vec<(freecut_cloud::transport::LateSink, LateResponse)>>,
}

impl std::fmt::Debug for DeliversLater {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeliversLater").finish_non_exhaustive()
    }
}

impl DeliversLater {
    fn deliver(&self) {
        for (sink, response) in std::mem::take(&mut *self.held.lock().unwrap()) {
            sink(response);
        }
    }
}

impl HttpTransport for DeliversLater {
    fn send(
        &self,
        request: HttpRequest,
        cancel: &CancelToken,
    ) -> Result<HttpResponse, TransportError> {
        let fake = self.fake.as_ref().unwrap();
        let Some(sink) = request.late.clone() else {
            return fake.send(request, cancel);
        };
        let mut answer = fake.send(request, cancel)?;
        let body = read_body(&mut answer.body, &CancelToken::new(), 1 << 20).unwrap();
        self.held.lock().unwrap().push((
            sink,
            LateResponse {
                status: answer.status,
                headers: answer.headers,
                body,
                complete: true,
            },
        ));
        Err(TransportError::maybe_sent(
            TransportErrorKind::Cancelled,
            "cancelled while waiting",
        ))
    }
}

/// R10: the late queue belongs to the app. A cost check is cancelled, the
/// provider that sent it is dropped (reconnect, account switch), and only
/// then does the late answer arrive: its job id still reaches the app.
#[test]
fn late_job_ids_reach_the_app_after_the_provider_is_gone() {
    let fake = fake_higgsfield();
    fake.on_tool("generate_image", |_| {
        ToolReply::Json(json!({"cost": {"credits": 2}, "job_id": JOB}))
    });
    let transport = Arc::new(DeliversLater {
        fake: Some(fake.clone()),
        ..DeliversLater::default()
    });
    let late = Arc::new(freecut_cloud::mcp::LateQueue::new());
    let p = provider_on(
        transport.clone(),
        token("t1"),
        Arc::new(PreflightGuard::new()),
        Arc::clone(&late),
    );
    assert!(matches!(
        p.quote(&image_op(), &run_tag(), &CancelToken::new())
            .unwrap(),
        QuoteOutcome::PossiblyStarted(_)
    ));
    drop(p);
    assert!(late.take().is_empty());
    transport.deliver();
    let results = late.take();
    assert_eq!(results.len(), 1, "{results:?}");
    assert_eq!(results[0].job_ids, vec![JOB]);
    assert_eq!(results[0].tool, "generate_image");
}

/// R15: two runs of one tool are in flight and both are cancelled; their
/// late answers arrive in the other order. Each job id lands on the run
/// that caused it, marked as a paid call; a late cost check is marked as a
/// check. A paid tool cannot go out without a run.
#[test]
fn a_late_job_id_lands_on_the_run_that_caused_it() {
    use freecut_cloud::mcp::{CallClass, RunTag};
    const JOB_A: &str = "aaaaaaaa-1111-4222-8333-944455556666";
    const JOB_B: &str = "bbbbbbbb-1111-4222-8333-944455556666";
    let fake = fake_higgsfield();
    fake.on_tool("generate_image", |args| {
        let job = if args["params"]["prompt"] == "a" {
            JOB_A
        } else {
            JOB_B
        };
        ToolReply::Json(json!({"jobs": [{"job_id": job}]}))
    });
    let transport = Arc::new(DeliversLater {
        fake: Some(fake.clone()),
        ..DeliversLater::default()
    });
    let c = McpClient::new(
        freecut_cloud::provider::higgsfield::ENDPOINT,
        transport.clone(),
        token("t1"),
        Arc::new(common::NoWait::default()),
        "0.1.0",
    );
    let (run_a, run_b) = (RunTag::new("run-a"), RunTag::new("run-b"));
    for (run, prompt) in [(&run_a, "a"), (&run_b, "b")] {
        let args = json!({"params": {"model": "gpt_image_2_5", "prompt": prompt}});
        assert!(
            c.call_paid(&allow::GENERATE_IMAGE, args, run, &CancelToken::new())
                .is_err()
        );
    }
    transport.held.lock().unwrap().reverse();
    transport.deliver();
    let mut late = c.take_late_results();
    late.sort_by(|x, y| x.job_ids.cmp(&y.job_ids));
    assert_eq!(late.len(), 2);
    assert_eq!(
        (late[0].run.as_ref(), late[0].job_ids.as_slice()),
        (Some(&run_a), &[JOB_A.to_owned()][..])
    );
    assert_eq!(
        (late[1].run.as_ref(), late[1].job_ids.as_slice()),
        (Some(&run_b), &[JOB_B.to_owned()][..])
    );
    assert!(late.iter().all(|r| r.class == CallClass::Paid));

    // A late cost check is marked as one, with its run.
    let check = QuoteRequest::new(
        "generate_image",
        json!({"params": {"model": "m", "prompt": "a"}}),
    )
    .unwrap();
    let run_c = RunTag::new("run-c");
    assert!(
        c.check_cost(&allow::GENERATE_IMAGE, &check, &run_c, &CancelToken::new())
            .is_err()
    );
    transport.deliver();
    let late = c.take_late_results();
    assert_eq!(late.len(), 1);
    assert_eq!(
        (late[0].run.as_ref(), late[0].class),
        (Some(&run_c), CallClass::Preflight)
    );

    // call_tool takes free tools only.
    let before = fake.calls_to("generate_image").len();
    assert!(matches!(
        c.call_tool(&allow::GENERATE_IMAGE, json!({}), &CancelToken::new()),
        Err(McpError::Guard(_))
    ));
    assert_eq!(fake.calls_to("generate_image").len(), before);
}

/// M3: job ids in an answer that arrives after a cancel reach the late queue,
/// over JSON and SSE; free calls carry no late sink at all.
#[test]
fn job_ids_that_arrive_after_a_cancel_are_kept() {
    for stream in [StreamMode::Json, StreamMode::Sse { chunk: 7 }] {
        let fake = paid_fake();
        fake.set_stream(stream);
        let c = McpClient::new(
            freecut_cloud::provider::higgsfield::ENDPOINT,
            Arc::new(CancelsAfterSending(fake.clone())),
            token("t1"),
            Arc::new(common::NoWait::default()),
            "0.1.0",
        );
        // A free call has no late sink, so it goes through untouched.
        c.call_tool(
            &allow::MODELS_EXPLORE,
            json!({"action": "list", "type": "audio"}),
            &CancelToken::new(),
        )
        .unwrap();
        assert!(c.take_late_results().is_empty());

        let err = submit(&c).unwrap_err();
        assert!(
            matches!(
                submit_disposition(&err, &Value::Null),
                SubmitError::OutcomeUnknown { .. }
            ),
            "{err:?}"
        );
        assert_eq!(fake.calls_to("generate_image").len(), 1);
        let late = c.take_late_results();
        assert_eq!(late.len(), 1, "{stream:?}");
        assert_eq!(late[0].tool, "generate_image");
        assert_eq!(
            late[0].job_ids,
            vec!["0b4f6d2a-1111-4222-8333-944455556666"]
        );
        assert!(late[0].value.is_some(), "{stream:?}");
        assert!(c.take_late_results().is_empty(), "taking empties the queue");
    }
}

/// F4: MCP 2025-11-25 lets a server end a stream after its first event; the
/// client resumes with GET and Last-Event-ID, after the server's retry
/// delay, and never sends the request again.
#[test]
fn a_stream_closed_after_its_first_event_is_resumed_without_resending() {
    let fake = paid_fake();
    fake.set_stream(StreamMode::SseResumable { chunk: 4 });
    let c = mcp_client(&fake, token("t1"));
    submit(&c).unwrap();
    assert_eq!(fake.calls_to("generate_image").len(), 1);
    let gets: Vec<_> = fake
        .requests()
        .into_iter()
        .filter(|r| r.method == freecut_cloud::transport::Method::Get)
        .collect();
    // initialize, tools/call: one resume each (the notification needs none).
    assert_eq!(gets.len(), 2);
    assert!(gets.iter().all(|r| {
        r.last_event_id
            .as_deref()
            .is_some_and(|id| id.starts_with("ev-"))
    }));
    assert!(
        gets.iter()
            .all(|r| r.session.as_deref() == Some("fake-session-1"))
    );

    // A cost check through the provider, with the server's 5 ms delay.
    let fake = fake_higgsfield();
    fake.set_stream(StreamMode::SseResumable { chunk: 64 });
    let (p, waits) = provider(&fake, token("t1"));
    assert!(matches!(
        p.quote(&image_op(), &run_tag(), &CancelToken::new())
            .unwrap(),
        QuoteOutcome::Quote(_)
    ));
    assert_eq!(fake.calls_to("generate_image").len(), 1);
    assert!(
        waits
            .waits
            .lock()
            .unwrap()
            .iter()
            .all(|d| *d == std::time::Duration::from_millis(5))
    );
}

/// R3: a stream cut inside the response event, after its `id:` line, resumes
/// from the last whole event, so the server sends the response again.
#[test]
fn a_stream_cut_inside_the_answer_resumes_from_the_last_whole_event() {
    let fake = paid_fake();
    fake.set_stream(StreamMode::SseCutInsideAnswer { chunk: 7 });
    let c = mcp_client(&fake, token("t1"));
    let output = c
        .call_paid(
            &allow::GENERATE_IMAGE,
            image_args(),
            &run_tag(),
            &CancelToken::new(),
        )
        .unwrap();
    assert!(
        output
            .value()
            .unwrap()
            .to_string()
            .contains("0b4f6d2a-1111-4222-8333-944455556666")
    );
    assert_eq!(fake.calls_to("generate_image").len(), 1);
    let resumed_from: Vec<_> = fake
        .requests()
        .into_iter()
        .filter(|r| r.method == freecut_cloud::transport::Method::Get)
        .filter_map(|r| r.last_event_id)
        .collect();
    // initialize primes ev-1 and cuts inside ev-2; tools/call ev-3 and ev-4.
    assert_eq!(resumed_from, vec!["ev-1", "ev-3"]);
}

/// F4: if the stream cannot be resumed, the answer is lost: a paid call is an
/// unknown outcome, sent once; a free call fails without being resent.
#[test]
fn a_stream_that_cannot_be_resumed_loses_the_answer() {
    for mode in [ResumeMode::Refuse(405), ResumeMode::Stall] {
        let fake = paid_fake();
        let c = mcp_client(&fake, token("t1"));
        c.initialize(&CancelToken::new()).unwrap();
        fake.set_stream(StreamMode::SseResumable { chunk: 16 });
        fake.set_resume(mode);
        let err = submit(&c).unwrap_err();
        assert!(
            matches!(
                submit_disposition(&err, &Value::Null),
                SubmitError::OutcomeUnknown { .. }
            ),
            "{mode:?}: {err:?}"
        );
        assert_eq!(fake.calls_to("generate_image").len(), 1, "{mode:?}");
        let gets = fake
            .requests()
            .iter()
            .filter(|r| r.method == freecut_cloud::transport::Method::Get)
            .count();
        let expected = if mode == ResumeMode::Stall {
            freecut_cloud::mcp::MAX_RESUMES as usize
        } else {
            1
        };
        assert_eq!(gets, expected, "{mode:?}");

        let err = c
            .call_tool(
                &allow::MODELS_EXPLORE,
                json!({"action": "list", "type": "audio"}),
                &CancelToken::new(),
            )
            .unwrap_err();
        assert!(matches!(err, McpError::Protocol { .. }), "{err:?}");
        assert_eq!(fake.calls_to("models_explore").len(), 1, "{mode:?}");
    }
}

/// F8: a cancel after the request may have left sends
/// notifications/cancelled with its id; a cancel before sending sends
/// nothing.
#[test]
fn a_cancel_after_sending_tells_the_server() {
    let fake = paid_fake();
    let cancel = CancelToken::new();
    let trigger = cancel.clone();
    fake.on_tool("generate_image", move |_| {
        trigger.cancel(); // the user cancels while the call is in flight
        ToolReply::Json(
            json!({"jobs": [{"index": 0, "job_id": "0b4f6d2a-1111-4222-8333-944455556666"}]}),
        )
    });
    let c = mcp_client(&fake, token("t1"));
    c.initialize(&CancelToken::new()).unwrap();
    let err = c
        .call_paid(&allow::GENERATE_IMAGE, image_args(), &run_tag(), &cancel)
        .unwrap_err();
    assert!(
        matches!(
            submit_disposition(&err, &Value::Null),
            SubmitError::OutcomeUnknown { .. }
        ),
        "{err:?}"
    );
    let requests = fake.requests();
    let call_id = requests
        .iter()
        .find(|r| r.tool() == Some("generate_image"))
        .unwrap()
        .body["id"]
        .clone();
    let notices: Vec<_> = requests
        .iter()
        .filter(|r| r.rpc_method() == Some("notifications/cancelled"))
        .collect();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].body["params"]["requestId"], call_id);
    assert_eq!(notices[0].session.as_deref(), Some("fake-session-1"));

    // Cancelled before sending: nothing reaches the server.
    let fake = paid_fake();
    let c = mcp_client(&fake, token("t1"));
    c.initialize(&CancelToken::new()).unwrap();
    let cancel = CancelToken::new();
    cancel.cancel();
    assert!(
        c.call_paid(&allow::GENERATE_IMAGE, image_args(), &run_tag(), &cancel)
            .is_err()
    );
    assert!(
        fake.requests()
            .iter()
            .all(|r| r.rpc_method() != Some("notifications/cancelled"))
    );
    assert!(fake.calls_to("generate_image").is_empty());
}

/// R1: a `404` to `notifications/initialized` ends the handshake. It must not
/// start another one (which recursed until the stack overflowed).
#[test]
fn a_session_404_during_the_handshake_fails_once_instead_of_recursing() {
    let fake = paid_fake();
    fake.fail_method(
        "notifications/initialized",
        Fault::Status(404, "unknown session".into()),
    );
    let c = mcp_client(&fake, token("t1"));
    let err = submit(&c).unwrap_err();
    assert!(
        matches!(
            submit_disposition(&err, &Value::Null),
            SubmitError::NotSent(_)
        ),
        "{err:?}"
    );
    assert_eq!(
        methods(&fake),
        vec!["initialize", "notifications/initialized"]
    );
    assert!(fake.calls_to("generate_image").is_empty());
    assert_eq!(c.session_id(), None);

    // The same through the provider: connect fails once, nothing is listed.
    let fake = fake_higgsfield();
    fake.fail_method(
        "notifications/initialized",
        Fault::Status(404, "unknown session".into()),
    );
    let (p, _) = provider(&fake, token("t1"));
    assert!(p.connect(&CancelToken::new()).is_err());
    assert_eq!(
        methods(&fake),
        vec!["initialize", "notifications/initialized"]
    );

    // Once the server behaves, the next call starts a fresh handshake.
    fake.clear_method_faults();
    p.connect(&CancelToken::new()).unwrap();
    assert_eq!(methods(&fake)[2], "initialize");
}

/// R2: whatever fails, no API path retries or resends a tool the allow list
/// marks paid, and its cost checks go out once per Check cost.
#[test]
fn no_api_path_retries_or_resends_a_paid_tool() {
    use freecut_cloud::fake::FakeService;
    use freecut_cloud::mcp::CallClass;
    let paid: Vec<_> = allow::ALLOW_LIST
        .iter()
        .copied()
        .filter(|t| t.class() == CallClass::Paid)
        .collect();
    assert!(paid.len() >= 4);
    type Inject = Box<dyn Fn(&FakeService)>;
    let faults: Vec<Inject> = vec![
        Box::new(|f| f.push_fault(Fault::Status(503, "unavailable".into()))),
        Box::new(|f| f.push_fault(Fault::Status(429, "slow down".into()))),
        Box::new(|f| f.push_fault(Fault::DropAfterSend)),
        Box::new(|f| f.expire_sessions()),
        Box::new(|f| {
            f.reject_token("t1");
            f.accept_token("t2");
        }),
    ];
    let ready = || {
        let fake = fake_higgsfield();
        // This test sends paid calls on purpose.
        for tool in allow::ALLOW_LIST {
            fake.on_tool(tool.name(), |_| ToolReply::Json(json!({"ok": true})));
        }
        let c = mcp_client(&fake, SwapToken::new("t1", Some("t2")));
        c.initialize(&CancelToken::new()).unwrap();
        (fake, c)
    };
    for tool in paid {
        for (i, inject) in faults.iter().enumerate() {
            let (fake, c) = ready();
            inject(&fake);
            let _ = c.call_paid(
                tool,
                json!({"params": {"model": "m"}}),
                &run_tag(),
                &CancelToken::new(),
            );
            assert_eq!(
                fake.calls_to(tool.name()).len(),
                1,
                "{} fault {i}",
                tool.name()
            );

            if tool.has_cost_preflight() {
                let (fake, c) = ready();
                inject(&fake);
                let quote =
                    QuoteRequest::new(tool.name(), json!({"params": {"model": "m"}})).unwrap();
                let _ = c.check_cost(tool, &quote, &run_tag(), &CancelToken::new());
                let sent = fake.calls_to(tool.name());
                assert_eq!(sent.len(), 1, "check of {} fault {i}", tool.name());
                assert_eq!(sent[0]["params"]["get_cost"], json!(true));
            }
        }
    }
}

#[test]
fn cost_checks_are_refused_for_tools_without_one_or_for_another_tool() {
    let fake = fake_higgsfield();
    let c = mcp_client(&fake, token("t1"));
    let quote = QuoteRequest::new("media_confirm", json!({"params": {}})).unwrap();
    assert!(matches!(
        c.check_cost(
            &allow::MEDIA_CONFIRM,
            &quote,
            &run_tag(),
            &CancelToken::new()
        ),
        Err(McpError::Guard(_))
    ));
    let quote = QuoteRequest::new("generate_video", json!({"params": {}})).unwrap();
    assert!(matches!(
        c.check_cost(
            &allow::GENERATE_IMAGE,
            &quote,
            &run_tag(),
            &CancelToken::new()
        ),
        Err(McpError::Guard(_))
    ));
    assert!(fake.requests().is_empty());
}

/// R4: an answer that breaks off unreadably still names its jobs. A paid
/// call is "outcome unknown" with them; a cost check reports the run it
/// started.
#[test]
fn unreadable_answers_keep_the_job_ids_they_name() {
    for fault in [
        Fault::GarbageWith(format!("{{\"jobs\":[{{\"job_id\":\"{JOB}\"")),
        Fault::CutStreamWith(format!("queued {JOB}, then")),
    ] {
        let fake = paid_fake();
        let c = mcp_client(&fake, token("t1"));
        c.initialize(&CancelToken::new()).unwrap();
        fake.push_fault(fault.clone());
        let d = submit_disposition(&submit(&c).unwrap_err(), &Value::Null);
        assert!(
            matches!(&d, SubmitError::OutcomeUnknown { job_ids, .. } if job_ids == &[JOB]),
            "{fault:?} gave {d:?}"
        );
        assert_eq!(fake.calls_to("generate_image").len(), 1);
    }

    let fake = fake_higgsfield();
    let (p, _) = provider(&fake, token("t1"));
    p.connect(&CancelToken::new()).unwrap();
    p.catalogue(&CancelToken::new()).unwrap();
    fake.push_fault(Fault::CutStreamWith(format!("queued {JOB}, then")));
    match p
        .quote(&image_op(), &run_tag(), &CancelToken::new())
        .unwrap()
    {
        QuoteOutcome::StartedRun(run) => assert_eq!(run.job_ids, vec![JOB]),
        other => panic!("{other:?}"),
    }
    assert!(p.guard().is_blocked("generate_image"));
}

/// R5: a refusal that names a job is never "rejected": whether the id is in
/// a plain-text or JSON tool error, in any content item, or in a JSON-RPC
/// error's data, even with a code that otherwise proves nothing ran. A cost
/// check that names one reports the run.
#[test]
fn refusals_that_name_a_job_are_never_rejected() {
    let disposition = |fault: Fault| {
        let fake = paid_fake();
        let c = mcp_client(&fake, token("t1"));
        c.initialize(&CancelToken::new()).unwrap();
        fake.push_fault(fault);
        submit_disposition(&submit(&c).unwrap_err(), &Value::Null)
    };
    for fault in [
        Fault::ToolError(format!("Job {JOB} failed: model overloaded")),
        Fault::ToolError(format!("{{\"error\":\"busy\",\"job_id\":\"{JOB}\"}}")),
        // Glued to a prefix (R13).
        Fault::ToolError(format!("request-{JOB} failed")),
        Fault::RpcErrorData(-32602, "Invalid params".into(), json!({"job_id": JOB})),
        Fault::RpcError(-32600, format!("request {JOB} was already queued")),
    ] {
        let d = disposition(fault.clone());
        assert!(
            matches!(&d, SubmitError::OutcomeUnknown { job_ids, .. } if job_ids == &[JOB]),
            "{fault:?} gave {d:?}"
        );
    }
    // Without an id they stay refusals.
    assert!(matches!(
        disposition(Fault::ToolError("NSFW content".into())),
        SubmitError::Rejected(_)
    ));
    assert!(matches!(
        disposition(Fault::RpcErrorData(
            -32602,
            "Invalid params".into(),
            json!({"field": "x"})
        )),
        SubmitError::Rejected(_)
    ));
    // An id in a later text item of an error result.
    let err = McpError::Tool {
        message: "failed".into(),
        structured: None,
        texts: vec!["failed".into(), format!("see job {JOB}")],
        job_ids: vec![],
    };
    assert_eq!(err.job_ids(), vec![JOB]);

    // Cost checks: a refusal or an answer naming a job is a started run.
    for reply in [
        ToolReply::Error(format!("Job {JOB} failed")),
        ToolReply::Json(json!({"cost": {"credits": 2}, "note": format!("queued {JOB}")})),
    ] {
        let fake = fake_higgsfield();
        let reply = std::sync::Mutex::new(Some(reply));
        fake.on_tool("generate_image", move |_| {
            reply
                .lock()
                .unwrap()
                .take()
                .unwrap_or(ToolReply::Json(json!({})))
        });
        let (p, _) = provider(&fake, token("t1"));
        match p
            .quote(&image_op(), &run_tag(), &CancelToken::new())
            .unwrap()
        {
            QuoteOutcome::StartedRun(run) => assert_eq!(run.job_ids, vec![JOB]),
            other => panic!("{other:?}"),
        }
    }
}

/// R6: two threads check the same tool at once. One check goes out; the
/// other is refused without sending anything, instead of a second check
/// (which can spend) going out beside it.
#[test]
fn two_checks_of_one_tool_never_go_out_together() {
    use std::sync::Mutex;
    use std::sync::mpsc::channel;
    use std::time::Duration;
    let fake = fake_higgsfield();
    let (started_tx, started_rx) = channel();
    let (go_tx, go_rx) = channel::<()>();
    let (started_tx, go_rx) = (Mutex::new(started_tx), Mutex::new(go_rx));
    fake.on_tool("generate_image", move |_| {
        let _ = started_tx.lock().unwrap().send(());
        let _ = go_rx.lock().unwrap().recv_timeout(Duration::from_secs(5));
        ToolReply::Json(json!({"cost": {"credits": 2, "credits_exact": 2}}))
    });
    let (p, _) = provider(&fake, token("t1"));
    let p = Arc::new(p);
    p.connect(&CancelToken::new()).unwrap();
    p.catalogue(&CancelToken::new()).unwrap();

    let first = {
        let p = Arc::clone(&p);
        std::thread::spawn(move || p.quote(&image_op(), &run_tag(), &CancelToken::new()))
    };
    started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let (done_tx, done_rx) = channel();
    {
        let p = Arc::clone(&p);
        std::thread::spawn(move || {
            let _ = done_tx.send(p.quote(&image_op(), &run_tag(), &CancelToken::new()));
        });
    }
    let second = done_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("the second check went out and is waiting for its answer");
    assert_eq!(
        second.unwrap(),
        QuoteOutcome::NotAvailable(Unavailable::PreflightStopped)
    );
    go_tx.send(()).unwrap();
    assert!(matches!(
        first.join().unwrap().unwrap(),
        QuoteOutcome::Quote(_)
    ));
    assert_eq!(fake.calls_to("generate_image").len(), 1);

    // A readable answer released the tool.
    go_tx.send(()).unwrap();
    assert!(matches!(
        p.quote(&image_op(), &run_tag(), &CancelToken::new())
            .unwrap(),
        QuoteOutcome::Quote(_)
    ));
    assert_eq!(fake.calls_to("generate_image").len(), 2);
}

/// R6: the app owns the guard. A block set through one provider holds for
/// the next one the app creates (reconnect, account switch) until FreeCut
/// restarts.
#[test]
fn a_block_outlives_the_provider_that_set_it() {
    let guard = Arc::new(PreflightGuard::new());
    let fake = fake_higgsfield();
    let (p, _) = provider_with_guard(&fake, token("t1"), Arc::clone(&guard));
    p.connect(&CancelToken::new()).unwrap();
    p.catalogue(&CancelToken::new()).unwrap();
    fake.push_fault(Fault::Garbage);
    assert!(matches!(
        p.quote(&image_op(), &run_tag(), &CancelToken::new())
            .unwrap(),
        QuoteOutcome::PossiblyStarted(_)
    ));
    drop(p);

    let (again, _) = provider_with_guard(&fake, token("t1"), Arc::clone(&guard));
    assert_eq!(
        again
            .quote(&image_op(), &run_tag(), &CancelToken::new())
            .unwrap(),
        QuoteOutcome::NotAvailable(Unavailable::PreflightStopped)
    );
    assert_eq!(fake.calls_to("generate_image").len(), 1);
}

/// The recorded service is stateless (§15.1): no `Mcp-Session-Id`. Calls
/// carry no session header, a paid call still goes out once, and closing
/// sends nothing.
#[test]
fn a_stateless_server_needs_no_session() {
    let fake = paid_fake();
    fake.set_sessions(false);
    fake.set_stream(StreamMode::Sse { chunk: 64 });
    let c = mcp_client(&fake, token("t1"));
    c.initialize(&CancelToken::new()).unwrap();
    assert_eq!(c.session_id(), None);
    submit(&c).unwrap();
    c.call_tool(
        &allow::MODELS_EXPLORE,
        json!({"action": "list", "type": "image"}),
        &CancelToken::new(),
    )
    .unwrap();
    c.close();
    let requests = fake.requests();
    assert!(requests.iter().all(|r| r.session.is_none()));
    assert!(
        requests
            .iter()
            .all(|r| r.method != freecut_cloud::transport::Method::Delete)
    );
    assert_eq!(fake.calls_to("generate_image").len(), 1);
}

/// R14: a refusal that echoes the request's own media id is not a job. A
/// paid call stays Rejected; a cost check is refused and releases its tool.
#[test]
fn a_refusal_that_echoes_a_media_id_is_not_a_job() {
    const MEDIA: &str = "123e4567-e89b-12d3-a456-426614174000";
    let args = json!({"params": {"model": "gpt_image_2_5", "prompt": "x",
        "medias": [{"value": MEDIA, "role": "image_references"}]}});
    for fault in [
        Fault::ToolError(format!("media {MEDIA} not found")),
        Fault::RpcErrorData(-32602, "Invalid params".into(), json!({"media": MEDIA})),
        Fault::ToolError("00000000-0000-0000-0000-000000000000 is not a valid id".into()),
    ] {
        let fake = paid_fake();
        let c = mcp_client(&fake, token("t1"));
        c.initialize(&CancelToken::new()).unwrap();
        fake.push_fault(fault.clone());
        let err = c
            .call_paid(
                &allow::GENERATE_IMAGE,
                args.clone(),
                &run_tag(),
                &CancelToken::new(),
            )
            .unwrap_err();
        assert!(
            matches!(submit_disposition(&err, &args), SubmitError::Rejected(_)),
            "{fault:?}"
        );
        assert_eq!(fake.calls_to("generate_image").len(), 1);
    }

    let fake = fake_higgsfield();
    fake.on_tool("generate_image", |_| {
        ToolReply::Error(format!("media {MEDIA} not found"))
    });
    let (p, _) = provider(&fake, token("t1"));
    let mut op = image_op();
    if let Operation::Image(r) = &mut op {
        r.refs.push(freecut_cloud::provider::RoleRef {
            role: freecut_cloud::catalogue::Role::parse("image_references"),
            media: freecut_cloud::provider::RemoteMediaId::parse(MEDIA).unwrap(),
        });
    }
    assert!(p.quote(&op, &run_tag(), &CancelToken::new()).is_err());
    assert!(!p.guard().is_blocked("generate_image"));
    assert!(p.quote(&op, &run_tag(), &CancelToken::new()).is_err());
    assert_eq!(
        fake.calls_to("generate_image").len(),
        2,
        "the tool was released"
    );
}

/// Breaks the body of a `tools/call` answer right after the fake's job id,
/// as a connection reset would.
#[derive(Debug)]
struct BreaksAfterTheJobId(freecut_cloud::fake::FakeService);

struct Broken(std::io::Cursor<Vec<u8>>);

impl std::io::Read for Broken {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self.0.read(buf)? {
            0 => Err(std::io::Error::from(std::io::ErrorKind::ConnectionReset)),
            n => Ok(n),
        }
    }
}

impl HttpTransport for BreaksAfterTheJobId {
    fn send(
        &self,
        request: HttpRequest,
        cancel: &CancelToken,
    ) -> Result<HttpResponse, TransportError> {
        let paid = String::from_utf8_lossy(&request.body).contains("generate_image");
        let mut answer = self.0.send(request, cancel)?;
        if !paid {
            return Ok(answer);
        }
        let body = read_body(&mut answer.body, &CancelToken::new(), 1 << 20).unwrap();
        let text = String::from_utf8_lossy(&body);
        let cut = text.find(PAID_JOB).unwrap() + PAID_JOB.len() + 2;
        answer.body = Box::new(Broken(std::io::Cursor::new(body[..cut].to_vec())));
        Ok(answer)
    }
}

/// The job id `paid_fake` answers with.
const PAID_JOB: &str = "0b4f6d2a-1111-4222-8333-944455556666";

/// R12: every byte of a paid call's or a cost check's answer is scanned for
/// job ids: a `resource_link` item, a 4xx body, a 5xx body past the part
/// kept for the message, a progress event before the response, and what
/// arrived of a JSON answer before the read broke.
#[test]
fn every_byte_of_an_answer_is_scanned_for_job_ids() {
    use freecut_cloud::fake::FakeService;
    let progress = json!({"jsonrpc": "2.0", "method": "notifications/progress",
        "params": {"progressToken": 1, "progress": 1, "message": format!("queued {JOB}")}});
    type Setup = Box<dyn Fn(&FakeService)>;
    let cases: Vec<(&str, Setup, Option<Fault>)> = vec![
        (
            "resource_link in an isError result",
            Box::new(|f| {
                f.on_tool("generate_image", |_| {
                    ToolReply::Raw(json!({"isError": true, "content": [
                        {"type": "text", "text": "failed"},
                        {"type": "resource_link", "name": "job",
                            "uri": format!("https://higgsfield.ai/jobs/{JOB}")}]}))
                });
            }),
            None,
        ),
        (
            "4xx body",
            Box::new(|_| {}),
            Some(Fault::Status(409, format!("job {JOB} already exists"))),
        ),
        (
            "long 5xx body",
            Box::new(|_| {}),
            Some(Fault::Status(502, format!("{}{JOB}", "x".repeat(3000)))),
        ),
        (
            "progress event before a refusal",
            Box::new(move |f| {
                f.set_stream(StreamMode::Sse { chunk: 64 });
                f.send_before_response(vec![progress.clone()]);
                f.on_tool("generate_image", |_| {
                    ToolReply::Error("NSFW content".into())
                });
            }),
            None,
        ),
    ];
    for (name, setup, fault) in cases {
        let fake = paid_fake();
        setup(&fake);
        let c = mcp_client(&fake, token("t1"));
        c.initialize(&CancelToken::new()).unwrap();
        if let Some(f) = fault {
            fake.push_fault(f);
        }
        let d = submit_disposition(&submit(&c).unwrap_err(), &image_args());
        assert!(
            matches!(&d, SubmitError::OutcomeUnknown { job_ids, .. } if job_ids == &[JOB]),
            "{name}: {d:?}"
        );
        assert_eq!(fake.calls_to("generate_image").len(), 1, "{name}");
    }

    // A JSON answer cut right after its job id.
    let fake = paid_fake();
    let c = McpClient::new(
        freecut_cloud::provider::higgsfield::ENDPOINT,
        Arc::new(BreaksAfterTheJobId(fake.clone())),
        token("t1"),
        Arc::new(common::NoWait::default()),
        "0.1.0",
    );
    let d = submit_disposition(&submit(&c).unwrap_err(), &image_args());
    assert!(
        matches!(&d, SubmitError::OutcomeUnknown { job_ids, .. } if job_ids == &[PAID_JOB]),
        "{d:?}"
    );

    // A cost check whose answer follows a progress event naming a job.
    let fake = fake_higgsfield();
    fake.set_stream(StreamMode::Sse { chunk: 64 });
    fake.send_before_response(vec![
        json!({"jsonrpc": "2.0", "method": "notifications/progress",
        "params": {"progressToken": 1, "progress": 1, "message": format!("queued {JOB}")}}),
    ]);
    let (p, _) = provider(&fake, token("t1"));
    match p
        .quote(&image_op(), &run_tag(), &CancelToken::new())
        .unwrap()
    {
        QuoteOutcome::StartedRun(run) => assert_eq!(run.job_ids, vec![JOB]),
        other => panic!("{other:?}"),
    }
}

/// R19: a model the service lists but FreeCut has not reviewed is refused
/// before any request: one in the recorded listing, and one the service
/// added since. Nothing reaches a generate tool.
#[test]
fn an_unreviewed_model_is_refused_before_any_request() {
    use freecut_cloud::catalogue::ModelId;
    let fake = fake_higgsfield();
    fake.on_tool("models_explore", |args| {
        let kind = args["type"].as_str().unwrap_or_default();
        let mut listing = fixture_json(&format!("models_explore-list-{kind}.json"));
        if kind == "video" {
            let mut added = listing["items"][0].clone();
            added["id"] = json!("brand_new_video_9");
            listing["items"].as_array_mut().unwrap().push(added);
        }
        ToolReply::Json(listing)
    });
    let (p, _) = provider(&fake, token("t1"));
    let mut image = image_op();
    if let Operation::Image(r) = &mut image {
        r.model = ModelId::new("nano_banana_2_shots");
    }
    let mut video = video_op();
    if let Operation::Video(r) = &mut video {
        r.model = ModelId::new("brand_new_video_9");
    }
    for op in [image, video] {
        match p.quote(&op, &run_tag(), &CancelToken::new()) {
            Err(CloudError::ModelUnavailable { message }) => assert!(
                message.contains("is not offered in FreeCut (not listed or not reviewed)"),
                "{message}"
            ),
            other => panic!("{other:?}"),
        }
    }
    assert!(fake.calls_to("generate_image").is_empty());
    assert!(fake.calls_to("generate_video").is_empty());
}

/// R31: on a stateless server a 404 is an ordinary error, not an expired
/// session: no new handshake, nothing sent again; a paid call is Rejected.
#[test]
fn a_404_from_a_stateless_server_is_an_ordinary_error() {
    let fake = paid_fake();
    fake.set_sessions(false);
    let c = mcp_client(&fake, token("t1"));
    c.initialize(&CancelToken::new()).unwrap();
    fake.push_fault(Fault::Status(404, "not found".into()));
    let err = c
        .call_tool(
            &allow::MODELS_EXPLORE,
            json!({"action": "list", "type": "image"}),
            &CancelToken::new(),
        )
        .unwrap_err();
    assert!(matches!(err, McpError::Http { status: 404, .. }), "{err:?}");
    fake.push_fault(Fault::Status(404, "not found".into()));
    let err = submit(&c).unwrap_err();
    assert!(
        matches!(
            submit_disposition(&err, &image_args()),
            SubmitError::Rejected(_)
        ),
        "{err:?}"
    );
    assert_eq!(fake.calls_to("generate_image").len(), 1);
    let methods = methods(&fake);
    assert_eq!(
        methods.iter().filter(|m| *m == "initialize").count(),
        1,
        "{methods:?}"
    );
    assert_eq!(
        methods
            .iter()
            .filter(|m| *m == "call:models_explore")
            .count(),
        1,
        "{methods:?}"
    );
}

/// R35: a 401, 403 or session-404 body that names a job is read on paid
/// calls and cost checks: the run is Outcome unknown with the job (a cost
/// check a started run), never Rejected, and nothing is renewed or resent.
#[test]
fn auth_and_session_refusals_that_name_a_job_keep_it() {
    for status in [401, 403, 404] {
        let fake = paid_fake();
        let c = mcp_client(&fake, SwapToken::new("t1", Some("t2")));
        c.initialize(&CancelToken::new()).unwrap();
        fake.push_fault(Fault::Status(
            status,
            format!("job {JOB} was created, then: denied"),
        ));
        let err = submit(&c).unwrap_err();
        let d = submit_disposition(&err, &image_args());
        assert!(
            matches!(&d, SubmitError::OutcomeUnknown { job_ids, .. } if job_ids == &[JOB]),
            "{status}: {d:?}"
        );
        assert_eq!(fake.calls_to("generate_image").len(), 1, "{status}");
    }
    // Without a job they stay what they were.
    let fake = paid_fake();
    let c = mcp_client(&fake, token("t1"));
    c.initialize(&CancelToken::new()).unwrap();
    fake.push_fault(Fault::Status(403, "forbidden".into()));
    assert!(matches!(
        submit(&c).unwrap_err(),
        McpError::Forbidden { .. }
    ));

    // A cost check refused with 403 that names a job started a run.
    let fake = fake_higgsfield();
    let (p, _) = provider(&fake, token("t1"));
    p.connect(&CancelToken::new()).unwrap();
    p.catalogue(&CancelToken::new()).unwrap();
    fake.push_fault(Fault::Status(403, format!("job {JOB}: plan required")));
    match p
        .quote(&image_op(), &run_tag(), &CancelToken::new())
        .unwrap()
    {
        QuoteOutcome::StartedRun(run) => assert_eq!(run.job_ids, vec![JOB]),
        other => panic!("{other:?}"),
    }
    assert!(p.guard().is_blocked("generate_image"));
}

/// Answers the paid call with a stream that primes `id: ev-1`, sends a
/// progress event naming [`JOB`], and ends before the response.
#[derive(Debug)]
struct EndsAfterProgress(freecut_cloud::fake::FakeService);

impl HttpTransport for EndsAfterProgress {
    fn send(
        &self,
        request: HttpRequest,
        cancel: &CancelToken,
    ) -> Result<HttpResponse, TransportError> {
        if !String::from_utf8_lossy(&request.body).contains("generate_image") {
            return self.0.send(request, cancel);
        }
        let progress = json!({"jsonrpc": "2.0", "method": "notifications/progress",
            "params": {"progressToken": 1, "progress": 1, "message": format!("queued {JOB}")}});
        Ok(HttpResponse::new(
            200,
            vec![("content-type".into(), "text/event-stream".into())],
            format!("id: ev-1\nretry: 5\ndata:\n\nevent: message\ndata: {progress}\n\n")
                .into_bytes(),
        ))
    }
}

/// Fires the cancel token when the client waits to resume.
#[derive(Debug)]
struct CancelsOnWait(CancelToken);

impl freecut_cloud::transport::Waiter for CancelsOnWait {
    fn wait(&self, _: std::time::Duration, _: &CancelToken) -> bool {
        self.0.cancel();
        false
    }

    fn jitter(&self) -> u32 {
        0
    }
}

/// R37: a stream ends after naming a job, and the user cancels during the
/// resume wait: the call is Outcome unknown with that job, not without it.
#[test]
fn a_cancel_while_resuming_keeps_the_job_ids_already_read() {
    let fake = paid_fake();
    let cancel = CancelToken::new();
    let c = McpClient::new(
        freecut_cloud::provider::higgsfield::ENDPOINT,
        Arc::new(EndsAfterProgress(fake.clone())),
        token("t1"),
        Arc::new(CancelsOnWait(cancel.clone())),
        "0.1.0",
    );
    c.initialize(&CancelToken::new()).unwrap();
    let err = c
        .call_paid(&allow::GENERATE_IMAGE, image_args(), &run_tag(), &cancel)
        .unwrap_err();
    assert!(matches!(err, McpError::Cancelled { .. }), "{err:?}");
    let d = submit_disposition(&err, &image_args());
    assert!(
        matches!(&d, SubmitError::OutcomeUnknown { job_ids, .. } if job_ids == &[JOB]),
        "{d:?}"
    );
}

/// A UUID in an SSE event id, as some servers write them (the reference
/// SDK's event store does).
const EVENT_ID: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";

/// Puts a UUID in front of each SSE event id of the fake (and takes it off
/// a `Last-Event-ID` again), and opens the stream with a comment that names
/// one.
#[derive(Debug)]
struct UuidEventIds(freecut_cloud::fake::FakeService);

impl HttpTransport for UuidEventIds {
    fn send(
        &self,
        mut request: HttpRequest,
        cancel: &CancelToken,
    ) -> Result<HttpResponse, TransportError> {
        for (name, value) in &mut request.headers {
            let prefix = format!("{EVENT_ID}_");
            if name == "last-event-id" && value.starts_with(&prefix) {
                *value = value[prefix.len()..].to_owned();
            }
        }
        let mut answer = self.0.send(request, cancel)?;
        if answer.media_type().as_deref() != Some("text/event-stream") {
            return Ok(answer);
        }
        let body = read_body(&mut answer.body, &CancelToken::new(), 1 << 20).unwrap();
        let mut text = format!(": trace {EVENT_ID}\n\n");
        for line in String::from_utf8_lossy(&body).split_inclusive('\n') {
            if let Some(id) = line.strip_prefix("id:") {
                text.push_str(&format!("id: {EVENT_ID}_{}\n", id.trim()));
            } else {
                text.push_str(line);
            }
        }
        answer.body = Box::new(std::io::Cursor::new(text.into_bytes()));
        Ok(answer)
    }
}

/// R38: UUIDs in SSE framing (event ids, comments) are not job ids. A cost
/// answer on such a stream is a quote, a refusal stays rejected, a progress
/// event before a refusal still names its job, and only it, and a line cut
/// off by a resume is not read as one with the next stream's id line.
#[test]
fn sse_framing_is_not_scanned_for_job_ids() {
    let fake = fake_higgsfield();
    fake.set_stream(StreamMode::Sse { chunk: 64 });
    let guard = Arc::new(PreflightGuard::new());
    let p = provider_on(
        Arc::new(UuidEventIds(fake.clone())),
        token("t1"),
        guard.clone(),
        Arc::new(freecut_cloud::mcp::LateQueue::new()),
    );
    let outcome = p
        .quote(&image_op(), &run_tag(), &CancelToken::new())
        .unwrap();
    assert!(matches!(outcome, QuoteOutcome::Quote(_)), "{outcome:?}");
    assert!(!guard.is_blocked("generate_image"));

    let fake = paid_fake();
    fake.set_stream(StreamMode::Sse { chunk: 64 });
    fake.on_tool("generate_image", |_| {
        ToolReply::Error("NSFW content".into())
    });
    let c = McpClient::new(
        freecut_cloud::provider::higgsfield::ENDPOINT,
        Arc::new(UuidEventIds(fake.clone())),
        token("t1"),
        Arc::new(common::NoWait::default()),
        "0.1.0",
    );
    c.initialize(&CancelToken::new()).unwrap();
    let d = submit_disposition(&submit(&c).unwrap_err(), &image_args());
    assert!(matches!(d, SubmitError::Rejected(_)), "{d:?}");

    fake.send_before_response(vec![
        json!({"jsonrpc": "2.0", "method": "notifications/progress",
        "params": {"progressToken": 1, "progress": 1, "message": format!("queued {JOB}")}}),
    ]);
    let d = submit_disposition(&submit(&c).unwrap_err(), &image_args());
    assert!(
        matches!(&d, SubmitError::OutcomeUnknown { job_ids, .. } if job_ids == &[JOB]),
        "{d:?}"
    );
    assert_eq!(fake.calls_to("generate_image").len(), 2);

    let fake = paid_fake();
    fake.set_stream(StreamMode::SseCutInsideAnswer { chunk: 7 });
    let c = McpClient::new(
        freecut_cloud::provider::higgsfield::ENDPOINT,
        Arc::new(UuidEventIds(fake.clone())),
        token("t1"),
        Arc::new(common::NoWait::default()),
        "0.1.0",
    );
    c.initialize(&CancelToken::new()).unwrap();
    let out = c
        .call_paid(
            &allow::GENERATE_IMAGE,
            image_args(),
            &run_tag(),
            &CancelToken::new(),
        )
        .unwrap();
    assert_eq!(out.job_ids(), vec![PAID_JOB]);
}

fn fault_strategy() -> impl Strategy<Value = Fault> {
    prop_oneof![
        Just(Fault::Refuse),
        Just(Fault::DropAfterSend),
        Just(Fault::Status(500, "x".into())),
        Just(Fault::Status(502, "x".into())),
        Just(Fault::Status(503, "x".into())),
        Just(Fault::Status(429, "x".into())),
        Just(Fault::Status(400, "x".into())),
        Just(Fault::Status(404, "x".into())),
        Just(Fault::Garbage),
        Just(Fault::CutStream),
        Just(Fault::GarbageWith(format!(
            "{{\"jobs\":[{{\"job_id\":\"{JOB}\""
        ))),
        Just(Fault::CutStreamWith(format!("queued {JOB}, then"))),
        Just(Fault::ToolError("NSFW content".into())),
        Just(Fault::ToolError(format!(
            "Job {JOB} failed: model overloaded"
        ))),
        Just(Fault::ToolError(format!(
            "{{\"error\":\"busy\",\"job_id\":\"{JOB}\"}}"
        ))),
        Just(Fault::RpcErrorData(
            -32602,
            "Invalid params".into(),
            json!({"job_id": JOB})
        )),
        Just(Fault::RpcErrorData(
            -32603,
            "Internal error".into(),
            json!({"field": "x"})
        )),
        Just(Fault::RpcError(-32603, "Internal error".into())),
        Just(Fault::RpcError(-32000, "Server error".into())),
        Just(Fault::RpcError(-32602, "Invalid params".into())),
    ]
}

/// Faults that leave the answer to a request that reached the server lost
/// or unreadable: the run must end as an unknown outcome.
fn hides_the_outcome(fault: &Fault) -> bool {
    match fault {
        Fault::DropAfterSend
        | Fault::Garbage
        | Fault::CutStream
        | Fault::GarbageWith(_)
        | Fault::CutStreamWith(_) => true,
        Fault::Status(status, _) => *status >= 500,
        Fault::RpcError(code, _) | Fault::RpcErrorData(code, _, _) => {
            !freecut_cloud::error::REJECTING_RPC_CODES.contains(code)
        }
        // A readable refusal; one naming a job is checked by `names_job`.
        Fault::ToolError(_) => false,
        Fault::Refuse => false,
    }
}

/// The job id the fake's faults name.
const JOB: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";

/// Faults whose answer names [`JOB`].
fn names_job(fault: &Fault) -> bool {
    match fault {
        Fault::GarbageWith(text) | Fault::CutStreamWith(text) | Fault::ToolError(text) => {
            text.contains(JOB)
        }
        Fault::RpcErrorData(_, _, data) => data.to_string().contains(JOB),
        _ => false,
    }
}

proptest! {
    /// For any sequence of failures spread over the handshake and the paid
    /// request, with or without SSE, expired sessions or a rejected token:
    /// - at most one submission reaches the service (never resent);
    /// - none reached it: the run is "not sent";
    /// - one reached it: the run is never "not sent", and when its answer
    ///   was lost or unreadable the run is "outcome unknown" (review M4);
    /// - one reached it with no fault on its answer: it succeeded, or the
    ///   service refused it readably (401 or session 404): "rejected".
    #[test]
    fn a_paid_call_is_submitted_at_most_once(
        faults in proptest::collection::vec(fault_strategy(), 0..8),
        sse in any::<bool>(),
        expire in any::<bool>(),
        reject in any::<bool>(),
        sessions in any::<bool>(),
    ) {
        use SubmitError::*;
        let fake = paid_fake();
        // A stateless server, as Higgsfield's is (§15.1), or one with sessions.
        fake.set_sessions(sessions);
        if sse {
            fake.set_stream(StreamMode::Sse { chunk: 5 });
        }
        let c = mcp_client(&fake, SwapToken::new("t1", Some("t2")));
        let _ = c.initialize(&CancelToken::new());
        if expire {
            fake.expire_sessions();
        }
        if reject {
            fake.reject_token("t1");
            fake.accept_token("t2");
        }
        for f in faults {
            fake.push_fault(f);
        }
        let result = submit(&c);
        let reached: Vec<_> = fake
            .requests()
            .into_iter()
            .filter(|r| r.tool() == Some("generate_image"))
            .collect();
        prop_assert!(reached.len() <= 1, "{} submissions", reached.len());
        match (reached.first(), result) {
            (None, Ok(())) => prop_assert!(false, "succeeded without reaching the service"),
            (None, Err(e)) => {
                prop_assert!(matches!(submit_disposition(&e, &Value::Null), NotSent(_)), "{:?}", e);
            }
            (Some(rec), Ok(())) => prop_assert!(rec.fault.is_none(), "{:?}", rec.fault),
            (Some(rec), Err(e)) => {
                let d = submit_disposition(&e, &Value::Null);
                prop_assert!(!matches!(d, NotSent(_)), "reached but {:?}", d);
                match &rec.fault {
                    // An answer that named a job keeps it (review R4, R5).
                    Some(f) if names_job(f) => {
                        prop_assert!(
                            matches!(&d, OutcomeUnknown { job_ids, .. } if job_ids == &[JOB]),
                            "{:?} gave {:?}", f, d
                        );
                    }
                    Some(f) if hides_the_outcome(f) => {
                        prop_assert!(matches!(d, OutcomeUnknown { .. }), "{:?} gave {:?}", f, d);
                    }
                    Some(_) => {}
                    None => prop_assert!(matches!(d, Rejected(_)), "{:?}", d),
                }
            }
        }
    }
}

#[test]
fn the_recorded_challenge_fixture_is_what_the_fake_sends() {
    let fake = fake_higgsfield();
    let r = freecut_cloud::transport::HttpTransport::send(
        &fake,
        freecut_cloud::transport::HttpRequest::post_json(
            freecut_cloud::provider::higgsfield::ENDPOINT,
            b"{}".to_vec(),
            CallLimits::READ,
        ),
        &CancelToken::new(),
    )
    .unwrap();
    assert_eq!(
        r.header("www-authenticate"),
        Some(fixture("www-authenticate.txt").trim())
    );
    let _ = fixture_json("oauth-protected-resource-mcp.json");
}
