//! A [`GenerationProvider`] that answers from a script (feature
//! `test-support`; architecture §6.1). App and assistant tests drive the
//! Generate tab, run sheet and placement with it, without the service, the
//! network or the fake MCP server.
//!
//! Each method takes the next scripted answer for that method, in order. A
//! cancelled token wins, as on the real provider, and leaves the script as it
//! was. A method with nothing left answers [`CloudError::Protocol`] naming the
//! method, so a missing step fails the test instead of hanging a worker.
//! Every call is recorded, cancelled or not.

use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard, PoisonError};

use freecut_media::cancel::CancelToken;

use super::{GenerationProvider, Operation, QuoteOutcome, ServiceInfo};
use crate::catalogue::Catalogue;
use crate::error::CloudError;
use crate::mcp::RunTag;
use crate::secret::ProviderId;

/// A call a [`ScriptedProvider`] received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptedCall {
    /// [`GenerationProvider::connect`].
    Connect,
    /// [`GenerationProvider::catalogue`].
    Catalogue,
    /// [`GenerationProvider::quote`], with its operation and run.
    Quote(Operation, RunTag),
}

#[derive(Debug, Default)]
struct Script {
    connect: VecDeque<Result<ServiceInfo, CloudError>>,
    catalogue: VecDeque<Result<Catalogue, CloudError>>,
    quote: VecDeque<Result<QuoteOutcome, CloudError>>,
    calls: Vec<ScriptedCall>,
}

/// A provider that answers from a script; see the module docs.
#[derive(Debug)]
pub struct ScriptedProvider {
    id: ProviderId,
    script: Mutex<Script>,
}

impl ScriptedProvider {
    /// A provider reporting `id`, with nothing scripted yet.
    pub fn new(id: ProviderId) -> ScriptedProvider {
        ScriptedProvider {
            id,
            script: Mutex::new(Script::default()),
        }
    }

    /// Queues the next answer to `connect`.
    pub fn push_connect(&self, answer: Result<ServiceInfo, CloudError>) -> &ScriptedProvider {
        self.script().connect.push_back(answer);
        self
    }

    /// Queues the next answer to `catalogue`.
    pub fn push_catalogue(&self, answer: Result<Catalogue, CloudError>) -> &ScriptedProvider {
        self.script().catalogue.push_back(answer);
        self
    }

    /// Queues the next answer to `quote`.
    pub fn push_quote(&self, answer: Result<QuoteOutcome, CloudError>) -> &ScriptedProvider {
        self.script().quote.push_back(answer);
        self
    }

    /// The calls received so far, in order.
    pub fn calls(&self) -> Vec<ScriptedCall> {
        self.script().calls.clone()
    }

    /// Whether every scripted answer has been used.
    pub fn is_finished(&self) -> bool {
        let script = self.script();
        script.connect.is_empty() && script.catalogue.is_empty() && script.quote.is_empty()
    }

    fn script(&self) -> MutexGuard<'_, Script> {
        // A test that panicked while holding the lock has already failed;
        // keep the script readable for its report.
        self.script.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn next<T>(
    answers: &mut VecDeque<Result<T, CloudError>>,
    method: &str,
    cancel: &CancelToken,
) -> Result<T, CloudError> {
    if cancel.is_cancelled() {
        return Err(CloudError::Cancelled);
    }
    answers.pop_front().unwrap_or_else(|| {
        Err(CloudError::Protocol {
            detail: format!("ScriptedProvider: nothing scripted for {method}"),
        })
    })
}

impl GenerationProvider for ScriptedProvider {
    fn id(&self) -> ProviderId {
        self.id
    }

    fn connect(&self, cancel: &CancelToken) -> Result<ServiceInfo, CloudError> {
        let mut guard = self.script();
        let script = &mut *guard;
        script.calls.push(ScriptedCall::Connect);
        next(&mut script.connect, "connect", cancel)
    }

    fn catalogue(&self, cancel: &CancelToken) -> Result<Catalogue, CloudError> {
        let mut guard = self.script();
        let script = &mut *guard;
        script.calls.push(ScriptedCall::Catalogue);
        next(&mut script.catalogue, "catalogue", cancel)
    }

    fn quote(
        &self,
        op: &Operation,
        run: &RunTag,
        cancel: &CancelToken,
    ) -> Result<QuoteOutcome, CloudError> {
        let mut guard = self.script();
        let script = &mut *guard;
        script
            .calls
            .push(ScriptedCall::Quote(op.clone(), run.clone()));
        next(&mut script.quote, "quote", cancel)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::*;
    use crate::catalogue::ModelId;
    use crate::credits::Credits;
    use crate::provider::{ImageRequest, ParamValues, Quote, ServiceQuestion};

    fn run() -> RunTag {
        RunTag::new("run-1")
    }

    fn op(prompt: &str) -> Operation {
        Operation::Image(ImageRequest {
            model: ModelId::new("model_a"),
            prompt: prompt.into(),
            refs: vec![],
            aspect: None,
            variants: 1,
            params: ParamValues::new(),
        })
    }

    fn info() -> ServiceInfo {
        ServiceInfo {
            server_name: "scripted".into(),
            server_version: "1".into(),
            protocol_version: "2025-06-18".into(),
            offered: vec!["generate_image".into()],
            missing: vec![],
        }
    }

    #[test]
    fn answers_come_in_script_order_and_calls_are_recorded() {
        let quote = QuoteOutcome::Quote(Quote {
            credits: Credits::parse_decimal("2.5").unwrap(),
            variants_covered: 1,
            adjustments: vec![],
            tool: "generate_image".into(),
            arguments: json!({"get_cost": true}),
        });
        let scripted = ScriptedProvider::new(ProviderId::HIGGSFIELD);
        scripted
            .push_connect(Ok(info()))
            .push_catalogue(Ok(Catalogue::default()))
            .push_quote(Ok(quote.clone()))
            .push_quote(Ok(QuoteOutcome::Question(ServiceQuestion::UseAllowance {
                cap_variants_to: None,
            })))
            .push_quote(Err(CloudError::InsufficientCredits));

        // Used the way the app uses a provider: shared, behind the trait.
        let provider: Arc<dyn GenerationProvider> = Arc::new(scripted);
        let none = CancelToken::new();
        let run = run();
        assert_eq!(provider.id(), ProviderId::HIGGSFIELD);
        assert_eq!(provider.connect(&none).unwrap(), info());
        assert_eq!(provider.catalogue(&none).unwrap(), Catalogue::default());
        assert_eq!(provider.quote(&op("a"), &run, &none).unwrap(), quote);
        assert!(matches!(
            provider.quote(&op("b"), &run, &none).unwrap(),
            QuoteOutcome::Question(_)
        ));
        assert_eq!(
            provider.quote(&op("c"), &run, &none).unwrap_err(),
            CloudError::InsufficientCredits
        );
    }

    #[test]
    fn cancel_wins_and_keeps_the_script_and_a_missing_step_is_an_error() {
        let scripted = ScriptedProvider::new(ProviderId::HIGGSFIELD);
        scripted.push_quote(Err(CloudError::Busy));
        let run = run();
        let cancelled = CancelToken::new();
        cancelled.cancel();
        assert_eq!(
            scripted.quote(&op("a"), &run, &cancelled).unwrap_err(),
            CloudError::Cancelled
        );
        assert!(!scripted.is_finished());

        let none = CancelToken::new();
        assert_eq!(
            scripted.quote(&op("b"), &run, &none).unwrap_err(),
            CloudError::Busy
        );
        assert!(scripted.is_finished());
        assert!(matches!(
            scripted.connect(&none),
            Err(CloudError::Protocol { detail }) if detail.contains("connect")
        ));
        assert_eq!(
            scripted.calls(),
            vec![
                ScriptedCall::Quote(op("a"), run.clone()),
                ScriptedCall::Quote(op("b"), run.clone()),
                ScriptedCall::Connect,
            ]
        );
    }
}
