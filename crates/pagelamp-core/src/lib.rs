//! PageLamp core: the local, read-mostly course knowledge base.
//!
//! Layering (see `docs/ARCHITECTURE.md`):
//! - `model`    — plain data types shared by every crate (also the MCP tool output types).
//! - `store`    — the SQLite store. The CLI/sync process is the only writer of synced data;
//!   MCP server processes open it read-only (plus one tiny write path for study plans).
//! - `ingest`   — sync-time extract → chunk → FTS index (heavy work never runs in MCP calls).
//! - `dates`    — instants to course calendar dates (course time zone), Monday alignment.
//! - `calendar` — course calendars the student accepted or typed (types, legacy overrides).
//! - `term`     — which dates count a course's weeks (plausibility, anchors, phases).
//! - `lifecycle` — upcoming / current / finishing / ended, and removal suggestions.
//! - `timeline` — pure functions that infer "which week is this course in" with evidence.
//! - `views`    — read views shared by the App facade and the MCP server.
//! - `diagnostics` — local log files, redaction, crash capture (logs never leave the device).
//! - `brand`    — product naming for user-facing text (edited by distributions).
//! - `paths`    — where data lives on disk (`PAGELAMP_HOME` overrides everything).
//! - `source`   — error type + progress callback shared by the sync sources.
//! - `secrets`  — OS keychain access for Canvas tokens / calendar-feed URLs. Never used by MCP.
//! - `ai`       — names of PageLamp's own model calls (block reasons, model errors, effort).
//! - `ai_gate`  — the course AI policy gate: the only producer of prompt text (v0.3 M1).
//! - `ai_rules` — the four core rules every AI is told (MCP server and PageLamp's own prompts).
//! - `planner`  — the deterministic study-plan scheduler (pure).

pub mod ai;
pub mod ai_gate;
pub mod ai_rules;
pub mod brand;
pub mod calendar;
pub mod dates;
pub mod diagnostics;
pub mod error;
pub mod ingest;
pub mod lifecycle;
pub mod model;
pub mod paths;
pub mod planner;
pub mod reminders;
pub mod removal;
pub mod secrets;
pub mod source;
pub mod store;
pub mod term;
pub mod timeline;
pub mod views;

pub use error::{Error, Result};
pub use store::Store;
