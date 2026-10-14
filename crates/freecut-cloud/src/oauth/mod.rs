//! Sign-in with OAuth 2.1 (architecture §5).
//!
//! The flow, with the app's part in brackets:
//! 1. [`OAuthClient::discover`] finds the authorization server;
//! 2. [`OAuthClient::register`] registers FreeCut once per installation
//!    (public client, loopback redirect, no secret);
//! 3. [`authorize_request`] builds the URL with PKCE and `state`
//!    [the app binds `127.0.0.1:0`, opens the URL in the system browser and
//!    accepts one request];
//! 4. [`parse_callback`] checks that request and yields the code;
//! 5. [`OAuthClient::exchange_code`] trades it for tokens, and
//!    [`TokenManager::store_sign_in`] stores them.
//!
//! [`TokenManager`] then supplies tokens to the MCP client, refreshing them
//! as §5.6 says, and revokes them on disconnect. Device code (§5.4) is
//! offered when the server advertises it.

pub mod discovery;
pub mod pkce;
pub mod token;

use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use freecut_media::cancel::CancelToken;
use serde_json::{Value, json};
use url::Url;

use crate::error::{CloudError, quoted};
use crate::mcp::{TokenPurpose, TokenSource};
use crate::secret::{
    Clock, Credential, OAuthCredential, ProviderId, Secret, SecretError, SecretStore,
};
use crate::transport::{CallLimits, HttpRequest, HttpTransport, SendState, read_body};

use self::discovery::{
    AuthPolicy, AuthServer, DiscoveryError, ProtectedResource, auth_server_metadata_urls,
    parse_auth_server, parse_protected_resource, protected_resource_urls,
};
use self::pkce::Pkce;
use self::token::{TokenError, TokenSet, parse_token_response};

const MAX_OAUTH_BODY: usize = 1024 * 1024;

/// The loopback host FreeCut's redirect uses (IPv4 only, RFC 8252 §7.3).
pub const LOOPBACK_HOST: &str = "127.0.0.1";
/// The redirect path.
pub const CALLBACK_PATH: &str = "/callback";
/// The redirect URI FreeCut registers; each sign-in uses its own port.
pub const REGISTERED_REDIRECT_URI: &str = "http://127.0.0.1/callback";
/// The device grant type (RFC 8628).
pub const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// The redirect URI for one sign-in on `port`.
pub fn loopback_redirect_uri(port: u16) -> String {
    format!("http://{LOOPBACK_HOST}:{port}{CALLBACK_PATH}")
}

/// Discovered sign-in metadata for one service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceAuth {
    /// The protected resource (the MCP endpoint).
    pub resource: ProtectedResource,
    /// The authorization server FreeCut will use.
    pub server: AuthServer,
}

/// FreeCut's registration with an authorization server. `Debug` hides the
/// client id, which is kept out of logs.
#[derive(Clone, PartialEq, Eq)]
pub struct ClientRegistration {
    /// The client id (not secret; kept in the preferences).
    pub client_id: String,
    /// The issuer it was registered with.
    pub issuer: String,
    /// Whether the registration includes the device grant, so "Use a code
    /// instead" (§5.4) can be offered.
    pub device_grant: bool,
}

impl fmt::Debug for ClientRegistration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientRegistration")
            .field("client_id", &"<redacted>")
            .field("issuer", &self.issuer)
            .field("device_grant", &self.device_grant)
            .finish()
    }
}

/// A browser sign-in in progress: the URL to open and what the callback
/// and code exchange need. `Debug` hides the URL's query, which carries the
/// `state` and the client id (review SEC-5).
#[derive(Clone)]
pub struct AuthorizeRequest {
    /// The URL to open in the system browser.
    pub url: String,
    /// The redirect URI used, needed again for the code exchange.
    pub redirect_uri: String,
    /// The expected `state`.
    pub state: Secret,
    /// The PKCE pair.
    pub pkce: Pkce,
}

impl fmt::Debug for AuthorizeRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let base = self.url.split('?').next().unwrap_or_default();
        f.debug_struct("AuthorizeRequest")
            .field("url", &format_args!("{base}?<redacted>"))
            .field("redirect_uri", &self.redirect_uri)
            .field("state", &self.state)
            .field("pkce", &self.pkce)
            .finish()
    }
}

/// Why a loopback request was not a valid sign-in callback.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CallbackError {
    /// Another path (the listener keeps waiting).
    #[error("not the sign-in callback")]
    NotTheCallback,
    /// `state` missing or different: possibly a forged request.
    #[error("the sign-in answer does not belong to this sign-in")]
    StateMismatch,
    /// The answer names another issuer (RFC 9207).
    #[error("the sign-in answer comes from another server")]
    IssuerMismatch,
    /// No `code`.
    #[error("the sign-in answer has no code")]
    MissingCode,
    /// The server returned an error, typically `access_denied`.
    #[error("sign-in was not completed: {error}")]
    Denied {
        /// The `error` code.
        error: String,
        /// `error_description` (quoted text).
        description: String,
    },
}

/// Builds the authorize URL (§5.3 step 2).
///
/// # Errors
/// [`CloudError::SignInFailed`] when the authorization endpoint is not a URL.
pub fn authorize_request(
    server: &AuthServer,
    client_id: &str,
    port: u16,
    policy: &AuthPolicy,
    pkce: Pkce,
    state: Secret,
) -> Result<AuthorizeRequest, CloudError> {
    let mut url =
        Url::parse(&server.authorization_endpoint).map_err(|_| CloudError::SignInFailed {
            message: "the sign-in address is not a URL".into(),
        })?;
    let redirect_uri = loopback_redirect_uri(port);
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("scope", &policy.scopes.join(" "))
        .append_pair("code_challenge", pkce.challenge())
        .append_pair("code_challenge_method", Pkce::METHOD)
        .append_pair("state", state.expose())
        .append_pair("resource", &policy.resource);
    Ok(AuthorizeRequest {
        url: url.into(),
        redirect_uri,
        state,
        pkce,
    })
}

/// Checks the loopback request (`/callback?code=…&state=…`) and returns the
/// code (§5.3 step 3). `target` is the HTTP request target.
///
/// # Errors
/// [`CallbackError`] for anything but a matching answer with a code.
pub fn parse_callback(
    target: &str,
    expected_state: &Secret,
    server: &AuthServer,
) -> Result<Secret, CallbackError> {
    // Origin-form only: anything else could move the host before the path
    // check (`@evil.example/callback`).
    if !target.starts_with('/') || target.starts_with("//") {
        return Err(CallbackError::NotTheCallback);
    }
    let url = Url::parse(&format!("http://{LOOPBACK_HOST}{target}"))
        .map_err(|_| CallbackError::NotTheCallback)?;
    if url.host_str() != Some(LOOPBACK_HOST) {
        return Err(CallbackError::NotTheCallback);
    }
    if url.path() != CALLBACK_PATH {
        return Err(CallbackError::NotTheCallback);
    }
    let mut state = None;
    let mut code = None;
    let mut iss = None;
    let mut error = None;
    let mut description = String::new();
    for (k, v) in url.query_pairs() {
        match k.as_ref() {
            "state" => state = Some(Secret::new(v.into_owned())),
            "code" => code = Some(Secret::new(v.into_owned())),
            "iss" => iss = Some(v.into_owned()),
            "error" => error = Some(v.into_owned()),
            "error_description" => description = quoted(&v),
            _ => {}
        }
    }
    if state.as_ref() != Some(expected_state) {
        return Err(CallbackError::StateMismatch);
    }
    match (&iss, server.authorization_response_iss_parameter_supported) {
        // RFC 9207: the exact issuer string (review SEC-7).
        (Some(iss), _) if *iss != server.issuer => {
            return Err(CallbackError::IssuerMismatch);
        }
        (None, true) => return Err(CallbackError::IssuerMismatch),
        _ => {}
    }
    if let Some(error) = error {
        return Err(CallbackError::Denied { error, description });
    }
    code.filter(|c| !c.is_empty())
        .ok_or(CallbackError::MissingCode)
}

/// The registration request (§5.2): a public native client with a loopback
/// redirect and no secret. The device grant is added only when the server
/// advertises device sign-in (§5.4).
pub fn registration_body(policy: &AuthPolicy, device_grant: bool) -> Value {
    let mut grants = vec!["authorization_code", "refresh_token"];
    if device_grant {
        grants.push(DEVICE_GRANT);
    }
    json!({
        "client_name": "FreeCut",
        "redirect_uris": [REGISTERED_REDIRECT_URI],
        "grant_types": grants,
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
        "application_type": "native",
        "scope": policy.scopes.join(" "),
    })
}

/// Device authorization (§5.4): what to show the user.
#[derive(Debug, Clone)]
pub struct DeviceAuthorization {
    /// The device code, polled with.
    pub device_code: Secret,
    /// The short code the user types.
    pub user_code: String,
    /// Where the user types it (HTTPS on the service's domain).
    pub verification_uri: String,
    /// The same with the code filled in, if offered.
    pub verification_uri_complete: Option<String>,
    /// Seconds until the codes expire.
    pub expires_in: u64,
    /// Seconds between polls.
    pub interval: u64,
}

/// One device-flow poll.
#[derive(Debug, Clone)]
pub enum DevicePoll {
    /// Not finished: poll again after `interval` seconds.
    Pending {
        /// The (possibly increased) interval.
        interval: u64,
    },
    /// Signed in.
    Done(TokenSet),
}

/// The HTTP side of sign-in. Every call is a user action or a refresh.
pub struct OAuthClient {
    transport: Arc<dyn HttpTransport>,
    policy: AuthPolicy,
}

impl fmt::Debug for OAuthClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuthClient")
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

fn discovery_failed(e: DiscoveryError) -> CloudError {
    CloudError::SignInFailed {
        message: e.to_string(),
    }
}

fn transport_failed(send: SendState) -> CloudError {
    match send {
        SendState::NotSent => CloudError::Offline,
        SendState::MaybeSent => CloudError::NoAnswer,
    }
}

impl OAuthClient {
    /// A client for the service `policy` describes.
    pub fn new(transport: Arc<dyn HttpTransport>, policy: AuthPolicy) -> OAuthClient {
        OAuthClient { transport, policy }
    }

    /// The policy.
    pub fn policy(&self) -> &AuthPolicy {
        &self.policy
    }

    /// Sends one request; returns the status and body.
    fn fetch(
        &self,
        request: HttpRequest,
        cancel: &CancelToken,
    ) -> Result<(u16, Vec<u8>), CloudError> {
        tracing::trace!(target: crate::mcp::WIRE_TARGET, url = %request.url, body = %crate::redact::body(&request.body), "oauth request");
        let mut response = self
            .transport
            .send(request, cancel)
            .map_err(|e| transport_failed(e.send))?;
        let body = read_body(&mut response.body, cancel, MAX_OAUTH_BODY).map_err(|e| match e {
            crate::transport::BodyError::Cancelled => CloudError::Cancelled,
            other => CloudError::Protocol {
                detail: other.to_string(),
            },
        })?;
        tracing::trace!(target: crate::mcp::WIRE_TARGET, status = response.status, body = %crate::redact::body(&body), "oauth answer");
        Ok((response.status, body))
    }

    /// `GET` JSON metadata: `None` for 404/410.
    fn get_metadata(&self, url: &str, cancel: &CancelToken) -> Result<Option<Vec<u8>>, CloudError> {
        let (status, body) = self.fetch(
            HttpRequest::get(url, CallLimits::OAUTH).header("accept", "application/json"),
            cancel,
        )?;
        match status {
            200 => Ok(Some(body)),
            404 | 410 => Ok(None),
            s => Err(CloudError::SignInFailed {
                message: format!("sign-in metadata answered HTTP {s}"),
            }),
        }
    }

    /// Finds the protected-resource metadata and the first usable
    /// authorization server it lists (§5.1). A `resource_metadata` URL from a
    /// `401` challenge is tried first when it is on the service's domain.
    ///
    /// # Errors
    /// [`CloudError`] when nothing usable is found.
    pub fn discover(
        &self,
        challenge_url: Option<&str>,
        cancel: &CancelToken,
    ) -> Result<ServiceAuth, CloudError> {
        let mut candidates = Vec::new();
        if let Some(url) = challenge_url
            && self.policy.trusted_url(url).is_ok()
        {
            candidates.push(url.to_owned());
        }
        candidates
            .extend(protected_resource_urls(&self.policy.resource).map_err(discovery_failed)?);
        let mut resource = None;
        for url in &candidates {
            if let Some(body) = self.get_metadata(url, cancel)? {
                resource =
                    Some(parse_protected_resource(&body, &self.policy).map_err(discovery_failed)?);
                break;
            }
        }
        let resource = resource.ok_or_else(|| CloudError::SignInFailed {
            message: "the service publishes no sign-in metadata".into(),
        })?;
        for issuer in &resource.authorization_servers {
            if self.policy.trusted_url(issuer).is_err() {
                tracing::debug!(%issuer, "skipped an authorization server off the service's domain");
                continue;
            }
            for url in auth_server_metadata_urls(issuer).map_err(discovery_failed)? {
                let Some(body) = self.get_metadata(&url, cancel)? else {
                    continue;
                };
                match parse_auth_server(&body, issuer, &self.policy) {
                    Ok(server) => return Ok(ServiceAuth { resource, server }),
                    Err(e) => {
                        tracing::debug!(%issuer, error = %e, "skipped an authorization server");
                        break;
                    }
                }
            }
        }
        Err(discovery_failed(DiscoveryError::NoUsableServer))
    }

    /// Registers FreeCut (RFC 7591, §5.2).
    ///
    /// # Errors
    /// [`CloudError::NotOffered`] without a registration endpoint;
    /// [`CloudError::SignInFailed`] with the service's message when refused.
    pub fn register(
        &self,
        server: &AuthServer,
        cancel: &CancelToken,
    ) -> Result<ClientRegistration, CloudError> {
        let device = server.device_authorization_endpoint.is_some()
            && server
                .grant_types_supported
                .iter()
                .any(|g| g == DEVICE_GRANT);
        if device {
            match self.register_with(server, true, cancel) {
                Ok(registration) => return Ok(registration),
                // Browser sign-in stays the main path: a server that refuses
                // the device grant gets the plain registration instead.
                Err(CloudError::SignInFailed { message }) => {
                    tracing::debug!(%message, "registration with the device grant refused");
                }
                Err(other) => return Err(other),
            }
        }
        self.register_with(server, false, cancel)
    }

    fn register_with(
        &self,
        server: &AuthServer,
        device_grant: bool,
        cancel: &CancelToken,
    ) -> Result<ClientRegistration, CloudError> {
        let endpoint =
            server
                .registration_endpoint
                .as_deref()
                .ok_or_else(|| CloudError::NotOffered {
                    what: "client registration".into(),
                })?;
        let body = registration_body(&self.policy, device_grant)
            .to_string()
            .into_bytes();
        let (status, answer) = self.fetch(
            HttpRequest::post_json(endpoint, body, CallLimits::OAUTH)
                .header("accept", "application/json"),
            cancel,
        )?;
        let value: Value = serde_json::from_slice(&answer).unwrap_or(Value::Null);
        if !(200..300).contains(&status) {
            let message = value
                .get("error_description")
                .or_else(|| value.get("error"))
                .and_then(Value::as_str)
                .map_or_else(|| format!("registration refused (HTTP {status})"), quoted);
            return Err(CloudError::SignInFailed { message });
        }
        let client_id = value
            .get("client_id")
            .and_then(Value::as_str)
            .filter(|id| {
                !id.is_empty() && id.len() <= 255 && id.bytes().all(|b| b.is_ascii_graphic())
            })
            .ok_or_else(|| CloudError::Protocol {
                detail: "registration answer without a usable client_id".into(),
            })?;
        // The service echoes the method; anything but a public client would
        // need a secret FreeCut must not hold. (`client_id_issued_at` is not
        // read: the service sends milliseconds where RFC 7591 says seconds.)
        if let Some(method) = value
            .get("token_endpoint_auth_method")
            .and_then(Value::as_str)
            && method != "none"
        {
            return Err(CloudError::SignInFailed {
                message: format!("registered as a {method} client, which needs a secret"),
            });
        }
        // A server may drop grants it will not allow; trust its echo.
        let granted_device = device_grant
            && value
                .get("grant_types")
                .and_then(Value::as_array)
                .is_none_or(|g| g.iter().any(|g| g.as_str() == Some(DEVICE_GRANT)));
        Ok(ClientRegistration {
            client_id: client_id.to_owned(),
            issuer: server.issuer.clone(),
            device_grant: granted_device,
        })
    }

    fn token_call(
        &self,
        server: &AuthServer,
        pairs: &[(&str, &str)],
        cancel: &CancelToken,
    ) -> Result<Result<TokenSet, TokenError>, CloudError> {
        let (status, mut body) = self.fetch(
            HttpRequest::post_form(&server.token_endpoint, pairs, CallLimits::OAUTH)
                .header("accept", "application/json"),
            cancel,
        )?;
        let tokens = parse_token_response(status, &body);
        // The raw answer holds the tokens too (review SEC-8).
        body.fill(0);
        Ok(tokens)
    }

    /// Exchanges an authorization code (§5.3 step 4).
    ///
    /// # Errors
    /// [`CloudError`]; a refused code is [`CloudError::SignInFailed`].
    pub fn exchange_code(
        &self,
        server: &AuthServer,
        client_id: &str,
        code: &Secret,
        redirect_uri: &str,
        verifier: &Secret,
        cancel: &CancelToken,
    ) -> Result<TokenSet, CloudError> {
        self.token_call(
            server,
            &[
                ("grant_type", "authorization_code"),
                ("code", code.expose()),
                ("redirect_uri", redirect_uri),
                ("client_id", client_id),
                ("code_verifier", verifier.expose()),
                ("resource", &self.policy.resource),
            ],
            cancel,
        )?
        .map_err(|e| CloudError::SignInFailed {
            message: e.to_string(),
        })
    }

    /// Refreshes tokens (§5.6).
    ///
    /// # Errors
    /// [`CloudError::SignInExpired`] for `invalid_grant`; other failures as
    /// classified.
    pub fn refresh(
        &self,
        server: &AuthServer,
        client_id: &str,
        refresh: &Secret,
        cancel: &CancelToken,
    ) -> Result<TokenSet, CloudError> {
        self.token_call(
            server,
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh.expose()),
                ("client_id", client_id),
                ("resource", &self.policy.resource),
            ],
            cancel,
        )?
        .map_err(|e| match e {
            TokenError::InvalidGrant => CloudError::SignInExpired,
            other => CloudError::SignInFailed {
                message: other.to_string(),
            },
        })
    }

    /// Starts a device sign-in (§5.4). The client must have been registered
    /// with the device grant.
    ///
    /// # Errors
    /// [`CloudError::NotOffered`] when the server has no device endpoint.
    pub fn start_device(
        &self,
        server: &AuthServer,
        client_id: &str,
        cancel: &CancelToken,
    ) -> Result<DeviceAuthorization, CloudError> {
        let endpoint = server
            .device_authorization_endpoint
            .as_deref()
            .ok_or_else(|| CloudError::NotOffered {
                what: "sign-in with a code".into(),
            })?;
        let scope = self.policy.scopes.join(" ");
        let (status, body) = self.fetch(
            HttpRequest::post_form(
                endpoint,
                &[
                    ("client_id", client_id),
                    ("scope", &scope),
                    ("resource", &self.policy.resource),
                ],
                CallLimits::OAUTH,
            ),
            cancel,
        )?;
        let mut body = body;
        let value: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        // The raw answer holds the device code (review SEC-8).
        body.fill(0);
        if status != 200 {
            let message = value
                .get("error_description")
                .or_else(|| value.get("error"))
                .and_then(Value::as_str)
                .map_or_else(|| format!("device sign-in refused (HTTP {status})"), quoted);
            return Err(CloudError::SignInFailed { message });
        }
        let text = |k: &str| value.get(k).and_then(Value::as_str).map(str::to_owned);
        let number = |k: &str| value.get(k).and_then(Value::as_u64);
        let malformed = || CloudError::Protocol {
            detail: "device authorization answer is incomplete".into(),
        };
        let verification_uri = text("verification_uri").ok_or_else(malformed)?;
        self.policy
            .trusted_url(&verification_uri)
            .map_err(discovery_failed)?;
        let verification_uri_complete =
            text("verification_uri_complete").filter(|u| self.policy.trusted_url(u).is_ok());
        Ok(DeviceAuthorization {
            device_code: Secret::new(text("device_code").ok_or_else(malformed)?),
            user_code: text("user_code").ok_or_else(malformed)?,
            verification_uri,
            verification_uri_complete,
            expires_in: number("expires_in").ok_or_else(malformed)?,
            interval: number("interval").unwrap_or(5).max(1),
        })
    }

    /// Polls once for a device sign-in; the caller waits `interval` seconds
    /// between polls.
    ///
    /// # Errors
    /// [`CloudError::SignInFailed`] when the user declined or the code expired.
    pub fn poll_device(
        &self,
        server: &AuthServer,
        client_id: &str,
        device: &DeviceAuthorization,
        interval: u64,
        cancel: &CancelToken,
    ) -> Result<DevicePoll, CloudError> {
        match self.token_call(
            server,
            &[
                ("grant_type", DEVICE_GRANT),
                ("device_code", device.device_code.expose()),
                ("client_id", client_id),
            ],
            cancel,
        )? {
            Ok(tokens) => Ok(DevicePoll::Done(tokens)),
            Err(TokenError::AuthorizationPending) => Ok(DevicePoll::Pending { interval }),
            Err(TokenError::SlowDown) => Ok(DevicePoll::Pending {
                interval: interval + 5,
            }),
            Err(e) => Err(CloudError::SignInFailed {
                message: e.to_string(),
            }),
        }
    }

    /// Revokes a token (RFC 7009). `Ok(false)` when the server offers no
    /// revocation.
    ///
    /// # Errors
    /// [`CloudError`] when the request fails.
    pub fn revoke(
        &self,
        server: &AuthServer,
        client_id: &str,
        token: &Secret,
        hint: &str,
        cancel: &CancelToken,
    ) -> Result<bool, CloudError> {
        let Some(endpoint) = server.revocation_endpoint.as_deref() else {
            return Ok(false);
        };
        let (status, _) = self.fetch(
            HttpRequest::post_form(
                endpoint,
                &[
                    ("token", token.expose()),
                    ("token_type_hint", hint),
                    ("client_id", client_id),
                ],
                CallLimits::OAUTH,
            ),
            cancel,
        )?;
        if status == 200 {
            Ok(true)
        } else {
            Err(CloudError::SignInFailed {
                message: format!("revocation answered HTTP {status}"),
            })
        }
    }
}

/// What [`TokenManager::disconnect`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Disconnected {
    /// The service confirmed revocation of at least one token.
    pub revoked: bool,
}

fn store_failed(e: SecretError) -> CloudError {
    CloudError::SignInUnavailable {
        detail: e.to_string(),
    }
}

/// Supplies tokens from a [`SecretStore`], refreshing OAuth tokens when they
/// near expiry or after a `401` (§5.6). It reads the store before each
/// request, and keeps in memory the server metadata and any refreshed tokens
/// the store refused (until a write succeeds).
///
/// **One per provider id for the app's whole run** (review R17). The app
/// passes the same `Arc` to every provider it creates (reconnect, account
/// switch) and to the sign-in and disconnect UI. A second instance would read
/// the superseded refresh token from the store and send it, could refresh
/// the same token at the same moment as this one, and would not see this
/// one's sign-in changes.
pub struct TokenManager {
    provider: ProviderId,
    store: Arc<dyn SecretStore>,
    oauth: OAuthClient,
    clock: Arc<dyn Clock>,
    server: Mutex<Option<AuthServer>>,
    /// One refresh at a time (held across the network call).
    refreshing: Mutex<()>,
    /// One write to the store at a time: sign-in, pasted token, the write
    /// after a refresh, and disconnect (review SEC-1). Never held across a
    /// network call. It also guards `unsaved`. The value counts sign-in
    /// changes (sign-in, pasted token, disconnect): a refresh that sees it
    /// change during its network call drops its tokens, without reading the
    /// store again, which may be locked by then (review R7).
    writing: Mutex<u64>,
    /// Refreshed tokens the store would not take (review SEC-3). They are
    /// newer than the stored ones, which the service may already have
    /// invalidated by rotation, so they are used until a write succeeds.
    unsaved: Mutex<Option<OAuthCredential>>,
}

impl fmt::Debug for TokenManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenManager")
            .field("provider", &self.provider)
            .finish_non_exhaustive()
    }
}

impl TokenManager {
    /// A manager for `provider`'s credential in `store`. Create one per
    /// provider id per app run (see [`TokenManager`]).
    pub fn new(
        provider: ProviderId,
        store: Arc<dyn SecretStore>,
        oauth: OAuthClient,
        clock: Arc<dyn Clock>,
    ) -> TokenManager {
        TokenManager {
            provider,
            store,
            oauth,
            clock,
            server: Mutex::new(None),
            refreshing: Mutex::new(()),
            writing: Mutex::new(0),
            unsaved: Mutex::new(None),
        }
    }

    /// The OAuth client.
    pub fn oauth(&self) -> &OAuthClient {
        &self.oauth
    }

    /// Remembers discovered server metadata, so refreshes need no discovery.
    pub fn set_server(&self, server: AuthServer) {
        *self.server.lock().unwrap_or_else(PoisonError::into_inner) = Some(server);
    }

    /// The stored credential.
    ///
    /// # Errors
    /// [`CloudError::NotConnected`] when none is stored, or a store failure.
    pub fn credential(&self) -> Result<Credential, CloudError> {
        if let Some(newer) = self.unsaved() {
            return Ok(Credential::OAuth(newer));
        }
        self.store
            .get(self.provider)
            .map_err(store_failed)?
            .ok_or(CloudError::NotConnected)
    }

    fn unsaved(&self) -> Option<OAuthCredential> {
        self.unsaved
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn set_unsaved(&self, credential: Option<OAuthCredential>) {
        *self.unsaved.lock().unwrap_or_else(PoisonError::into_inner) = credential;
    }

    /// Tries again to store refreshed tokens the store refused earlier.
    /// Called with `writing` held.
    fn flush_unsaved(&self) {
        let Some(newer) = self.unsaved() else {
            return;
        };
        match self.store.set(self.provider, &Credential::OAuth(newer)) {
            Ok(()) => self.set_unsaved(None),
            Err(e) => tracing::warn!(error = %e, "the refreshed sign-in still cannot be stored"),
        }
    }

    /// Stores the tokens of a completed sign-in.
    ///
    /// # Errors
    /// A store failure as [`CloudError::SignInUnavailable`].
    pub fn store_sign_in(
        &self,
        tokens: TokenSet,
        registration: &ClientRegistration,
    ) -> Result<OAuthCredential, CloudError> {
        let credential = OAuthCredential::from_tokens(
            tokens,
            self.clock.now(),
            &registration.client_id,
            &registration.issuer,
            None,
        );
        let mut changes = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        self.store
            .set(self.provider, &Credential::OAuth(credential.clone()))
            .map_err(store_failed)?;
        self.set_unsaved(None);
        *changes += 1;
        Ok(credential)
    }

    /// Stores a pasted token (§5.5).
    ///
    /// # Errors
    /// A store failure as [`CloudError::SignInUnavailable`].
    pub fn store_pasted(&self, token: Secret) -> Result<(), CloudError> {
        let mut changes = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        self.store
            .set(self.provider, &Credential::Bearer { token })
            .map_err(store_failed)?;
        self.set_unsaved(None);
        *changes += 1;
        Ok(())
    }

    fn server_for(&self, issuer: &str, cancel: &CancelToken) -> Result<AuthServer, CloudError> {
        if let Some(server) = self
            .server
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .filter(|s| s.issuer == issuer)
        {
            return Ok(server.clone());
        }
        let found = self.oauth.discover(None, cancel)?.server;
        if found.issuer != issuer {
            // The service moved sign-in elsewhere: the old tokens cannot be
            // refreshed there.
            return Err(CloudError::SignInExpired);
        }
        self.set_server(found.clone());
        Ok(found)
    }

    /// Refreshes unless another thread already did since `seen` was read,
    /// and returns the access token to use. The new tokens are kept only if
    /// the sign-in did not change during the network call: a disconnect or
    /// a new sign-in wins (review SEC-1). Once the service has answered, the
    /// new tokens are never lost to a store failure: they stay in memory
    /// until a write succeeds (SEC-3, R7).
    fn refresh(&self, seen: &OAuthCredential, cancel: &CancelToken) -> Result<Secret, CloudError> {
        let _one_at_a_time = self
            .refreshing
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let changes_before = {
            // Before the next refresh, try again to store the last one.
            let changes = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
            self.flush_unsaved();
            *changes
        };
        let current = match self.credential()? {
            Credential::OAuth(c) => c,
            Credential::Bearer { .. } => return Err(CloudError::SignInExpired),
        };
        if current.access != seen.access {
            return Ok(current.access);
        }
        let Some(refresh_token) = current.refresh.clone() else {
            return Err(CloudError::SignInExpired);
        };
        let server = self.server_for(&current.issuer, cancel)?;
        let tokens = self
            .oauth
            .refresh(&server, &current.client_id, &refresh_token, cancel)?;
        let renewed = OAuthCredential::from_tokens(
            tokens,
            self.clock.now(),
            &current.client_id,
            &current.issuer,
            Some(&current),
        );
        let changes = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        if *changes != changes_before {
            tracing::debug!("the sign-in changed during a refresh; the new tokens are dropped");
            drop(changes);
            return self.credential().map(|c| c.access_token().clone());
        }
        match self
            .store
            .set(self.provider, &Credential::OAuth(renewed.clone()))
        {
            Ok(()) => self.set_unsaved(None),
            // The old refresh token may be dead after rotation: keep the
            // new tokens in memory and store them later.
            Err(e) => {
                tracing::warn!(error = %e, "refreshed tokens kept in memory; the store refused them");
                self.set_unsaved(Some(renewed.clone()));
            }
        }
        tracing::debug!("access token refreshed");
        Ok(renewed.access)
    }

    /// Revokes the tokens when the service offers revocation (best effort),
    /// then deletes them from the store (CG-002).
    ///
    /// # Errors
    /// A store failure; revocation failures only make `revoked` false.
    pub fn disconnect(&self, cancel: &CancelToken) -> Result<Disconnected, CloudError> {
        let mut revoked = false;
        if let Ok(Credential::OAuth(c)) = self.credential()
            && let Ok(server) = self.server_for(&c.issuer, cancel)
        {
            if let Some(refresh) = &c.refresh {
                revoked |= self
                    .oauth
                    .revoke(&server, &c.client_id, refresh, "refresh_token", cancel)
                    .unwrap_or(false);
            }
            revoked |= self
                .oauth
                .revoke(&server, &c.client_id, &c.access, "access_token", cancel)
                .unwrap_or(false);
        }
        let mut changes = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        // Newer tokens kept in memory go only once the stored ones are gone:
        // if the delete fails, they stay the ones in use (review R7).
        self.store.delete(self.provider).map_err(store_failed)?;
        self.set_unsaved(None);
        *changes += 1;
        Ok(Disconnected { revoked })
    }
}

impl TokenSource for TokenManager {
    fn bearer(&self, purpose: TokenPurpose, cancel: &CancelToken) -> Result<Secret, CloudError> {
        let credential = match self.credential()? {
            Credential::Bearer { token } => return Ok(token),
            Credential::OAuth(c) => c,
        };
        let now = self.clock.now();
        let Some(expires_at) = credential.expires_at else {
            return Ok(credential.access);
        };
        let left = expires_at.0.saturating_sub(now.0);
        if left >= purpose.margin_secs() {
            return Ok(credential.access);
        }
        if credential.refresh.is_none() {
            return if left > 0 {
                Ok(credential.access)
            } else {
                Err(CloudError::SignInExpired)
            };
        }
        match self.refresh(&credential, cancel) {
            Ok(access) => Ok(access),
            // Still valid for a moment: use it and refresh next time.
            Err(CloudError::Offline | CloudError::NoAnswer) if left > 0 => Ok(credential.access),
            Err(e) => Err(e),
        }
    }

    fn renew(&self, cancel: &CancelToken) -> Result<(), CloudError> {
        match self.credential()? {
            Credential::Bearer { .. } => Err(CloudError::SignInExpired),
            Credential::OAuth(c) => self.refresh(&c, cancel).map(|_| ()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn server(iss_param: bool) -> AuthServer {
        AuthServer {
            issuer: "https://clerk.higgsfield.ai".into(),
            authorization_endpoint: "https://clerk.higgsfield.ai/oauth/authorize".into(),
            token_endpoint: "https://clerk.higgsfield.ai/oauth/token".into(),
            registration_endpoint: Some("https://clerk.higgsfield.ai/oauth/register".into()),
            revocation_endpoint: None,
            device_authorization_endpoint: None,
            grant_types_supported: vec![],
            scopes_supported: vec![],
            authorization_response_iss_parameter_supported: iss_param,
            client_id_metadata_document_supported: true,
        }
    }

    fn policy() -> AuthPolicy {
        AuthPolicy {
            resource: "https://mcp.higgsfield.ai/mcp".into(),
            trusted_domain: "higgsfield.ai".into(),
            scopes: vec!["openid".into(), "email".into(), "offline_access".into()],
        }
    }

    #[test]
    fn the_authorize_url_carries_every_required_parameter() {
        let pkce = Pkce::from_verifier(Secret::new("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"))
            .unwrap();
        let req = authorize_request(
            &server(true),
            "client-1",
            51234,
            &policy(),
            pkce,
            Secret::new("st4te"),
        )
        .unwrap();
        assert_eq!(
            req.url,
            "https://clerk.higgsfield.ai/oauth/authorize?response_type=code&client_id=client-1\
             &redirect_uri=http%3A%2F%2F127.0.0.1%3A51234%2Fcallback&scope=openid+email+offline_access\
             &code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM&code_challenge_method=S256\
             &state=st4te&resource=https%3A%2F%2Fmcp.higgsfield.ai%2Fmcp"
        );
        assert_eq!(req.redirect_uri, "http://127.0.0.1:51234/callback");
    }

    /// SEC-5: debug output of a sign-in in progress hides the state and the
    /// client id.
    #[test]
    fn debug_output_of_a_sign_in_is_redacted() {
        let pkce = Pkce::from_entropy(&[3u8; 64]);
        let req = authorize_request(
            &server(true),
            "client-xyz",
            50000,
            &policy(),
            pkce,
            Secret::new("st4te-xyz"),
        )
        .unwrap();
        let reg = ClientRegistration {
            client_id: "client-xyz".into(),
            issuer: "https://clerk.higgsfield.ai".into(),
            device_grant: false,
        };
        let shown = format!("{req:?} {reg:?}");
        assert!(!shown.contains("st4te-xyz"), "{shown}");
        assert!(!shown.contains("client-xyz"), "{shown}");
        assert!(
            shown.contains("https://clerk.higgsfield.ai/oauth/authorize?<redacted>"),
            "{shown}"
        );
    }

    #[test]
    fn the_callback_must_match_state_and_issuer() {
        let s = server(true);
        let state = Secret::new("abc");
        let ok = "/callback?code=c0de&state=abc&iss=https%3A%2F%2Fclerk.higgsfield.ai";
        assert_eq!(parse_callback(ok, &state, &s).unwrap().expose(), "c0de");
        assert_eq!(
            parse_callback(
                "/callback?code=c&state=evil&iss=https%3A%2F%2Fclerk.higgsfield.ai",
                &state,
                &s
            ),
            Err(CallbackError::StateMismatch)
        );
        assert_eq!(
            parse_callback("/callback?code=c", &state, &s),
            Err(CallbackError::StateMismatch)
        );
        assert_eq!(
            parse_callback("/favicon.ico", &state, &s),
            Err(CallbackError::NotTheCallback)
        );
        assert_eq!(
            parse_callback("@evil.example/callback?code=c&state=abc", &state, &s),
            Err(CallbackError::NotTheCallback)
        );
        assert_eq!(
            parse_callback("//evil.example/callback?code=c&state=abc", &state, &s),
            Err(CallbackError::NotTheCallback)
        );
        assert_eq!(
            parse_callback("/callback?code=c&state=abc", &state, &s),
            Err(CallbackError::IssuerMismatch)
        );
        assert_eq!(
            parse_callback(
                "/callback?code=c&state=abc&iss=https%3A%2F%2Fevil.example",
                &state,
                &s
            ),
            Err(CallbackError::IssuerMismatch)
        );
        // SEC-7: exact match, a trailing slash included.
        assert_eq!(
            parse_callback(
                "/callback?code=c&state=abc&iss=https%3A%2F%2Fclerk.higgsfield.ai%2F",
                &state,
                &s
            ),
            Err(CallbackError::IssuerMismatch)
        );
        assert_eq!(
            parse_callback(
                "/callback?state=abc&iss=https%3A%2F%2Fclerk.higgsfield.ai",
                &state,
                &s
            ),
            Err(CallbackError::MissingCode)
        );
        assert!(matches!(
            parse_callback("/callback?error=access_denied&error_description=No%0Athanks&state=abc&iss=https%3A%2F%2Fclerk.higgsfield.ai", &state, &s),
            Err(CallbackError::Denied { ref error, ref description }) if error == "access_denied" && description == "No thanks"
        ));
        // Without RFC 9207 support, `iss` is optional.
        assert!(parse_callback("/callback?code=c&state=abc", &state, &server(false)).is_ok());
    }

    #[test]
    fn the_registration_request_is_a_public_native_client() {
        assert_eq!(
            registration_body(&policy(), false),
            json!({
                "client_name": "FreeCut",
                "redirect_uris": ["http://127.0.0.1/callback"],
                "grant_types": ["authorization_code", "refresh_token"],
                "response_types": ["code"],
                "token_endpoint_auth_method": "none",
                "application_type": "native",
                "scope": "openid email offline_access",
            })
        );
    }
}
