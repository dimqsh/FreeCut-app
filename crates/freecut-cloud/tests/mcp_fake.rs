//! The MCP client, provider and guard against the in-memory fake service:
//! handshake, SSE, session expiry, `401` and renewal, retries, the preflight
//! guard and the unknown-outcome rule. No sockets.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use std::sync::Arc;

use common::{
    SwapToken, fake_higgsfield, fixture, fixture_json, golden, image_op, mcp_client, provider,
    video_op,
};
use freecut_cloud::CancelToken;
use freecut_cloud::credits::Credits;
use freecut_cloud::error::{CloudError, NotAcceptedReason, SubmitError, submit_disposition};
use freecut_cloud::fake::{Fault, ResumeMode, StreamMode, ToolReply};
use freecut_cloud::guard::QuoteRequest;
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
        p.quote(&image_op(), &CancelToken::new()).unwrap(),
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
    let QuoteOutcome::Quote(q) = p.quote(&image_op(), &cancel).unwrap() else {
        panic!("expected a quote");
    };
    assert_eq!(q.credits, Credits::from_millis(250));
    assert_eq!(q.arguments, golden("get_cost-image-gpt_image_2_5.json"));
    assert_eq!(
        fake.calls_to("generate_image"),
        vec![golden("get_cost-image-gpt_image_2_5.json")]
    );

    let QuoteOutcome::Quote(q) = p.quote(&video_op(), &cancel).unwrap() else {
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
        let QuoteOutcome::Quote(q) = p.quote(&image_op(), &CancelToken::new()).unwrap() else {
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
    let QuoteOutcome::Quote(q) = p.quote(&image_op(), &CancelToken::new()).unwrap() else {
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
        p.quote(&Operation::Image(r), &CancelToken::new()).unwrap(),
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
        p.quote(&image_op(), &cancel),
        Err(CloudError::NotAccepted {
            reason: NotAcceptedReason::SessionExpired
        })
    );
    assert_eq!(fake.calls_to("generate_image").len(), 1);
    // The user's next Check cost works on the new session.
    assert!(matches!(
        p.quote(&image_op(), &cancel),
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
        p.quote(&image_op(), &cancel),
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
        let got = match p.quote(&image_op(), &cancel) {
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
        let QuoteOutcome::PossiblyStarted(started) = p.quote(&image_op(), &cancel).unwrap() else {
            panic!("expected possibly started for {answer}");
        };
        assert_eq!(started.tool, "generate_image");
        assert_eq!(
            started.arguments,
            golden("get_cost-image-gpt_image_2_5.json")
        );
        assert_eq!(
            p.quote(&image_op(), &cancel).unwrap(),
            QuoteOutcome::NotAvailable(Unavailable::PreflightStopped)
        );
        assert_eq!(fake.calls_to("generate_image").len(), 1, "{answer}");
        assert!(matches!(
            p.quote(&video_op(), &cancel).unwrap(),
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
    let QuoteOutcome::StartedRun(run) = p.quote(&image_op(), &cancel).unwrap() else {
        panic!("expected a started run");
    };
    assert_eq!(run.job_ids, vec!["5c6f0d1e-2b3a-4c4d-8e5f-6a7b8c9d0e1f"]);
    assert_eq!(run.tool, "generate_image");
    assert!(p.guard().is_blocked("generate_image"));
    // Exactly one submission per Check cost, and none after that.
    for _ in 0..3 {
        assert_eq!(
            p.quote(&image_op(), &cancel).unwrap(),
            QuoteOutcome::NotAvailable(Unavailable::PreflightStopped)
        );
    }
    assert_eq!(fake.calls_to("generate_image").len(), 1);
    // Other tools keep their checks.
    assert!(matches!(
        p.quote(&video_op(), &cancel).unwrap(),
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
        p.quote(&image_op(), &CancelToken::new()).unwrap(),
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
        p.quote(&image_op(), &CancelToken::new()),
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
        p.quote(&image_op(), &CancelToken::new()),
        Err(CloudError::ModelUnavailable { .. })
    ));
    let fake = fake_higgsfield();
    fake.on_tool("generate_video", |_| {
        ToolReply::Error("Insufficient credits".into())
    });
    let (p, _) = provider(&fake, token("t1"));
    assert_eq!(
        p.quote(&video_op(), &CancelToken::new()),
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
    c.call_tool(&allow::GENERATE_IMAGE, image_args(), &CancelToken::new())
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
            submit_disposition(&err),
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
        matches!(d, OutcomeUnknown(CloudError::NoAnswer)) && n == 1,
        "{d:?} {n}"
    );
    let (d, n) = at_call(Fault::Status(502, "bad gateway".into()));
    assert!(matches!(d, OutcomeUnknown(_)) && n == 1, "{d:?} {n}");
    let (d, n) = at_call(Fault::Status(429, "too many concurrent jobs".into()));
    assert!(
        matches!(d, Rejected(CloudError::Busy)) && n == 1,
        "{d:?} {n}"
    );
    let (d, n) = at_call(Fault::Garbage);
    assert!(
        matches!(d, OutcomeUnknown(CloudError::Protocol { .. })) && n == 1,
        "{d:?} {n}"
    );
    let (d, n) = at_call(Fault::CutStream);
    assert!(matches!(d, OutcomeUnknown(_)) && n == 1, "{d:?} {n}");
    let (d, n) = at_call(Fault::RpcError(-32603, "Internal error".into()));
    assert!(matches!(d, OutcomeUnknown(_)) && n == 1, "{d:?} {n}");
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
        submit_disposition(&err),
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
        submit_disposition(&err),
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
        matches!(submit_disposition(&err), NotSent(CloudError::NoAnswer)),
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
        let d = submit_disposition(&err);
        if rejected {
            assert!(matches!(d, Rejected(_)), "{code}: {d:?}");
        } else {
            assert!(matches!(d, OutcomeUnknown(_)), "{code}: {d:?}");
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
        submit_disposition(&err),
        OutcomeUnknown(CloudError::InsufficientCredits)
    );

    // And a cost check answered with such an error fails closed (M1).
    let fake = fake_higgsfield();
    fake.on_tool("generate_image", |_| {
        ToolReply::RpcError(-32603, "Internal error".into())
    });
    let (p, _) = provider(&fake, token("t1"));
    assert!(matches!(
        p.quote(&image_op(), &CancelToken::new()).unwrap(),
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
            matches!(submit_disposition(&err), SubmitError::OutcomeUnknown(_)),
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
        p.quote(&image_op(), &CancelToken::new()).unwrap(),
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
            matches!(submit_disposition(&err), SubmitError::OutcomeUnknown(_)),
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
        .call_tool(&allow::GENERATE_IMAGE, image_args(), &cancel)
        .unwrap_err();
    assert!(
        matches!(submit_disposition(&err), SubmitError::OutcomeUnknown(_)),
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
        c.call_tool(&allow::GENERATE_IMAGE, image_args(), &cancel)
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
        matches!(submit_disposition(&err), SubmitError::NotSent(_)),
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
            let _ = c.call_tool(tool, json!({"params": {"model": "m"}}), &CancelToken::new());
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
                let _ = c.check_cost(tool, &quote, &CancelToken::new());
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
        c.check_cost(&allow::MEDIA_CONFIRM, &quote, &CancelToken::new()),
        Err(McpError::Guard(_))
    ));
    let quote = QuoteRequest::new("generate_video", json!({"params": {}})).unwrap();
    assert!(matches!(
        c.check_cost(&allow::GENERATE_IMAGE, &quote, &CancelToken::new()),
        Err(McpError::Guard(_))
    ));
    assert!(fake.requests().is_empty());
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
        Just(Fault::RpcError(-32603, "Internal error".into())),
        Just(Fault::RpcError(-32000, "Server error".into())),
        Just(Fault::RpcError(-32602, "Invalid params".into())),
    ]
}

/// Faults that leave the answer to a request that reached the server lost
/// or unreadable: the run must end as an unknown outcome.
fn hides_the_outcome(fault: &Fault) -> bool {
    match fault {
        Fault::DropAfterSend | Fault::Garbage | Fault::CutStream => true,
        Fault::Status(status, _) => *status >= 500,
        Fault::RpcError(code, _) => !freecut_cloud::error::REJECTING_RPC_CODES.contains(code),
        Fault::Refuse => false,
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
    ) {
        use SubmitError::*;
        let fake = paid_fake();
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
                prop_assert!(matches!(submit_disposition(&e), NotSent(_)), "{:?}", e);
            }
            (Some(rec), Ok(())) => prop_assert!(rec.fault.is_none(), "{:?}", rec.fault),
            (Some(rec), Err(e)) => {
                let d = submit_disposition(&e);
                prop_assert!(!matches!(d, NotSent(_)), "reached but {:?}", d);
                match &rec.fault {
                    Some(f) if hides_the_outcome(f) => {
                        prop_assert!(matches!(d, OutcomeUnknown(_)), "{:?} gave {:?}", f, d);
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
