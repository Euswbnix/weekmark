//! PageLamp's own model calls (v0.3 M1; docs/design/v0.3-model-access.md §2.5–§3.7).
//!
//! GUARDRAILS — product requirements, not suggestions:
//! - A driver sends nothing but a `pagelamp_core::ai_gate::RenderedPrompt`: the course AI policy
//!   gate is enforced by types (ARCHITECTURE rule 9). No tools, ever, in v0.3.0.
//! - Only `pagelamp-app` and the CLI use this crate. `pagelamp-mcp` and `pagelamp-core` never
//!   depend on it (a test in pagelamp-app checks the crate graph), so `pagelamp mcp` makes no
//!   model call and no network call.
//! - Keys (`ApiKey`) come from `pagelamp_core::secrets` (`llm:<provider_id>`) and are never
//!   logged, stored in the database or put into an error message; the HTTP header carrying one
//!   is marked sensitive.
//! - HTTPS for every base URL that isn't loopback, and no redirects at all, so a key can never
//!   follow a redirect to another host.
//! - An honest `User-Agent: PageLamp/<version>`; client checks are never worked around.
//!   Coding-plan endpoints and keys are refused (`profile::check_endpoint`).
//! - Retries only before the first streamed byte; errors after text was shown are the caller's
//!   "Try again", so nobody is billed twice silently.
//! - Logs carry codes, statuses and lengths — never prompt text, model output or keys.
//!
//! Layout: `profile` (provider presets and the coding-plan block list, data files in `data/`),
//! `client` (the HTTP rules), `sse` (event-stream framing), `wire` (one module per wire
//! dialect), `backend` (the closed `Backend` enum that runs a request), `error`.

pub mod backend;
pub mod catalog;
mod client;
pub mod codex;
pub mod error;
pub mod estimate;
pub mod output;
pub mod profile;
pub mod request;
mod retry;
mod sse;
mod wire;

pub use backend::{Backend, HttpDriver};
pub use client::{BaseUrlProblem, check_base_url, is_loopback};
pub use error::{LlmError, ModelError};
pub use profile::{ApiKey, ProviderProfile, Wire};
pub use request::{GenerateRequest, Outcome, OutputSpec, StopReason, StreamEvent, Usage};
pub use tokio_util::sync::CancellationToken;
