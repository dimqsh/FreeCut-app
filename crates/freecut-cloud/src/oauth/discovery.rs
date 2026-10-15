//! OAuth discovery (architecture §5.1): protected-resource metadata
//! (RFC 9728) and authorization-server metadata (RFC 8414, with the OpenID
//! fallback the MCP authorization spec allows).
//!
//! Only the MCP endpoint is built in. Every other endpoint comes from
//! metadata, and each must be HTTPS on the provider's own domain
//! ([`AuthPolicy::is_trusted`]); the resource must equal the endpoint and an
//! issuer must equal the server it was fetched for.

use serde::Deserialize;
use url::Url;

/// What FreeCut expects of a provider's sign-in metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthPolicy {
    /// The MCP endpoint, which is also the OAuth resource (RFC 8707).
    pub resource: String,
    /// The registrable domain every sign-in endpoint must be on.
    pub trusted_domain: String,
    /// The scopes FreeCut asks for.
    pub scopes: Vec<String>,
}

impl AuthPolicy {
    /// True for HTTPS URLs on the trusted domain (or a subdomain), with the
    /// default port and no user info.
    pub fn is_trusted(&self, url: &Url) -> bool {
        let domain = self.trusted_domain.as_str();
        url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url.port().is_none()
            && url.host_str().is_some_and(|h| {
                let h = h.to_ascii_lowercase();
                h == domain || h.ends_with(&format!(".{domain}"))
            })
    }

    /// Parses `text` and checks it with [`AuthPolicy::is_trusted`].
    ///
    /// # Errors
    /// [`DiscoveryError`] for unparseable or untrusted URLs.
    pub fn trusted_url(&self, text: &str) -> Result<Url, DiscoveryError> {
        let url = Url::parse(text).map_err(|_| DiscoveryError::BadUrl(text.to_owned()))?;
        if self.is_trusted(&url) {
            Ok(url)
        } else {
            Err(DiscoveryError::Untrusted(text.to_owned()))
        }
    }
}

/// Why sign-in metadata was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DiscoveryError {
    /// Not a URL.
    #[error("not a URL: {0}")]
    BadUrl(String),
    /// Not HTTPS on the provider's domain.
    #[error("not on the service's own HTTPS domain: {0}")]
    Untrusted(String),
    /// The metadata describes another resource.
    #[error("the metadata is for {found}, not {expected}")]
    ResourceMismatch {
        /// The endpoint FreeCut connects to.
        expected: String,
        /// What the metadata names.
        found: String,
    },
    /// The issuer is not the server whose metadata was fetched.
    #[error("the issuer is {found}, not {expected}")]
    IssuerMismatch {
        /// The server FreeCut asked.
        expected: String,
        /// What its metadata names.
        found: String,
    },
    /// No PKCE S256 support (the MCP spec requires refusing such servers).
    #[error("the sign-in server does not support PKCE S256")]
    NoS256,
    /// Missing fields or invalid JSON.
    #[error("unreadable metadata: {0}")]
    Malformed(String),
    /// None of the listed servers could be used.
    #[error("no usable sign-in server")]
    NoUsableServer,
}

/// A `WWW-Authenticate: Bearer …` challenge (RFC 6750, RFC 9728 §5.1).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BearerChallenge {
    /// Where the protected-resource metadata is.
    pub resource_metadata: Option<String>,
    /// The scopes the resource asks for.
    pub scope: Option<String>,
    /// `invalid_token`, `insufficient_scope`, …
    pub error: Option<String>,
    /// The server's description (untrusted text).
    pub error_description: Option<String>,
}

/// Parses the first `Bearer` challenge of a `WWW-Authenticate` value.
pub fn parse_bearer_challenge(header: &str) -> Option<BearerChallenge> {
    let trimmed = header.trim_start();
    let scheme_len = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
    if !trimmed[..scheme_len].eq_ignore_ascii_case("bearer") {
        return None;
    }
    let mut challenge = BearerChallenge::default();
    for (name, value) in auth_params(&trimmed[scheme_len..]) {
        match name.to_ascii_lowercase().as_str() {
            "resource_metadata" => challenge.resource_metadata = Some(value),
            "scope" => challenge.scope = Some(value),
            "error" => challenge.error = Some(value),
            "error_description" => challenge.error_description = Some(value),
            _ => {}
        }
    }
    Some(challenge)
}

/// `name=value` pairs, values as tokens or quoted strings with `\` escapes.
fn auth_params(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| c.is_whitespace() || *c == ',') {
            chars.next();
        }
        let mut name = String::new();
        while let Some(&c) = chars.peek() {
            if c == '=' || c == ',' || c.is_whitespace() {
                break;
            }
            name.push(c);
            chars.next();
        }
        if name.is_empty() {
            return out;
        }
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        if chars.peek() != Some(&'=') {
            // A bare token (another scheme starts here): stop.
            return out;
        }
        chars.next();
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        let mut value = String::new();
        if chars.peek() == Some(&'"') {
            chars.next();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => {
                        if let Some(escaped) = chars.next() {
                            value.push(escaped);
                        }
                    }
                    '"' => break,
                    c => value.push(c),
                }
            }
        } else {
            while let Some(&c) = chars.peek() {
                if c == ',' || c.is_whitespace() {
                    break;
                }
                value.push(c);
                chars.next();
            }
        }
        out.push((name, value));
    }
}

/// Protected-resource metadata (RFC 9728).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectedResource {
    /// The resource identifier.
    pub resource: String,
    /// Authorization servers, in the service's order of preference.
    pub authorization_servers: Vec<String>,
    /// Scopes the resource supports.
    pub scopes_supported: Vec<String>,
}

#[derive(Deserialize)]
struct RawProtectedResource {
    resource: String,
    #[serde(default)]
    authorization_servers: Vec<String>,
    #[serde(default)]
    scopes_supported: Vec<String>,
}

/// Where to look for a resource's metadata: the path-specific well-known
/// URL first, then the root one (RFC 9728 §3.1).
///
/// # Errors
/// [`DiscoveryError::BadUrl`] for an unparseable resource.
pub fn protected_resource_urls(resource: &str) -> Result<Vec<String>, DiscoveryError> {
    well_known(resource, "oauth-protected-resource")
}

fn well_known(base: &str, name: &str) -> Result<Vec<String>, DiscoveryError> {
    let url = Url::parse(base).map_err(|_| DiscoveryError::BadUrl(base.to_owned()))?;
    let origin = url.origin().ascii_serialization();
    let path = url.path().trim_end_matches('/');
    let mut urls = Vec::new();
    if !path.is_empty() {
        urls.push(format!("{origin}/.well-known/{name}{path}"));
    }
    urls.push(format!("{origin}/.well-known/{name}"));
    Ok(urls)
}

/// Reads protected-resource metadata and checks that it describes the
/// expected resource.
///
/// # Errors
/// [`DiscoveryError`] for unreadable JSON or another resource.
pub fn parse_protected_resource(
    body: &[u8],
    policy: &AuthPolicy,
) -> Result<ProtectedResource, DiscoveryError> {
    let raw: RawProtectedResource =
        serde_json::from_slice(body).map_err(|e| DiscoveryError::Malformed(e.to_string()))?;
    if !same_url(&raw.resource, &policy.resource) {
        return Err(DiscoveryError::ResourceMismatch {
            expected: policy.resource.clone(),
            found: raw.resource,
        });
    }
    Ok(ProtectedResource {
        resource: raw.resource,
        authorization_servers: raw.authorization_servers,
        scopes_supported: raw.scopes_supported,
    })
}

/// Authorization-server metadata (RFC 8414) as FreeCut uses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthServer {
    /// The issuer identifier.
    pub issuer: String,
    /// Where the browser goes to sign in.
    pub authorization_endpoint: String,
    /// Where codes and refresh tokens are exchanged.
    pub token_endpoint: String,
    /// Dynamic client registration (RFC 7591), if offered.
    pub registration_endpoint: Option<String>,
    /// Token revocation (RFC 7009), if offered.
    pub revocation_endpoint: Option<String>,
    /// Device authorization (RFC 8628), if offered.
    pub device_authorization_endpoint: Option<String>,
    /// Supported grant types.
    pub grant_types_supported: Vec<String>,
    /// Supported scopes.
    pub scopes_supported: Vec<String>,
    /// Whether redirects carry `iss` (RFC 9207), which FreeCut then checks.
    pub authorization_response_iss_parameter_supported: bool,
    /// Whether an HTTPS client-id metadata document is accepted.
    pub client_id_metadata_document_supported: bool,
}

#[derive(Deserialize)]
struct RawAuthServer {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    registration_endpoint: Option<String>,
    revocation_endpoint: Option<String>,
    device_authorization_endpoint: Option<String>,
    #[serde(default)]
    grant_types_supported: Vec<String>,
    #[serde(default)]
    scopes_supported: Vec<String>,
    #[serde(default)]
    code_challenge_methods_supported: Vec<String>,
    #[serde(default)]
    authorization_response_iss_parameter_supported: bool,
    #[serde(default)]
    client_id_metadata_document_supported: bool,
}

/// Where to look for an issuer's metadata, in the MCP authorization spec's
/// order: OAuth with path insertion, OpenID with path insertion, OpenID with
/// the path appended.
///
/// # Errors
/// [`DiscoveryError::BadUrl`] for an unparseable issuer.
pub fn auth_server_metadata_urls(issuer: &str) -> Result<Vec<String>, DiscoveryError> {
    let url = Url::parse(issuer).map_err(|_| DiscoveryError::BadUrl(issuer.to_owned()))?;
    let origin = url.origin().ascii_serialization();
    let path = url.path().trim_end_matches('/');
    Ok(if path.is_empty() {
        vec![
            format!("{origin}/.well-known/oauth-authorization-server"),
            format!("{origin}/.well-known/openid-configuration"),
        ]
    } else {
        vec![
            format!("{origin}/.well-known/oauth-authorization-server{path}"),
            format!("{origin}/.well-known/openid-configuration{path}"),
            format!("{origin}{path}/.well-known/openid-configuration"),
        ]
    })
}

/// Reads authorization-server metadata fetched for `expected_issuer` and
/// checks it: same issuer, trusted HTTPS endpoints, PKCE S256.
///
/// # Errors
/// [`DiscoveryError`] when any check fails.
pub fn parse_auth_server(
    body: &[u8],
    expected_issuer: &str,
    policy: &AuthPolicy,
) -> Result<AuthServer, DiscoveryError> {
    let raw: RawAuthServer =
        serde_json::from_slice(body).map_err(|e| DiscoveryError::Malformed(e.to_string()))?;
    // RFC 8414 §3.3: identical, not merely equivalent (review SEC-7).
    if raw.issuer != expected_issuer {
        return Err(DiscoveryError::IssuerMismatch {
            expected: expected_issuer.to_owned(),
            found: raw.issuer,
        });
    }
    if !raw
        .code_challenge_methods_supported
        .iter()
        .any(|m| m == "S256")
    {
        return Err(DiscoveryError::NoS256);
    }
    let check = |u: &str| policy.trusted_url(u).map(|_| u.to_owned());
    let check_opt = |u: Option<String>| u.map(|u| check(&u)).transpose();
    Ok(AuthServer {
        issuer: check(&raw.issuer)?,
        authorization_endpoint: check(&raw.authorization_endpoint)?,
        token_endpoint: check(&raw.token_endpoint)?,
        registration_endpoint: check_opt(raw.registration_endpoint)?,
        revocation_endpoint: check_opt(raw.revocation_endpoint)?,
        device_authorization_endpoint: check_opt(raw.device_authorization_endpoint)?,
        grant_types_supported: raw.grant_types_supported,
        scopes_supported: raw.scopes_supported,
        authorization_response_iss_parameter_supported: raw
            .authorization_response_iss_parameter_supported,
        client_id_metadata_document_supported: raw.client_id_metadata_document_supported,
    })
}

/// URL equality ignoring a trailing slash: for the resource identifier
/// only. Issuers are compared exactly (RFC 8414 §3.3, RFC 9207).
pub(crate) fn same_url(a: &str, b: &str) -> bool {
    a.trim_end_matches('/') == b.trim_end_matches('/')
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn policy() -> AuthPolicy {
        AuthPolicy {
            resource: "https://mcp.higgsfield.ai/mcp".into(),
            trusted_domain: "higgsfield.ai".into(),
            scopes: vec!["openid".into(), "email".into(), "offline_access".into()],
        }
    }

    #[test]
    fn the_recorded_challenge_names_the_metadata() {
        let header = r#"Bearer resource_metadata="https://mcp.higgsfield.ai/.well-known/oauth-protected-resource/mcp", scope="openid email offline_access""#;
        assert_eq!(
            parse_bearer_challenge(header),
            Some(BearerChallenge {
                resource_metadata: Some(
                    "https://mcp.higgsfield.ai/.well-known/oauth-protected-resource/mcp".into()
                ),
                scope: Some("openid email offline_access".into()),
                error: None,
                error_description: None,
            })
        );
    }

    #[test]
    fn challenge_parameters_in_any_form() {
        let c = parse_bearer_challenge(
            r#"bearer error=invalid_token, error_description="a \"quoted\" text""#,
        )
        .unwrap();
        assert_eq!(c.error.as_deref(), Some("invalid_token"));
        assert_eq!(c.error_description.as_deref(), Some(r#"a "quoted" text"#));
        assert_eq!(parse_bearer_challenge("Basic realm=x"), None);
        assert_eq!(
            parse_bearer_challenge("Bearer"),
            Some(BearerChallenge::default())
        );
    }

    #[test]
    fn well_known_urls() {
        assert_eq!(
            protected_resource_urls("https://mcp.higgsfield.ai/mcp").unwrap(),
            vec![
                "https://mcp.higgsfield.ai/.well-known/oauth-protected-resource/mcp",
                "https://mcp.higgsfield.ai/.well-known/oauth-protected-resource",
            ]
        );
        assert_eq!(
            auth_server_metadata_urls("https://clerk.higgsfield.ai").unwrap(),
            vec![
                "https://clerk.higgsfield.ai/.well-known/oauth-authorization-server",
                "https://clerk.higgsfield.ai/.well-known/openid-configuration",
            ]
        );
        assert_eq!(
            auth_server_metadata_urls("https://auth.example.test/tenant1/").unwrap(),
            vec![
                "https://auth.example.test/.well-known/oauth-authorization-server/tenant1",
                "https://auth.example.test/.well-known/openid-configuration/tenant1",
                "https://auth.example.test/tenant1/.well-known/openid-configuration",
            ]
        );
    }

    #[test]
    fn trust_is_https_on_the_providers_domain() {
        let p = policy();
        let t = |u: &str| p.is_trusted(&Url::parse(u).unwrap());
        assert!(t("https://clerk.higgsfield.ai/oauth/token"));
        assert!(t("https://higgsfield.ai/x"));
        assert!(!t("http://clerk.higgsfield.ai/oauth/token"));
        assert!(!t("https://higgsfield.ai.evil.test/x"));
        assert!(!t("https://evilhiggsfield.ai/x"));
        assert!(!t("https://user@clerk.higgsfield.ai/x"));
        assert!(!t("https://clerk.higgsfield.ai:8443/x"));
    }

    #[test]
    fn a_resource_mismatch_is_refused() {
        let body = br#"{"resource":"https://other.higgsfield.ai/mcp","authorization_servers":[]}"#;
        assert!(matches!(
            parse_protected_resource(body, &policy()),
            Err(DiscoveryError::ResourceMismatch { .. })
        ));
    }

    #[test]
    fn issuer_and_s256_are_checked() {
        let p = policy();
        let base = |issuer: &str, s256: bool| {
            format!(
                r#"{{"issuer":"{issuer}","authorization_endpoint":"https://clerk.higgsfield.ai/oauth/authorize",
                "token_endpoint":"https://clerk.higgsfield.ai/oauth/token","code_challenge_methods_supported":{}}}"#,
                if s256 { r#"["S256"]"# } else { "[]" }
            )
        };
        assert!(
            parse_auth_server(
                base("https://clerk.higgsfield.ai", true).as_bytes(),
                "https://clerk.higgsfield.ai",
                &p
            )
            .is_ok()
        );
        // SEC-7: issuers match exactly; a trailing slash is another issuer.
        assert!(matches!(
            parse_auth_server(
                base("https://clerk.higgsfield.ai", true).as_bytes(),
                "https://clerk.higgsfield.ai/",
                &p
            ),
            Err(DiscoveryError::IssuerMismatch { .. })
        ));
        assert!(matches!(
            parse_auth_server(
                base("https://mcp.higgsfield.ai", true).as_bytes(),
                "https://clerk.higgsfield.ai",
                &p
            ),
            Err(DiscoveryError::IssuerMismatch { .. })
        ));
        assert_eq!(
            parse_auth_server(
                base("https://clerk.higgsfield.ai", false).as_bytes(),
                "https://clerk.higgsfield.ai",
                &p
            ),
            Err(DiscoveryError::NoS256)
        );
        let foreign_token = r#"{"issuer":"https://clerk.higgsfield.ai","authorization_endpoint":"https://clerk.higgsfield.ai/a",
            "token_endpoint":"https://tokens.example.test/t","code_challenge_methods_supported":["S256"]}"#;
        assert!(matches!(
            parse_auth_server(foreign_token.as_bytes(), "https://clerk.higgsfield.ai", &p),
            Err(DiscoveryError::Untrusted(_))
        ));
    }
}
