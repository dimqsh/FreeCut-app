//! Token endpoint answers (RFC 6749 §5, RFC 8628 §3.5) and the unverified
//! ID-token claims FreeCut uses as labels.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;

use crate::secret::{OAuthCredential, Secret, SubjectHash, UnixSeconds};

/// Tokens from a successful exchange, refresh or device poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenSet {
    /// The access token.
    pub access: Secret,
    /// A (possibly rotated) refresh token.
    pub refresh: Option<Secret>,
    /// Lifetime of the access token in seconds, if given.
    pub expires_in: Option<u64>,
    /// Granted scopes, if given.
    pub scope: Option<String>,
    /// The ID token, if the `openid` scope was granted.
    pub id_token: Option<Secret>,
}

/// An error answer from the token endpoint.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TokenError {
    /// `invalid_grant`: the code or refresh token is expired or revoked.
    #[error("the grant is invalid or expired")]
    InvalidGrant,
    /// Device flow: the user has not finished yet.
    #[error("authorization pending")]
    AuthorizationPending,
    /// Device flow: poll more slowly.
    #[error("slow down")]
    SlowDown,
    /// The user declined.
    #[error("access denied")]
    AccessDenied,
    /// Device flow: the code expired.
    #[error("the device code expired")]
    ExpiredToken,
    /// Another OAuth error.
    #[error("{code}: {description}")]
    Other {
        /// The `error` code.
        code: String,
        /// `error_description` (untrusted text).
        description: String,
    },
    /// Not a token answer at all.
    #[error("unreadable token answer: {0}")]
    Malformed(String),
}

#[derive(Deserialize)]
struct RawTokens {
    access_token: String,
    #[serde(default)]
    token_type: String,
    refresh_token: Option<String>,
    expires_in: Option<serde_json::Value>,
    scope: Option<String>,
    id_token: Option<String>,
}

#[derive(Deserialize)]
struct RawError {
    error: String,
    #[serde(default)]
    error_description: String,
}

/// Reads a token endpoint answer.
///
/// # Errors
/// [`TokenError`] for OAuth error answers and anything unreadable.
pub fn parse_token_response(status: u16, body: &[u8]) -> Result<TokenSet, TokenError> {
    if (200..300).contains(&status) {
        let raw: RawTokens =
            serde_json::from_slice(body).map_err(|e| TokenError::Malformed(e.to_string()))?;
        if !raw.token_type.eq_ignore_ascii_case("bearer") {
            return Err(TokenError::Malformed(format!(
                "token type {:?} is not Bearer",
                raw.token_type
            )));
        }
        if raw.access_token.is_empty() {
            return Err(TokenError::Malformed("empty access token".into()));
        }
        // Some servers send `expires_in` as a string.
        let expires_in = match raw.expires_in {
            Some(serde_json::Value::Number(n)) => n.as_u64(),
            Some(serde_json::Value::String(s)) => s.trim().parse().ok(),
            _ => None,
        };
        return Ok(TokenSet {
            access: Secret::new(raw.access_token),
            refresh: raw.refresh_token.filter(|r| !r.is_empty()).map(Secret::new),
            expires_in,
            scope: raw.scope,
            id_token: raw.id_token.map(Secret::new),
        });
    }
    let raw: RawError = serde_json::from_slice(body)
        .map_err(|_| TokenError::Malformed(format!("HTTP {status} without an OAuth error")))?;
    Err(match raw.error.as_str() {
        "invalid_grant" => TokenError::InvalidGrant,
        "authorization_pending" => TokenError::AuthorizationPending,
        "slow_down" => TokenError::SlowDown,
        "access_denied" => TokenError::AccessDenied,
        "expired_token" => TokenError::ExpiredToken,
        _ => TokenError::Other {
            code: raw.error,
            description: crate::error::quoted(&raw.error_description),
        },
    })
}

/// Claims read from an ID token **without** verifying its signature. They
/// only label the account (status line) and tag ledger entries; nothing is
/// authorized on them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdClaims {
    /// `sub`.
    pub subject: Option<String>,
    /// `email`.
    pub email: Option<String>,
}

/// Reads `sub` and `email` from a JWT's payload.
pub fn id_token_claims(id_token: &str) -> Option<IdClaims> {
    let payload = id_token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let text = |k: &str| {
        value
            .get(k)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    Some(IdClaims {
        subject: text("sub"),
        email: text("email"),
    })
}

impl OAuthCredential {
    /// The credential for a fresh token set. On refresh, pass the previous
    /// credential: its refresh token, scope and account tags are kept where
    /// the new answer omits them.
    pub fn from_tokens(
        tokens: TokenSet,
        now: UnixSeconds,
        client_id: &str,
        issuer: &str,
        previous: Option<&OAuthCredential>,
    ) -> OAuthCredential {
        let claims = tokens
            .id_token
            .as_ref()
            .and_then(|t| id_token_claims(t.expose()))
            .unwrap_or_default();
        let expires_at = tokens.expires_in.and_then(|secs| {
            i64::try_from(secs)
                .ok()
                .and_then(|s| now.0.checked_add(s))
                .map(UnixSeconds)
        });
        OAuthCredential {
            access: tokens.access,
            refresh: tokens
                .refresh
                .or_else(|| previous.and_then(|p| p.refresh.clone())),
            expires_at,
            scope: tokens
                .scope
                .or_else(|| previous.map(|p| p.scope.clone()))
                .unwrap_or_default(),
            client_id: client_id.to_owned(),
            issuer: issuer.to_owned(),
            subject_hash: claims
                .subject
                .as_deref()
                .map(SubjectHash::of)
                .or_else(|| previous.and_then(|p| p.subject_hash)),
            account_label: claims
                .email
                .or_else(|| previous.and_then(|p| p.account_label.clone())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn jwt(payload: &str) -> String {
        format!(
            "eyJhbGciOiJSUzI1NiJ9.{}.sig",
            URL_SAFE_NO_PAD.encode(payload)
        )
    }

    #[test]
    fn a_full_token_answer() {
        let body = format!(
            r#"{{"access_token":"at","token_type":"bearer","expires_in":"3600","refresh_token":"rt","scope":"openid email","id_token":"{}"}}"#,
            jwt(r#"{"sub":"user_1","email":"a@example.test"}"#)
        );
        let set = parse_token_response(200, body.as_bytes()).unwrap();
        assert_eq!(set.access.expose(), "at");
        assert_eq!(set.expires_in, Some(3600));
        let cred =
            OAuthCredential::from_tokens(set, UnixSeconds(1_000), "cid", "https://iss", None);
        assert_eq!(cred.expires_at, Some(UnixSeconds(4_600)));
        assert_eq!(cred.subject_hash, Some(SubjectHash::of("user_1")));
        assert_eq!(cred.account_label.as_deref(), Some("a@example.test"));
    }

    #[test]
    fn a_refresh_without_a_new_refresh_token_keeps_the_old_one() {
        let first = OAuthCredential::from_tokens(
            parse_token_response(
                200,
                br#"{"access_token":"a1","token_type":"Bearer","refresh_token":"r1","scope":"s"}"#,
            )
            .unwrap(),
            UnixSeconds(0),
            "cid",
            "iss",
            None,
        );
        let second = OAuthCredential::from_tokens(
            parse_token_response(
                200,
                br#"{"access_token":"a2","token_type":"Bearer","expires_in":60}"#,
            )
            .unwrap(),
            UnixSeconds(10),
            "cid",
            "iss",
            Some(&first),
        );
        assert_eq!(second.access.expose(), "a2");
        assert_eq!(second.refresh.as_ref().map(Secret::expose), Some("r1"));
        assert_eq!(second.scope, "s");
        assert_eq!(second.expires_at, Some(UnixSeconds(70)));
    }

    #[test]
    fn oauth_errors() {
        assert_eq!(
            parse_token_response(400, br#"{"error":"invalid_grant"}"#),
            Err(TokenError::InvalidGrant)
        );
        assert_eq!(
            parse_token_response(400, br#"{"error":"slow_down"}"#),
            Err(TokenError::SlowDown)
        );
        assert!(matches!(
            parse_token_response(400, br#"{"error":"server_error","error_description":"x"}"#),
            Err(TokenError::Other { .. })
        ));
        assert!(matches!(
            parse_token_response(500, b"<html>"),
            Err(TokenError::Malformed(_))
        ));
        assert!(matches!(
            parse_token_response(200, br#"{"access_token":"a","token_type":"mac"}"#),
            Err(TokenError::Malformed(_))
        ));
    }

    #[test]
    fn claims_from_an_unsigned_read() {
        assert_eq!(
            id_token_claims(&jwt(r#"{"sub":"s1"}"#)),
            Some(IdClaims {
                subject: Some("s1".into()),
                email: None
            })
        );
        assert_eq!(id_token_claims("not-a-jwt"), None);
    }
}
