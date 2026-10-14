//! Sign-in against the recorded discovery documents, served by the fake:
//! discovery, registration, the authorize URL and callback, code exchange,
//! storage split over keychain entries, refresh (with rotation and
//! `invalid_grant`), device code, disconnect with revocation, and the MCP
//! client renewing a token through all of it. No sockets.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use std::sync::{Arc, Mutex};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use common::{fake_higgsfield, fixture_json, golden, provider};
use freecut_cloud::CancelToken;
use freecut_cloud::error::CloudError;
use freecut_cloud::fake::FakeService;
use freecut_cloud::mcp::{TokenPurpose, TokenSource};
use freecut_cloud::oauth::pkce::Pkce;
use freecut_cloud::oauth::{
    ClientRegistration, DevicePoll, OAuthClient, TokenManager, authorize_request, parse_callback,
};
use freecut_cloud::provider::GenerationProvider;
use freecut_cloud::provider::higgsfield::auth_policy;
use freecut_cloud::secret::{
    ChunkedSecretStore, Clock, Credential, MemoryEntryStore, ProviderId, Secret, SecretStore,
    SubjectHash, UnixSeconds,
};
use freecut_cloud::transport::Method;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

const PRM: &str = "https://mcp.higgsfield.ai/.well-known/oauth-protected-resource/mcp";
const CLERK_META: &str = "https://clerk.higgsfield.ai/.well-known/oauth-authorization-server";
const TOKEN: &str = "https://clerk.higgsfield.ai/oauth/token";
const REGISTER: &str = "https://clerk.higgsfield.ai/oauth/register";
const REVOKE: &str = "https://clerk.higgsfield.ai/oauth/token/revoke";
const DEVICE: &str = "https://clerk.higgsfield.ai/oauth/device_authorization";

#[derive(Debug)]
struct TestClock(Mutex<i64>);

impl Clock for TestClock {
    fn now(&self) -> UnixSeconds {
        UnixSeconds(*self.0.lock().unwrap())
    }
}

fn jwt(payload: &Value) -> String {
    format!(
        "eyJhbGciOiJSUzI1NiJ9.{}.c2ln",
        URL_SAFE_NO_PAD.encode(payload.to_string())
    )
}

/// The fake plus the recorded discovery documents.
fn signed_out_service() -> FakeService {
    let fake = fake_higgsfield();
    fake.serve_json(PRM, fixture_json("oauth-protected-resource-mcp.json"));
    fake.serve_json(
        CLERK_META,
        fixture_json("oauth-authorization-server-clerk.json"),
    );
    fake.on_route(Method::Post, REGISTER, |_| {
        (201, fixture_json("oauth-register-clerk-201.json"))
    });
    fake
}

fn token_answer(access: &str, refresh: Option<&str>, expires_in: u64) -> Value {
    let mut v = json!({
        "access_token": access,
        "token_type": "Bearer",
        "expires_in": expires_in,
        "scope": "openid email offline_access",
        "id_token": jwt(&json!({"sub": "user_2abc", "email": "someone@example.test"})),
    });
    if let Some(r) = refresh {
        v["refresh_token"] = json!(r);
    }
    v
}

#[test]
fn discovery_follows_the_resource_metadata_to_clerk() {
    let fake = signed_out_service();
    let oauth = OAuthClient::new(Arc::new(fake.clone()), auth_policy());
    let found = oauth.discover(None, &CancelToken::new()).unwrap();
    assert_eq!(found.server.issuer, "https://clerk.higgsfield.ai");
    assert_eq!(found.server.token_endpoint, TOKEN);
    assert_eq!(found.server.revocation_endpoint.as_deref(), Some(REVOKE));
    assert_eq!(
        found.server.device_authorization_endpoint.as_deref(),
        Some(DEVICE)
    );
    assert!(found.server.authorization_response_iss_parameter_supported);
    assert_eq!(fake.requests()[0].url, PRM);
}

#[test]
fn discovery_skips_a_server_without_metadata_and_refuses_foreign_ones() {
    let fake = signed_out_service();
    let mut prm = fixture_json("oauth-protected-resource-mcp.json");
    prm["authorization_servers"] = json!([
        "https://auth.example.test",
        "https://fnf-device-auth.higgsfield.ai",
        "https://clerk.higgsfield.ai"
    ]);
    fake.serve_json(PRM, prm);
    let oauth = OAuthClient::new(Arc::new(fake.clone()), auth_policy());
    let found = oauth.discover(None, &CancelToken::new()).unwrap();
    assert_eq!(found.server.issuer, "https://clerk.higgsfield.ai");
    assert!(
        fake.requests()
            .iter()
            .all(|r| !r.url.contains("example.test"))
    );

    let fake = signed_out_service();
    let mut prm = fixture_json("oauth-protected-resource-mcp.json");
    prm["resource"] = json!("https://mcp.higgsfield.ai/other");
    fake.serve_json(PRM, prm);
    let oauth = OAuthClient::new(Arc::new(fake), auth_policy());
    assert!(matches!(
        oauth.discover(None, &CancelToken::new()),
        Err(CloudError::SignInFailed { .. })
    ));
}

#[test]
fn registration_sends_the_golden_request_and_reads_the_recorded_answer() {
    let fake = signed_out_service();
    let oauth = OAuthClient::new(Arc::new(fake.clone()), auth_policy());
    let cancel = CancelToken::new();
    let server = oauth.discover(None, &cancel).unwrap().server;
    let reg = oauth.register(&server, &cancel).unwrap();
    assert_eq!(
        reg,
        ClientRegistration {
            client_id: "FAKE_CLIENT_ID".into(),
            issuer: "https://clerk.higgsfield.ai".into(),
            // The recorded answer echoes no device grant.
            device_grant: false,
        }
    );
    let sent: Vec<_> = fake
        .requests()
        .into_iter()
        .filter(|r| r.url == REGISTER)
        .collect();
    assert_eq!(sent.len(), 1);
    // Clerk advertises device sign-in, so the grant is requested.
    assert_eq!(sent[0].body, golden("registration-request.json"));
    assert_eq!(sent[0].bearer, None);
}

#[test]
fn an_echoed_device_grant_enables_sign_in_with_a_code() {
    let fake = signed_out_service();
    fake.on_route(Method::Post, REGISTER, |req| {
        let mut answer = fixture_json("oauth-register-clerk-201.json");
        answer["grant_types"] = req.body["grant_types"].clone();
        (201, answer)
    });
    let oauth = OAuthClient::new(Arc::new(fake), auth_policy());
    let cancel = CancelToken::new();
    let server = oauth.discover(None, &cancel).unwrap().server;
    assert!(oauth.register(&server, &cancel).unwrap().device_grant);
}

#[test]
fn a_refused_device_grant_falls_back_to_browser_only_registration() {
    let fake = signed_out_service();
    fake.on_route(Method::Post, REGISTER, |req| {
        let asks_device = req.body["grant_types"]
            .as_array()
            .is_some_and(|g| g.iter().any(|g| g.as_str().is_some_and(|g| g.ends_with("device_code"))));
        if asks_device {
            (400, json!({"error": "invalid_client_metadata", "error_description": "grant not allowed"}))
        } else {
            (201, fixture_json("oauth-register-clerk-201.json"))
        }
    });
    let oauth = OAuthClient::new(Arc::new(fake.clone()), auth_policy());
    let cancel = CancelToken::new();
    let server = oauth.discover(None, &cancel).unwrap().server;
    let reg = oauth.register(&server, &cancel).unwrap();
    assert!(!reg.device_grant);
    let sent: Vec<_> = fake
        .requests()
        .into_iter()
        .filter(|r| r.url == REGISTER)
        .collect();
    assert_eq!(sent.len(), 2);
    assert_eq!(
        sent[1].body,
        golden("registration-request-browser-only.json")
    );
}

#[test]
fn without_device_sign_in_only_the_browser_grants_are_requested() {
    let fake = signed_out_service();
    let mut meta = fixture_json("oauth-authorization-server-clerk.json");
    meta.as_object_mut()
        .unwrap()
        .remove("device_authorization_endpoint");
    fake.serve_json(CLERK_META, meta);
    let oauth = OAuthClient::new(Arc::new(fake.clone()), auth_policy());
    let cancel = CancelToken::new();
    let server = oauth.discover(None, &cancel).unwrap().server;
    oauth.register(&server, &cancel).unwrap();
    let sent: Vec<_> = fake
        .requests()
        .into_iter()
        .filter(|r| r.url == REGISTER)
        .collect();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0].body,
        golden("registration-request-browser-only.json")
    );
}

#[test]
fn a_refused_registration_shows_the_services_message() {
    let fake = signed_out_service();
    fake.on_route(Method::Post, REGISTER, |_| {
        (400, json!({"error": "invalid_redirect_uri", "error_description": "Loopback\nredirects are not allowed"}))
    });
    let oauth = OAuthClient::new(Arc::new(fake), auth_policy());
    let cancel = CancelToken::new();
    let server = oauth.discover(None, &cancel).unwrap().server;
    assert_eq!(
        oauth.register(&server, &cancel),
        Err(CloudError::SignInFailed {
            message: "Loopback redirects are not allowed".into()
        })
    );
}

/// Discovery → registration → browser → callback → exchange → storage,
/// then the stored sign-in drives the MCP client.
#[test]
fn a_full_browser_sign_in() {
    let fake = signed_out_service();
    let long_access = format!("at-{}", "x".repeat(3_000));
    let access_for_route = long_access.clone();
    let seen_exchange = Arc::new(Mutex::new(None));
    let seen = seen_exchange.clone();
    fake.on_route(Method::Post, TOKEN, move |req| {
        *seen.lock().unwrap() = Some(req.body.clone());
        (200, token_answer(&access_for_route, Some("rt-1"), 3600))
    });
    fake.accept_token(&long_access);

    let cancel = CancelToken::new();
    let oauth = OAuthClient::new(Arc::new(fake.clone()), auth_policy());
    let server = oauth.discover(None, &cancel).unwrap().server;
    let reg = oauth.register(&server, &cancel).unwrap();
    let pkce = Pkce::generate().unwrap();
    let challenge = pkce.challenge().to_owned();
    let req = authorize_request(
        &server,
        &reg.client_id,
        50123,
        oauth.policy(),
        pkce,
        Secret::new("state-1"),
    )
    .unwrap();
    assert!(req.url.starts_with(
        "https://clerk.higgsfield.ai/oauth/authorize?response_type=code&client_id=FAKE_CLIENT_ID"
    ));
    assert!(req.url.contains(&format!("code_challenge={challenge}")));

    // The browser comes back to the app's loopback listener.
    let code = parse_callback(
        "/callback?code=c0de&state=state-1&iss=https%3A%2F%2Fclerk.higgsfield.ai",
        &req.state,
        &server,
    )
    .unwrap();
    let tokens = oauth
        .exchange_code(
            &server,
            &reg.client_id,
            &code,
            &req.redirect_uri,
            req.pkce.verifier(),
            &cancel,
        )
        .unwrap();
    let body = seen_exchange.lock().unwrap().clone().unwrap();
    assert_eq!(body["grant_type"], "authorization_code");
    assert_eq!(body["code"], "c0de");
    assert_eq!(body["redirect_uri"], "http://127.0.0.1:50123/callback");
    assert_eq!(body["client_id"], "FAKE_CLIENT_ID");
    assert_eq!(body["code_verifier"], req.pkce.verifier().expose());
    assert_eq!(body["resource"], "https://mcp.higgsfield.ai/mcp");

    let store = Arc::new(ChunkedSecretStore::new(
        MemoryEntryStore::with_windows_limit(),
    ));
    let clock = Arc::new(TestClock(Mutex::new(1_000)));
    let manager = Arc::new(TokenManager::new(
        ProviderId::HIGGSFIELD,
        store.clone(),
        oauth,
        clock,
    ));
    manager.set_server(server);
    let stored = manager.store_sign_in(tokens, &reg).unwrap();
    assert_eq!(stored.expires_at, Some(UnixSeconds(4_600)));
    assert_eq!(stored.subject_hash, Some(SubjectHash::of("user_2abc")));
    assert_eq!(
        stored.account_label.as_deref(),
        Some("someone@example.test")
    );
    // Over Windows' limit, so split; the round trip is exact.
    assert!(
        store
            .entries()
            .accounts()
            .iter()
            .any(|a| a == "higgsfield/access.1.2")
    );
    assert_eq!(
        store.get(ProviderId::HIGGSFIELD).unwrap(),
        Some(Credential::OAuth(stored))
    );

    let (p, _) = provider(&fake, manager);
    assert!(p.connect(&cancel).is_ok());
    assert_eq!(
        fake.requests().last().unwrap().bearer.as_deref(),
        Some(long_access.as_str())
    );
}

fn signed_in(fake: &FakeService, now: i64, expires_in: u64) -> (Arc<TokenManager>, Arc<TestClock>) {
    let cancel = CancelToken::new();
    let oauth = OAuthClient::new(Arc::new(fake.clone()), auth_policy());
    let server = oauth.discover(None, &cancel).unwrap().server;
    let store = Arc::new(ChunkedSecretStore::new(MemoryEntryStore::new()));
    let clock = Arc::new(TestClock(Mutex::new(now)));
    let manager = Arc::new(TokenManager::new(
        ProviderId::HIGGSFIELD,
        store,
        oauth,
        clock.clone(),
    ));
    manager.set_server(server);
    let tokens = freecut_cloud::oauth::token::parse_token_response(
        200,
        token_answer("at-1", Some("rt-1"), expires_in)
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    manager
        .store_sign_in(
            tokens,
            &ClientRegistration {
                client_id: "cid".into(),
                issuer: "https://clerk.higgsfield.ai".into(),
                device_grant: false,
            },
        )
        .unwrap();
    (manager, clock)
}

#[test]
fn tokens_refresh_when_under_a_minute_remains_and_rotation_is_stored() {
    let fake = signed_out_service();
    let refreshes = Arc::new(Mutex::new(Vec::new()));
    let log = refreshes.clone();
    fake.on_route(Method::Post, TOKEN, move |req| {
        log.lock().unwrap().push(req.body.clone());
        (200, token_answer("at-2", Some("rt-2"), 3600))
    });
    let (manager, clock) = signed_in(&fake, 0, 3600);
    let cancel = CancelToken::new();

    *clock.0.lock().unwrap() = 3600 - 61;
    assert_eq!(
        manager
            .bearer(TokenPurpose::Ordinary, &cancel)
            .unwrap()
            .expose(),
        "at-1"
    );
    // Under five minutes: a paid submission renews first.
    assert_eq!(
        manager
            .bearer(TokenPurpose::BeforePaid, &cancel)
            .unwrap()
            .expose(),
        "at-2"
    );
    let sent = refreshes.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["grant_type"], "refresh_token");
    assert_eq!(sent[0]["refresh_token"], "rt-1");
    assert_eq!(sent[0]["resource"], "https://mcp.higgsfield.ai/mcp");
    let Credential::OAuth(c) = manager.credential().unwrap() else {
        panic!()
    };
    assert_eq!(c.refresh.unwrap().expose(), "rt-2");
    // The account tag survives refreshes.
    assert_eq!(c.subject_hash, Some(SubjectHash::of("user_2abc")));
}

#[test]
fn invalid_grant_is_sign_in_expired_and_deletes_nothing() {
    let fake = signed_out_service();
    fake.on_route(Method::Post, TOKEN, |_| {
        (400, json!({"error": "invalid_grant"}))
    });
    let (manager, clock) = signed_in(&fake, 0, 100);
    *clock.0.lock().unwrap() = 200;
    assert_eq!(
        manager.bearer(TokenPurpose::Ordinary, &CancelToken::new()),
        Err(CloudError::SignInExpired)
    );
    assert!(manager.credential().is_ok());
}

#[test]
fn an_offline_refresh_still_uses_a_token_that_is_valid_for_a_moment() {
    let fake = signed_out_service();
    let (manager, clock) = signed_in(&fake, 0, 100);
    *clock.0.lock().unwrap() = 70;
    fake.push_fault(freecut_cloud::fake::Fault::Refuse);
    assert_eq!(
        manager
            .bearer(TokenPurpose::Ordinary, &CancelToken::new())
            .unwrap()
            .expose(),
        "at-1"
    );
}

#[test]
fn the_mcp_client_renews_through_the_token_manager_after_a_401() {
    let fake = signed_out_service();
    fake.on_route(Method::Post, TOKEN, |_| {
        (200, token_answer("at-2", None, 3600))
    });
    let (manager, _) = signed_in(&fake, 0, 3600);
    fake.accept_token("at-1");
    let (p, _) = provider(&fake, manager.clone());
    let cancel = CancelToken::new();
    p.connect(&cancel).unwrap();
    fake.reject_token("at-1");
    fake.accept_token("at-2");
    p.catalogue(&cancel).unwrap();
    let Credential::OAuth(c) = manager.credential().unwrap() else {
        panic!()
    };
    assert_eq!(c.access.expose(), "at-2");
    // No new refresh token in the answer: the old one is kept.
    assert_eq!(c.refresh.unwrap().expose(), "rt-1");
}

#[test]
fn disconnect_revokes_both_tokens_then_deletes() {
    let fake = signed_out_service();
    let revoked = Arc::new(Mutex::new(Vec::new()));
    let log = revoked.clone();
    fake.on_route(Method::Post, REVOKE, move |req| {
        log.lock().unwrap().push(
            req.body["token_type_hint"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        );
        (200, json!({}))
    });
    let (manager, _) = signed_in(&fake, 0, 3600);
    let report = manager.disconnect(&CancelToken::new()).unwrap();
    assert!(report.revoked);
    assert_eq!(
        *revoked.lock().unwrap(),
        vec!["refresh_token", "access_token"]
    );
    assert_eq!(manager.credential(), Err(CloudError::NotConnected));
}

#[test]
fn disconnect_deletes_even_when_revocation_fails() {
    let fake = signed_out_service();
    fake.on_route(Method::Post, REVOKE, |_| (503, json!({})));
    let (manager, _) = signed_in(&fake, 0, 3600);
    assert!(!manager.disconnect(&CancelToken::new()).unwrap().revoked);
    assert_eq!(manager.credential(), Err(CloudError::NotConnected));
}

#[test]
fn a_pasted_token_is_used_as_is_and_cannot_be_renewed() {
    let fake = signed_out_service();
    let oauth = OAuthClient::new(Arc::new(fake.clone()), auth_policy());
    let store: Arc<dyn SecretStore> = Arc::new(freecut_cloud::secret::SessionSecretStore::new());
    let manager = TokenManager::new(
        ProviderId::HIGGSFIELD,
        store,
        oauth,
        Arc::new(TestClock(Mutex::new(0))),
    );
    manager
        .store_pasted(freecut_cloud::secret::parse_pasted_token(" pasted-1 \n").unwrap())
        .unwrap();
    let cancel = CancelToken::new();
    assert_eq!(
        manager
            .bearer(TokenPurpose::BeforePaid, &cancel)
            .unwrap()
            .expose(),
        "pasted-1"
    );
    assert_eq!(manager.renew(&cancel), Err(CloudError::SignInExpired));
    assert!(fake.requests().is_empty());
}

#[test]
fn device_sign_in_polls_until_done_and_slows_down_when_asked() {
    let fake = signed_out_service();
    fake.on_route(Method::Post, DEVICE, |_| {
        (
            200,
            json!({
                "device_code": "dc-1", "user_code": "WDJB-MJHT",
                "verification_uri": "https://clerk.higgsfield.ai/device",
                "verification_uri_complete": "https://evil.example.test/device?c=WDJB-MJHT",
                "expires_in": 600, "interval": 5
            }),
        )
    });
    let polls = Arc::new(Mutex::new(0));
    let count = polls.clone();
    fake.on_route(Method::Post, TOKEN, move |req| {
        assert_eq!(
            req.body["grant_type"],
            "urn:ietf:params:oauth:grant-type:device_code"
        );
        let mut n = count.lock().unwrap();
        *n += 1;
        match *n {
            1 => (400, json!({"error": "authorization_pending"})),
            2 => (400, json!({"error": "slow_down"})),
            _ => (200, token_answer("at-d", Some("rt-d"), 3600)),
        }
    });
    let cancel = CancelToken::new();
    let oauth = OAuthClient::new(Arc::new(fake), auth_policy());
    let server = oauth.discover(None, &cancel).unwrap().server;
    let device = oauth.start_device(&server, "cid", &cancel).unwrap();
    assert_eq!(device.user_code, "WDJB-MJHT");
    // A completion link off the service's domain is dropped.
    assert_eq!(device.verification_uri_complete, None);
    let mut interval = device.interval;
    let tokens = loop {
        match oauth
            .poll_device(&server, "cid", &device, interval, &cancel)
            .unwrap()
        {
            DevicePoll::Pending { interval: next } => interval = next,
            DevicePoll::Done(tokens) => break tokens,
        }
    };
    assert_eq!(interval, 10);
    assert_eq!(tokens.access.expose(), "at-d");
}

#[test]
fn device_sign_in_is_not_offered_without_the_endpoint() {
    let fake = signed_out_service();
    let mut meta = fixture_json("oauth-authorization-server-clerk.json");
    meta.as_object_mut()
        .unwrap()
        .remove("device_authorization_endpoint");
    fake.serve_json(CLERK_META, meta);
    let oauth = OAuthClient::new(Arc::new(fake), auth_policy());
    let cancel = CancelToken::new();
    let server = oauth.discover(None, &cancel).unwrap().server;
    assert!(matches!(
        oauth.start_device(&server, "cid", &cancel),
        Err(CloudError::NotOffered { .. })
    ));
}
