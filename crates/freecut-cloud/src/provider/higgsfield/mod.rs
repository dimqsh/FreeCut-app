//! Higgsfield's MCP server as a [`GenerationProvider`] (architecture §1, §6).

pub mod allow;
pub mod map;
pub mod reviewed;

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use freecut_media::cancel::CancelToken;
use serde_json::{Value, json};

use crate::catalogue::{Catalogue, parse_listing};
use crate::credits::Credits;
use crate::error::{CloudError, SubmitError, classify, quoted, submit_disposition};
use crate::guard::{CheckReservation, PreflightGuard, QuoteRequest, schema_offers_get_cost};
use crate::mcp::{McpClient, ServerInfo, TokenSource, ToolInfo, schema};
use crate::oauth::discovery::AuthPolicy;
use crate::provider::{
    Adjustment, GenerationProvider, Operation, PossiblyStarted, Quote, QuoteOutcome, ServiceInfo,
    ServiceQuestion, StartedRun, Unavailable,
};
use crate::secret::ProviderId;
use crate::transport::{HttpTransport, Waiter};

/// "The only official connector URL" (Higgsfield help center).
pub const ENDPOINT: &str = "https://mcp.higgsfield.ai/mcp";
/// Every sign-in endpoint must be on this domain.
pub const TRUSTED_DOMAIN: &str = "higgsfield.ai";
/// The scopes the service lists (`offline_access` brings a refresh token).
pub const SCOPES: &[&str] = &["openid", "email", "offline_access"];
/// FreeCut's default image model (CG-003, re-checked each release).
pub const DEFAULT_IMAGE_MODEL: &str = "gpt_image_2_5";
/// FreeCut's default video model.
pub const DEFAULT_VIDEO_MODEL: &str = "seedance_2_5";
/// Variants one estimate covers. Recorded 2026-10-14: `generate_image` with
/// `count: 2` answered the same figure as `count: 1`, so the figure is not
/// known to cover more than one variant (CG-005).
pub const VARIANTS_COVERED_BY_QUOTE: u8 = 1;

const CATALOGUE_TYPES: &[&str] = &["image", "video", "audio"];
const MAX_CATALOGUE_PAGES: usize = 10;

/// The sign-in policy for Higgsfield.
pub fn auth_policy() -> AuthPolicy {
    AuthPolicy {
        resource: ENDPOINT.into(),
        trusted_domain: TRUSTED_DOMAIN.into(),
        scopes: SCOPES.iter().map(|s| (*s).to_owned()).collect(),
    }
}

/// The Higgsfield provider. Constructing it sends nothing; the first request
/// is made by [`GenerationProvider::connect`] or a later call.
pub struct HiggsfieldProvider {
    mcp: McpClient,
    tools: Mutex<Option<BTreeMap<String, ToolInfo>>>,
    catalogue: Mutex<Option<Catalogue>>,
    guard: Arc<PreflightGuard>,
}

impl fmt::Debug for HiggsfieldProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HiggsfieldProvider")
            .field("mcp", &self.mcp)
            .finish_non_exhaustive()
    }
}

impl HiggsfieldProvider {
    /// A provider for Higgsfield's endpoint. `guard` is the app's preflight
    /// guard, shared by every provider it creates during its run (§4.7,
    /// review R6).
    pub fn new(
        transport: Arc<dyn HttpTransport>,
        tokens: Arc<dyn TokenSource>,
        waiter: Arc<dyn Waiter>,
        client_version: &str,
        guard: Arc<PreflightGuard>,
    ) -> HiggsfieldProvider {
        HiggsfieldProvider::with_endpoint(
            ENDPOINT,
            transport,
            tokens,
            waiter,
            client_version,
            guard,
        )
    }

    /// A provider for another endpoint (tests against a fake service).
    pub fn with_endpoint(
        endpoint: &str,
        transport: Arc<dyn HttpTransport>,
        tokens: Arc<dyn TokenSource>,
        waiter: Arc<dyn Waiter>,
        client_version: &str,
        guard: Arc<PreflightGuard>,
    ) -> HiggsfieldProvider {
        HiggsfieldProvider {
            mcp: McpClient::new(endpoint, transport, tokens, waiter, client_version),
            tools: Mutex::new(None),
            catalogue: Mutex::new(None),
            guard,
        }
    }

    /// What the server said about itself in the last handshake (for the
    /// debug view; its `instructions` are never acted on).
    pub fn server_info(&self) -> Option<ServerInfo> {
        self.mcp.server()
    }

    /// The preflight guard's state.
    pub fn guard(&self) -> &PreflightGuard {
        &self.guard
    }

    /// The catalogue from the last load, if any.
    pub fn cached_catalogue(&self) -> Option<Catalogue> {
        self.catalogue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Ends the session (disconnect, quit) and forgets the tool list.
    pub fn close(&self) {
        self.mcp.close();
        *self.tools.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }

    fn tools(&self, cancel: &CancelToken) -> Result<BTreeMap<String, ToolInfo>, CloudError> {
        if let Some(tools) = self
            .tools
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
        {
            return Ok(tools);
        }
        self.connect(cancel)?;
        Ok(self
            .tools
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .unwrap_or_default())
    }
}

impl GenerationProvider for HiggsfieldProvider {
    fn id(&self) -> ProviderId {
        ProviderId::HIGGSFIELD
    }

    fn connect(&self, cancel: &CancelToken) -> Result<ServiceInfo, CloudError> {
        let server = self.mcp.initialize(cancel).map_err(|e| classify(&e))?;
        let listed = self.mcp.list_tools(cancel).map_err(|e| classify(&e))?;
        let allowed: BTreeMap<String, ToolInfo> = listed
            .into_iter()
            .filter(|t| allow::lookup(&t.name).is_some())
            .map(|t| (t.name.clone(), t))
            .collect();
        let missing = allow::NEEDED_NOW
            .iter()
            .filter(|n| !allowed.contains_key(**n))
            .map(|n| (*n).to_owned())
            .collect();
        let info = ServiceInfo {
            server_name: server.name,
            server_version: server.version,
            protocol_version: server.protocol_version,
            offered: allowed.keys().cloned().collect(),
            missing,
        };
        *self.tools.lock().unwrap_or_else(PoisonError::into_inner) = Some(allowed);
        Ok(info)
    }

    fn catalogue(&self, cancel: &CancelToken) -> Result<Catalogue, CloudError> {
        let mut pages = Vec::new();
        for kind in CATALOGUE_TYPES {
            let mut after: Option<String> = None;
            for _ in 0..MAX_CATALOGUE_PAGES {
                let mut args = json!({ "action": "list", "type": kind, "limit": 100 });
                if let Some(cursor) = &after {
                    args["after"] = Value::String(cursor.clone());
                }
                let output = self
                    .mcp
                    .call_tool(&allow::MODELS_EXPLORE, args, cancel)
                    .map_err(|e| classify(&e))?;
                let listing = output.value().ok_or_else(|| CloudError::Protocol {
                    detail: "empty model listing".into(),
                })?;
                let page = parse_listing(&listing).map_err(|e| CloudError::Protocol {
                    detail: e.to_string(),
                })?;
                after = page.next_page.clone();
                pages.push(page);
                if after.is_none() {
                    break;
                }
            }
        }
        let mut catalogue = Catalogue::from_pages(pages);
        // Only reviewed models are offered (owner decision, review round 2).
        reviewed::restrict_to_reviewed(&mut catalogue);
        if catalogue.skipped() > 0 {
            tracing::warn!(
                skipped = catalogue.skipped(),
                "some catalogue entries could not be read"
            );
        }
        *self
            .catalogue
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(catalogue.clone());
        Ok(catalogue)
    }

    fn quote(&self, op: &Operation, cancel: &CancelToken) -> Result<QuoteOutcome, CloudError> {
        if op.variants() > VARIANTS_COVERED_BY_QUOTE {
            return Ok(QuoteOutcome::NotAvailable(
                Unavailable::VariantsNotCovered {
                    max: VARIANTS_COVERED_BY_QUOTE,
                },
            ));
        }
        let catalogue = match self.cached_catalogue() {
            Some(c) => c,
            None => self.catalogue(cancel)?,
        };
        let model = catalogue
            .get(op.model().as_str())
            .filter(|m| m.is_visible())
            .ok_or_else(|| CloudError::ModelUnavailable {
                message: format!("{} is not in the service's model list", op.model()),
            })?;
        let call = map::tool_call(op, model).map_err(|e| CloudError::NotOffered {
            what: e.to_string(),
        })?;
        let tools = self.tools(cancel)?;
        let Some(tool) = tools.get(call.tool.name()) else {
            return Ok(QuoteOutcome::NotAvailable(Unavailable::ToolMissing {
                tool: call.tool.name().to_owned(),
            }));
        };
        if !schema_offers_get_cost(&tool.input_schema) {
            return Ok(QuoteOutcome::NotAvailable(Unavailable::NoEstimate));
        }
        if self.guard.is_blocked(call.tool.name()) {
            return Ok(QuoteOutcome::NotAvailable(Unavailable::PreflightStopped));
        }
        let request = QuoteRequest::new(call.tool.name(), call.arguments).map_err(|e| {
            CloudError::GuardRefused {
                detail: e.to_string(),
            }
        })?;
        schema::validate(&tool.input_schema, request.arguments()).map_err(|v| {
            CloudError::SchemaDrift {
                tool: call.tool.name().to_owned(),
                detail: v.to_string(),
            }
        })?;
        // Reserved under one lock with the test: a second check of this tool
        // while one is in flight is refused (review R6).
        let Ok(reservation) = self.guard.reserve(call.tool.name()) else {
            return Ok(QuoteOutcome::NotAvailable(Unavailable::PreflightStopped));
        };
        let check = Check {
            tool: call.tool.name(),
            arguments: request.arguments(),
            reservation,
        };
        let output = match self.mcp.check_cost(call.tool, &request, cancel) {
            Ok(output) => output,
            Err(e) => {
                return match submit_disposition(&e) {
                    // An answer that named jobs, readable or not (R4, R5).
                    SubmitError::OutcomeUnknown { job_ids, .. } if !job_ids.is_empty() => {
                        Ok(check.started_run(job_ids))
                    }
                    SubmitError::OutcomeUnknown { cause, .. } => Ok(check.possibly_started(cause)),
                    SubmitError::NotSent(cause) | SubmitError::Rejected(cause) => {
                        check.reservation.release();
                        Err(cause)
                    }
                };
            }
        };
        // Every content item, not only the one read as the answer (R5).
        let job_ids = output.job_ids();
        if !job_ids.is_empty() {
            return Ok(check.started_run(job_ids));
        }
        let Some(answer) = output.value() else {
            return Ok(check.possibly_started(CloudError::Protocol {
                detail: "empty cost answer".into(),
            }));
        };
        match parse_quote_answer(&answer, check.tool, check.arguments) {
            Ok(outcome) => {
                check.reservation.release();
                Ok(outcome)
            }
            // Fail closed (review M1): an answer FreeCut cannot read may hide
            // a paid run.
            Err(cause) => Ok(check.possibly_started(cause)),
        }
    }
}

/// A cost check on the wire, holding its tool's reservation.
struct Check<'a> {
    tool: &'a str,
    arguments: &'a Value,
    reservation: CheckReservation<'a>,
}

impl Check<'_> {
    /// The check named job ids: a run started. The ids go to the ledger
    /// (§4.7 rule 4) and this tool's checks stop until restart.
    fn started_run(self, job_ids: Vec<String>) -> QuoteOutcome {
        self.reservation.block();
        tracing::warn!(
            tool = self.tool,
            jobs = job_ids.len(),
            "a cost check started a run; cost checks for this tool are off until restart"
        );
        QuoteOutcome::StartedRun(StartedRun {
            tool: self.tool.to_owned(),
            job_ids,
            arguments: self.arguments.clone(),
        })
    }

    /// The answer was lost or could not be read: it may hide a run, so this
    /// tool's checks stop until restart (review M1).
    fn possibly_started(self, cause: CloudError) -> QuoteOutcome {
        self.reservation.block();
        tracing::warn!(
            tool = self.tool,
            error = %cause,
            "a cost check may have started a run; cost checks for this tool are off until restart"
        );
        QuoteOutcome::PossiblyStarted(PossiblyStarted {
            tool: self.tool.to_owned(),
            arguments: self.arguments.clone(),
            cause,
        })
    }
}

/// Reads a cost-check answer (recorded shape: `{"cost": {"credits": 35,
/// "credits_exact": 35}}`), or the service's question or follow-up.
///
/// # Errors
/// [`CloudError::Protocol`] when it holds none of them, or a cost FreeCut
/// cannot read; the provider then treats the check as possibly started.
pub fn parse_quote_answer(
    answer: &Value,
    tool: &str,
    arguments: &Value,
) -> Result<QuoteOutcome, CloudError> {
    if answer.get("unlim_choice").is_some() {
        // The tool description: an allowance-paid run "is always one
        // generation".
        return Ok(QuoteOutcome::Question(ServiceQuestion::UseAllowance {
            cap_variants_to: Some(1),
        }));
    }
    if let Some(follow) = answer.get("recovery_tool") {
        let name = follow
            .as_str()
            .or_else(|| follow.get("name").and_then(Value::as_str))
            .unwrap_or_default();
        return Ok(QuoteOutcome::NotAvailable(Unavailable::FollowUp {
            tool: quoted(name),
        }));
    }
    let cost = answer.get("cost").ok_or_else(|| CloudError::Protocol {
        detail: "the cost answer has no cost".into(),
    })?;
    let figure = cost
        .get("credits_exact")
        .or_else(|| cost.get("credits"))
        .unwrap_or(cost);
    let credits = Credits::from_json(figure).map_err(|e| CloudError::Protocol {
        detail: e.to_string(),
    })?;
    let adjustments = answer
        .get("adjustments")
        .and_then(Value::as_array)
        .map(|items| items.iter().map(adjustment).collect())
        .unwrap_or_default();
    Ok(QuoteOutcome::Quote(Quote {
        credits,
        variants_covered: VARIANTS_COVERED_BY_QUOTE,
        adjustments,
        tool: tool.to_owned(),
        arguments: arguments.clone(),
    }))
}

/// FreeCut's summary of one adjustment (the exact shape is recorded in H2).
fn adjustment(item: &Value) -> Adjustment {
    let text = |keys: &[&str]| {
        keys.iter()
            .find_map(|k| item.get(*k))
            .map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned))
    };
    let summary = match (
        text(&["field", "param", "name"]),
        text(&["requested", "from"]),
        text(&["applied", "effective", "to"]),
    ) {
        (Some(field), Some(from), Some(to)) => format!("{field}: {from} → {to}"),
        _ => item
            .as_str()
            .map_or_else(|| item.to_string(), str::to_owned),
    };
    Adjustment {
        summary: quoted(&summary),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorded_cost_answers() {
        let args = json!({"params": {"get_cost": true}});
        let QuoteOutcome::Quote(q) = parse_quote_answer(
            &json!({"cost": {"credits": 0.25, "credits_exact": 0.25}}),
            "generate_image",
            &args,
        )
        .unwrap() else {
            panic!("expected a quote");
        };
        assert_eq!(q.credits, Credits::from_millis(250));
        assert_eq!(q.variants_covered, 1);
        let QuoteOutcome::Quote(q) = parse_quote_answer(
            &json!({"cost": {"credits": 35, "credits_exact": 35}}),
            "generate_video",
            &args,
        )
        .unwrap() else {
            panic!("expected a quote");
        };
        assert_eq!(q.credits, Credits::from_millis(35_000));
    }

    #[test]
    fn questions_follow_ups_and_adjustments() {
        let args = json!({});
        assert_eq!(
            parse_quote_answer(&json!({"unlim_choice": {"remaining": 3}}), "t", &args).unwrap(),
            QuoteOutcome::Question(ServiceQuestion::UseAllowance {
                cap_variants_to: Some(1)
            })
        );
        assert_eq!(
            parse_quote_answer(
                &json!({"recovery_tool": {"name": "models_explore"}}),
                "t",
                &args
            )
            .unwrap(),
            QuoteOutcome::NotAvailable(Unavailable::FollowUp {
                tool: "models_explore".into()
            })
        );
        let QuoteOutcome::Quote(q) = parse_quote_answer(
            &json!({"cost": {"credits": 8}, "adjustments": [{"field": "duration", "requested": 7, "applied": 8}, "count capped to 1"]}),
            "t",
            &args,
        )
        .unwrap() else {
            panic!("expected a quote");
        };
        assert_eq!(
            q.adjustments
                .iter()
                .map(|a| a.summary.as_str())
                .collect::<Vec<_>>(),
            vec!["duration: 7 → 8", "count capped to 1"]
        );
        assert!(matches!(
            parse_quote_answer(&json!({"ok": true}), "t", &args),
            Err(CloudError::Protocol { .. })
        ));
    }
}
