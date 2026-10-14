//! Opt-in cloud generation through the user's own account on a third-party
//! service (Higgsfield first). Design: `docs/architecture/cloud-generation.md`;
//! features CG-001…CG-026 in `docs/features/cloud-generation.md`.
//!
//! What it provides (slice H1 core):
//! - [`mcp`]: a JSON-RPC 2.0 client for the Model Context Protocol over
//!   Streamable HTTP (single JSON or SSE answers, session ids, cancellation
//!   between chunks, retries only where §4.5 allows them);
//! - [`oauth`]: discovery, dynamic client registration, PKCE, the authorize
//!   URL and loopback callback check, code exchange, refresh, device code and
//!   revocation (§5). The app owns the loopback listener and the browser;
//! - [`secret`]: [`Credential`](secret::Credential), the
//!   [`SecretStore`](secret::SecretStore) trait and a store that splits long
//!   tokens over several keychain entries;
//! - [`catalogue`]: the service's model listing as [`Catalogue`](catalogue::Catalogue),
//!   with filters by output type and input role;
//! - [`provider`]: the provider-neutral [`GenerationProvider`](provider::GenerationProvider)
//!   trait and its Higgsfield implementation (tool allow list, operation
//!   mapping, cost preflight);
//! - [`guard`]: the preflight guard of §4.7;
//! - [`error`]: [`CloudError`](error::CloudError) and the classifiers that turn
//!   protocol failures into it;
//! - [`fake`]: an in-memory fake of the service for tests (no sockets);
//! - `provider::scripted` (feature `test-support`): a provider that answers
//!   from a script, for app and assistant tests.
//!
//! **Money.** Paid calls are never retried or resent by this crate, and a
//! paid request that may have reached the service without a readable answer
//! is reported as an unknown outcome (architecture §4.5–§4.7, CG-010).
//!
//! **I/O.** Everything here blocks; call it from the app's cloud worker,
//! never the UI thread. Without the `http` feature the crate has no
//! networking code: the only transports are the ones a caller supplies.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod catalogue;
pub mod credits;
pub mod error;
pub mod fake;
pub mod guard;
#[cfg(feature = "http")]
pub mod http;
pub mod mcp;
pub mod oauth;
pub mod provider;
pub mod redact;
pub mod secret;
pub mod transport;

pub use freecut_media::cancel::CancelToken;
