//! Errors as the rest of FreeCut sees them (architecture §10).
//!
//! [`classify`] turns any MCP failure into a [`CloudError`]; for paid
//! submissions [`submit_disposition`] also decides between *not sent*, *not
//! accepted* and *outcome unknown* (CG-010). Service messages are matched by
//! structured fields first, then by known phrases, and otherwise passed on as
//! quoted text. All server text is untrusted data.

use serde_json::Value;

use crate::mcp::McpError;
use crate::transport::{SendState, TransportErrorKind};

/// Why the service refused a request that FreeCut then did not resend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotAcceptedReason {
    /// The service answered `401`; FreeCut renewed the sign-in for the next
    /// attempt.
    Unauthorized,
    /// The service answered `404` for the session; FreeCut opened (or tried
    /// to open) a new one for the next attempt.
    SessionExpired,
}

/// A cloud failure, classified for the UI (§10's table). The `Display` text is
/// for logs; the app words each case for the user.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CloudError {
    /// DNS, connect or TLS failure before anything was sent.
    #[error("can't reach the service")]
    Offline,
    /// A timeout or reset after the request may have been sent.
    #[error("sent, but no answer from the service")]
    NoAnswer,
    /// No account is connected for this provider.
    #[error("no account is connected")]
    NotConnected,
    /// The sign-in expired and could not be renewed.
    #[error("the sign-in expired")]
    SignInExpired,
    /// The credential store could not be used.
    #[error("the sign-in is not available: {detail}")]
    SignInUnavailable {
        /// What the store reported.
        detail: String,
    },
    /// Sign-in or registration failed at the service.
    #[error("sign-in failed: {message}")]
    SignInFailed {
        /// The service's message or FreeCut's description (quoted text).
        message: String,
    },
    /// `403` / `insufficient_scope`.
    #[error("the service did not allow this for FreeCut's sign-in")]
    NotPermitted,
    /// Not enough credits on the account's plan.
    #[error("not enough credits")]
    InsufficientCredits,
    /// The connection needs a paid plan.
    #[error("a paid plan is required")]
    PlanRequired,
    /// Refused under the service's content rules.
    #[error("refused under the service's content rules: {message}")]
    ContentRefused {
        /// The service's message (quoted text).
        message: String,
    },
    /// The model is blocked, unknown or unavailable.
    #[error("the model is not available: {message}")]
    ModelUnavailable {
        /// The service's message (quoted text).
        message: String,
    },
    /// Concurrency or rate limit.
    #[error("the service is busy for this account")]
    Busy,
    /// The service rejected the arguments.
    #[error("the service rejected the request: {message}")]
    InvalidRequest {
        /// The service's message (quoted text).
        message: String,
    },
    /// The service does not offer what this action needs right now.
    #[error("not offered by the service right now: {what}")]
    NotOffered {
        /// The missing tool or capability.
        what: String,
    },
    /// The tool's live schema no longer accepts FreeCut's arguments.
    #[error("the service changed {tool}: {detail}")]
    SchemaDrift {
        /// The tool.
        tool: String,
        /// The first mismatch.
        detail: String,
    },
    /// The service refused the request before running it and FreeCut did not
    /// resend it (CG-010's *Not accepted*).
    #[error("the service did not accept the request ({reason:?})")]
    NotAccepted {
        /// Why.
        reason: NotAcceptedReason,
    },
    /// An answer FreeCut cannot read.
    #[error("unexpected answer from the service: {detail}")]
    Protocol {
        /// Details for the log.
        detail: String,
    },
    /// The service speaks an MCP revision FreeCut does not implement.
    #[error("unsupported service version {version}")]
    UnsupportedServiceVersion {
        /// The revision the service chose.
        version: String,
    },
    /// The caller cancelled.
    #[error("cancelled")]
    Cancelled,
    /// FreeCut's own preflight guard refused to send (a bug, §4.7).
    #[error("refused by the preflight guard: {detail}")]
    GuardRefused {
        /// What the guard found.
        detail: String,
    },
    /// Any other service error.
    #[error("service error: {message}")]
    ServiceError {
        /// The service's message (quoted text).
        message: String,
    },
}

/// How a paid submission failed (architecture §6.1, CG-010).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SubmitError {
    /// Nothing left the computer: *Failed: not sent*; Run again carries no
    /// double-payment warning.
    #[error("not sent: {0}")]
    NotSent(CloudError),
    /// It may have reached the service without a readable answer: *Outcome
    /// unknown*; never resent automatically.
    #[error("outcome unknown: {0}")]
    OutcomeUnknown(CloudError),
    /// The service answered with a refusal or an error.
    #[error("rejected: {0}")]
    Rejected(CloudError),
}

/// The longest service message FreeCut keeps, in characters.
const MAX_MESSAGE_CHARS: usize = 500;

/// Service text made safe to show as a quote: control characters become
/// spaces, whitespace runs collapse, and it is cut to 500 characters.
pub fn quoted(message: &str) -> String {
    let cleaned: String = message
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let mut out = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.chars().count() > MAX_MESSAGE_CHARS {
        out = out.chars().take(MAX_MESSAGE_CHARS).collect();
        out.push('…');
    }
    out
}

/// Classifies any MCP failure (free calls, preflights, catalogue, handshake).
pub fn classify(err: &McpError) -> CloudError {
    match err {
        McpError::Transport(e) => match (e.kind, e.send) {
            (TransportErrorKind::Cancelled, _) => CloudError::Cancelled,
            (_, SendState::NotSent) => CloudError::Offline,
            (_, SendState::MaybeSent) => CloudError::NoAnswer,
        },
        McpError::SignIn(e) => e.clone(),
        McpError::Unauthorized { .. } => CloudError::SignInExpired,
        McpError::Forbidden { .. } => CloudError::NotPermitted,
        McpError::SessionExpired => CloudError::NotAccepted {
            reason: NotAcceptedReason::SessionExpired,
        },
        McpError::NotAccepted(reason) => CloudError::NotAccepted { reason: *reason },
        McpError::Http { status, excerpt } => classify_status(*status, excerpt),
        McpError::Rpc(e) => {
            if e.code == -32602 {
                // "Invalid params": still look for a more specific cause.
                match classify_service_message(&e.message, e.data.as_ref()) {
                    CloudError::ServiceError { message } => CloudError::InvalidRequest { message },
                    other => other,
                }
            } else {
                classify_service_message(&e.message, e.data.as_ref())
            }
        }
        McpError::Tool {
            message,
            structured,
        } => classify_service_message(message, structured.as_ref()),
        McpError::Protocol { detail } => CloudError::Protocol {
            detail: detail.clone(),
        },
        McpError::UnsupportedVersion(v) => {
            CloudError::UnsupportedServiceVersion { version: v.clone() }
        }
        McpError::Cancelled { .. } => CloudError::Cancelled,
        McpError::Guard(v) => CloudError::GuardRefused {
            detail: v.to_string(),
        },
        McpError::BeforeSending(inner) => classify(inner),
    }
}

fn classify_status(status: u16, excerpt: &str) -> CloudError {
    match status {
        402 => CloudError::InsufficientCredits,
        403 => CloudError::NotPermitted,
        429 => CloudError::Busy,
        _ => match classify_service_message(excerpt, None) {
            CloudError::ServiceError { .. } => CloudError::ServiceError {
                message: if excerpt.is_empty() {
                    format!("HTTP {status}")
                } else {
                    format!("HTTP {status}: {}", quoted(excerpt))
                },
            },
            specific => specific,
        },
    }
}

/// Where a paid submission's failure leaves the run (CG-010). Only failures
/// that provably happened before sending are `NotSent`; anything the service
/// answered with a refusal is `Rejected`; everything else is
/// `OutcomeUnknown`.
pub fn submit_disposition(err: &McpError) -> SubmitError {
    match err {
        McpError::Transport(e) => match e.send {
            SendState::NotSent => SubmitError::NotSent(classify(err)),
            SendState::MaybeSent => SubmitError::OutcomeUnknown(CloudError::NoAnswer),
        },
        McpError::Cancelled { send } => match send {
            SendState::NotSent => SubmitError::NotSent(CloudError::Cancelled),
            SendState::MaybeSent => SubmitError::OutcomeUnknown(CloudError::NoAnswer),
        },
        // Refused before sending: no token, a guard refusal, an unsupported
        // service found during the handshake.
        McpError::SignIn(_)
        | McpError::Guard(_)
        | McpError::UnsupportedVersion(_)
        | McpError::BeforeSending(_) => SubmitError::NotSent(classify(err)),
        McpError::Unauthorized { .. }
        | McpError::Forbidden { .. }
        | McpError::SessionExpired
        | McpError::NotAccepted(_)
        | McpError::Rpc(_)
        | McpError::Tool { .. } => SubmitError::Rejected(classify(err)),
        McpError::Http { status, .. } => {
            if *status >= 500 {
                // A server or gateway error says nothing about whether the
                // job was created behind it.
                SubmitError::OutcomeUnknown(classify(err))
            } else {
                SubmitError::Rejected(classify(err))
            }
        }
        // An answer that cannot be read after a paid request was sent is
        // always an unknown outcome, never a failure.
        McpError::Protocol { .. } => SubmitError::OutcomeUnknown(classify(err)),
    }
}

/// Structured error codes FreeCut recognizes. The service documents none, so
/// these are common spellings; unknown codes fall through to phrases.
const CODES: &[(&str, Kind)] = &[
    ("insufficient_credits", Kind::Credits),
    ("not_enough_credits", Kind::Credits),
    ("insufficient_balance", Kind::Credits),
    ("payment_required", Kind::Credits),
    ("plan_required", Kind::Plan),
    ("subscription_required", Kind::Plan),
    ("nsfw", Kind::Content),
    ("content_policy", Kind::Content),
    ("content_policy_violation", Kind::Content),
    ("moderation", Kind::Content),
    ("model_unavailable", Kind::Model),
    ("model_not_found", Kind::Model),
    ("unknown_model", Kind::Model),
    ("rate_limited", Kind::Busy),
    ("rate_limit", Kind::Busy),
    ("too_many_requests", Kind::Busy),
    ("concurrency_limit", Kind::Busy),
    ("invalid_request", Kind::Invalid),
    ("validation_error", Kind::Invalid),
];

/// Phrases, lower-case, checked in order (the first match wins).
const PHRASES: &[(&str, Kind)] = &[
    ("insufficient credits", Kind::Credits),
    ("not enough credits", Kind::Credits),
    ("insufficient balance", Kind::Credits),
    ("out of credits", Kind::Credits),
    ("paid plan", Kind::Plan),
    ("subscription required", Kind::Plan),
    ("upgrade your plan", Kind::Plan),
    ("nsfw", Kind::Content),
    ("content policy", Kind::Content),
    ("moderation", Kind::Content),
    ("unknown model", Kind::Model),
    ("model not found", Kind::Model),
    ("model is not available", Kind::Model),
    ("model unavailable", Kind::Model),
    ("model is disabled", Kind::Model),
    ("rate limit", Kind::Busy),
    ("too many requests", Kind::Busy),
    ("too many concurrent", Kind::Busy),
    ("concurrency limit", Kind::Busy),
    // Recorded 2026-10-14: `Invalid request: unknown model "…". Use …`.
    ("invalid request", Kind::Invalid),
    ("validation", Kind::Invalid),
];

#[derive(Debug, Clone, Copy)]
enum Kind {
    Credits,
    Plan,
    Content,
    Model,
    Busy,
    Invalid,
}

fn from_kind(kind: Kind, message: &str) -> CloudError {
    let message = quoted(message);
    match kind {
        Kind::Credits => CloudError::InsufficientCredits,
        Kind::Plan => CloudError::PlanRequired,
        Kind::Content => CloudError::ContentRefused { message },
        Kind::Model => CloudError::ModelUnavailable { message },
        Kind::Busy => CloudError::Busy,
        Kind::Invalid => CloudError::InvalidRequest { message },
    }
}

/// Classifies a service error message, with its structured payload if any.
pub fn classify_service_message(message: &str, structured: Option<&Value>) -> CloudError {
    if let Some(value) = structured {
        for code in error_codes(value) {
            let code = code.to_ascii_lowercase();
            if let Some((_, kind)) = CODES.iter().find(|(c, _)| *c == code) {
                return from_kind(*kind, message);
            }
        }
    }
    let lower = message.to_lowercase();
    match PHRASES.iter().find(|(p, _)| lower.contains(p)) {
        Some((_, kind)) => from_kind(*kind, message),
        None => CloudError::ServiceError {
            message: quoted(message),
        },
    }
}

/// String values of `code`, `error_code`, `type` and `error` fields at the top
/// level or inside an `error` object.
fn error_codes(value: &Value) -> Vec<&str> {
    fn scan<'a>(v: &'a Value, out: &mut Vec<&'a str>) {
        for key in ["code", "error_code", "type", "error"] {
            if let Some(s) = v.get(key).and_then(Value::as_str) {
                out.push(s);
            }
        }
    }
    let mut out = Vec::new();
    scan(value, &mut out);
    if let Some(inner) = value.get("error") {
        scan(inner, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::jsonrpc::RpcError;
    use crate::transport::TransportError;
    use serde_json::json;

    fn transport(kind: TransportErrorKind, send: SendState) -> McpError {
        McpError::Transport(TransportError {
            kind,
            send,
            detail: String::new(),
        })
    }

    #[test]
    fn the_recorded_unknown_model_message_is_a_model_error() {
        let msg = "Invalid request: unknown model \"freecut_no_such_model\". Use models_explore(action:'list') to see available models.";
        assert!(matches!(
            classify_service_message(msg, None),
            CloudError::ModelUnavailable { .. }
        ));
    }

    #[test]
    fn phrases_and_codes() {
        let c = |m: &str| classify_service_message(m, None);
        assert_eq!(
            c("Insufficient credits for this generation"),
            CloudError::InsufficientCredits
        );
        assert_eq!(
            c("This feature requires a paid plan"),
            CloudError::PlanRequired
        );
        assert!(matches!(
            c("Blocked: NSFW content"),
            CloudError::ContentRefused { .. }
        ));
        assert_eq!(c("Too many requests, slow down"), CloudError::Busy);
        assert!(matches!(
            c("Invalid request: count must be <= 4"),
            CloudError::InvalidRequest { .. }
        ));
        assert!(matches!(
            c("something odd"),
            CloudError::ServiceError { .. }
        ));
        let structured = json!({"error": {"code": "INSUFFICIENT_CREDITS"}});
        assert_eq!(
            classify_service_message("whatever", Some(&structured)),
            CloudError::InsufficientCredits
        );
    }

    #[test]
    fn transport_failures_split_on_send_state() {
        assert_eq!(
            classify(&transport(TransportErrorKind::Connect, SendState::NotSent)),
            CloudError::Offline
        );
        assert_eq!(
            classify(&transport(
                TransportErrorKind::Timeout,
                SendState::MaybeSent
            )),
            CloudError::NoAnswer
        );
        assert_eq!(
            classify(&transport(
                TransportErrorKind::Cancelled,
                SendState::MaybeSent
            )),
            CloudError::Cancelled
        );
    }

    #[test]
    fn statuses() {
        let http = |status| McpError::Http {
            status,
            excerpt: String::new(),
        };
        assert_eq!(classify(&http(402)), CloudError::InsufficientCredits);
        assert_eq!(classify(&http(429)), CloudError::Busy);
        assert_eq!(
            classify(&http(503)),
            CloudError::ServiceError {
                message: "HTTP 503".into()
            }
        );
    }

    #[test]
    fn paid_dispositions_follow_cg_010() {
        use SubmitError::*;
        let d = |e: &McpError| submit_disposition(e);
        assert!(matches!(
            d(&transport(TransportErrorKind::Connect, SendState::NotSent)),
            NotSent(CloudError::Offline)
        ));
        assert!(matches!(
            d(&transport(
                TransportErrorKind::Timeout,
                SendState::MaybeSent
            )),
            OutcomeUnknown(_)
        ));
        assert!(matches!(
            d(&transport(TransportErrorKind::Io, SendState::MaybeSent)),
            OutcomeUnknown(_)
        ));
        assert!(matches!(
            d(&McpError::Cancelled {
                send: SendState::MaybeSent
            }),
            OutcomeUnknown(_)
        ));
        assert!(matches!(
            d(&McpError::Cancelled {
                send: SendState::NotSent
            }),
            NotSent(_)
        ));
        assert!(matches!(
            d(&McpError::NotAccepted(NotAcceptedReason::Unauthorized)),
            Rejected(CloudError::NotAccepted { .. })
        ));
        assert!(matches!(
            d(&McpError::Unauthorized { challenge: None }),
            Rejected(CloudError::SignInExpired)
        ));
        assert!(matches!(
            d(&McpError::Http {
                status: 429,
                excerpt: String::new()
            }),
            Rejected(CloudError::Busy)
        ));
        assert!(matches!(
            d(&McpError::Http {
                status: 502,
                excerpt: String::new()
            }),
            OutcomeUnknown(_)
        ));
        assert!(matches!(
            d(&McpError::Protocol { detail: "x".into() }),
            OutcomeUnknown(_)
        ));
        assert!(matches!(
            d(&McpError::Rpc(RpcError {
                code: -32000,
                message: "Not enough credits".into(),
                data: None
            })),
            Rejected(CloudError::InsufficientCredits)
        ));
        assert!(matches!(
            d(&McpError::Tool {
                message: "NSFW".into(),
                structured: None
            }),
            Rejected(CloudError::ContentRefused { .. })
        ));
        assert!(matches!(
            d(&McpError::SignIn(CloudError::NotConnected)),
            NotSent(_)
        ));
    }

    #[test]
    fn quoted_text_is_plain_and_short() {
        assert_eq!(quoted("a\u{7}b\n\n  c"), "a b c");
        let long = "x".repeat(600);
        assert_eq!(quoted(&long).chars().count(), MAX_MESSAGE_CHARS + 1);
    }
}
