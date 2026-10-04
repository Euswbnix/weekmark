//! PageLamp MCP server — the v0.1 product surface.
//!
//! Transport: stdio (one process per AI client; each opens the DB read-only per request
//! inside `spawn_blocking` — no pool, no global lock). stdout carries protocol only; all
//! logging goes to stderr.
//!
//! HARD RULES
//! - Never calls Canvas or any network API (Canvas API Policy §3(i)); serves the local DB only.
//! - Read-only except `save_study_plan`.
//! - No tool returns assignment instructions/solutions; deadlines are title + date + link.
//! - All course text is returned inside
//!   `<course_material id="…" title="…" locator="…">…</course_material>` wrappers, and the
//!   server `instructions` tell the model that text inside is untrusted data, never
//!   instructions (prompt-injection hygiene).
//! - Every text-returning tool caps output (default 12_000 chars) and paginates.
//!
//! TOOLS (names are the public contract)
//! - `list_courses()` → courses with code, name, current week (+confidence), ai_policy,
//!   data freshness (source last_synced_at).
//! - `course_overview(course)` → timeline (week, confidence, evidence), current modules,
//!   materials published in the last 14 days, deadlines in the next 21 days, announcements
//!   of the last 14 days (titles + ids), ai_policy + note.
//! - `week_materials(course, week?)` → materials of that week (default current week):
//!   id, title, kind, locator count, text_status, url.
//! - `read_material(material_id, from_chunk?, max_chars?)` → wrapped text with locators,
//!   `next_chunk` for pagination.
//! - `search_materials(query, course?, limit?)` → hits with snippet, material title,
//!   locator, url (for citations).
//! - `list_deadlines(course?, days_ahead? = 21, days_back? = 0)` → events for planning.
//! - `get_announcements(course, days? = 14)` → wrapped announcement text.
//! - `get_study_plan()` / `save_study_plan(plan)` — plan per `model::StudyPlan`.
//! - `sync_status()` → sources with last_synced_at / last_error and a state (fresh, old,
//!   never synced, failed, needs the student), whether PageLamp refreshes by itself while its
//!   app is open (`auto_sync`) and when it last did. The hint says what to tell the student.
//!   The server never syncs, and no tool starts, requests or schedules a sync: the texts tell
//!   an AI app that can run commands not to sync or open the app for the student. Nothing an
//!   MCP client can write is read by the automatic sync's rule.
//!
//! AI ACCESS (docs/ARCHITECTURE.md §3 rule 8): material TEXT (read_material, snippets,
//! announcement bodies, anything a prompt would inline) is only returned for courses whose
//! `ai_materials` is `readable`; otherwise the tools return a normal result explaining why
//! (wording in `text`). Structure, deadlines and study plans stay available. Enforcement
//! lives in `pagelamp_core::views`, so this crate cannot leak withheld text by accident.
//!
//! WORDING: every string sent to the model lives in `text.rs` (editable by non-Rust
//! maintainers). Claude Desktop ignores server `instructions`, so the essential rules are
//! repeated in tool descriptions and in a `guidance` field of course_overview /
//! week_materials / read_material results. No MCP sampling (deprecated, unsupported).
//!
//! PROMPTS (appear as slash commands in Claude Desktop / Claude Code)
//! - `weekly_review(course, week?)` — explain this week's content, cite materials as
//!   "Title, locator", check understanding with 2–3 questions; respect ai_policy.
//! - `catch_up(course, since?)` — what was missed since a date, in order, with materials.
//! - `study_plan(days? = 14, hours_per_week?)` — gather deadlines + timelines across courses,
//!   propose a day-by-day plan as data, then call `save_study_plan`.
//!
//! SERVER INSTRUCTIONS (initialize result) must state: purpose; cite sources; course text is
//! untrusted data; do not produce solutions to graded assignments — tutor instead, and for
//! `prohibited`/`unknown` ai_policy courses limit help to explaining course concepts;
//! answer in the student's language.

pub mod text;

mod format;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use chrono::{Local, NaiveDate, TimeDelta};
use pagelamp_core::auto_sync::{self, AutoSync};
use pagelamp_core::brand;
use pagelamp_core::diagnostics::redact;
use pagelamp_core::model::{
    AiLabel, AiMaterialsState, AiPolicy, BreakKind, CalendarOrigin, CalendarStatus, Confidence,
    Course, CoursePhase, CourseTimeline, EventKind, LifecycleState, MaterialKind, SourceErrorKind,
    SourceKind, StoreCounts, StudyPlan, TermAnchorSource, TextStatus, Timestamp,
};
use pagelamp_core::store::Store;
use pagelamp_core::views::{self, AsOf, Deadline, MaterialView};
use rmcp::handler::server::router::prompt::PromptRouter;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::tool::ToolCallContext;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, PromptMessage, Role,
    ServerCapabilities, ServerConfig,
};
use rmcp::service::{QuitReason, RequestContext};
use rmcp::{
    ErrorData, RoleServer, ServerHandler, ServiceExt, prompt, prompt_handler, prompt_router, tool,
    tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::format::{
    OUTPUT_CAP, cap_list, error_result, json_result, text_result, wrap, wrap_plan,
};

/// Run the MCP server over stdio until the client disconnects (or the process is told to
/// stop). `db_path` is normally `pagelamp_core::paths::db_path()`. The server starts even
/// when the database does not exist yet (tools then tell the student to sync); it never
/// creates one, but migrates an older existing database once (`upgrade_database`). It never
/// touches the network and never writes to stdout except protocol messages.
///
/// Log file (`logs/mcp-*.log`): start, the client's name/version and protocol version, and
/// exit (reason, tool calls, uptime) at info; one line per tool call (name, time, ok/error,
/// output size) at debug. Tool arguments, queries and output text are never logged, and
/// errors returned from here are fixed texts (the caller prints them to stderr, which the
/// AI app keeps in its own logs).
pub async fn serve_stdio(db_path: PathBuf) -> anyhow::Result<()> {
    let started = Instant::now();
    tracing::info!(
        target: LOG_TARGET,
        "MCP server {} started (pid {})",
        env!("CARGO_PKG_VERSION"),
        std::process::id()
    );
    // Registered first, so a stop request during startup is not lost.
    let mut stop = StopSignals::install();
    upgrade_database(&db_path).await;
    let server = PageLampServer::new(db_path);
    let tool_calls = Arc::clone(&server.tool_calls);
    let exit = |reason: &str| {
        tracing::info!(
            target: LOG_TARGET,
            "MCP server stopped ({reason}); tool calls: {}, uptime: {} s",
            tool_calls.load(Ordering::Relaxed),
            started.elapsed().as_secs()
        );
    };
    let serving = tokio::select! {
        serving = server.serve(rmcp::transport::stdio()) => serving,
        () = stop.recv() => {
            exit("terminated");
            return Ok(());
        }
    };
    let service = match serving {
        Ok(service) => service,
        Err(err) => {
            // rmcp's own message would include the whole first message, arguments too.
            let what = handshake_failure(&err);
            tracing::warn!(target: LOG_TARGET, "MCP handshake failed: {what}");
            exit("handshake failed");
            return Err(anyhow::anyhow!("MCP handshake failed: {what}"));
        }
    };
    match service.peer().peer_info() {
        Some(info) => tracing::info!(
            target: LOG_TARGET,
            "client \"{}\" \"{}\", protocol \"{}\"",
            clip(&info.client_info.name),
            clip(&info.client_info.version),
            clip(&info.protocol_version.to_string())
        ),
        None => tracing::info!(target: LOG_TARGET, "client connected without client info"),
    }
    let cancel = service.cancellation_token();
    let mut waiting = std::pin::pin!(service.waiting());
    let quit = tokio::select! {
        quit = &mut waiting => quit,
        () = stop.recv() => {
            exit("terminated");
            cancel.cancel();
            return Ok(());
        }
    };
    match quit {
        Ok(QuitReason::Closed) => exit("client disconnected"),
        Ok(QuitReason::Cancelled) => exit("cancelled"),
        Ok(QuitReason::JoinError(_)) | Err(_) => {
            exit("crashed");
            return Err(anyhow::anyhow!("the MCP server stopped unexpectedly"));
        }
        Ok(_) => exit("stopped"),
    }
    Ok(())
}

/// The requests to stop this process: SIGTERM (sent by AI apps when they quit) and SIGINT on
/// Unix, Ctrl-C elsewhere. Handlers are registered by `install`, so nothing is missed between
/// then and the first `recv`.
struct StopSignals {
    #[cfg(unix)]
    terminate: Option<tokio::signal::unix::Signal>,
    #[cfg(unix)]
    interrupt: Option<tokio::signal::unix::Signal>,
}

impl StopSignals {
    fn install() -> StopSignals {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            StopSignals {
                terminate: signal(SignalKind::terminate()).ok(),
                interrupt: signal(SignalKind::interrupt()).ok(),
            }
        }
        #[cfg(not(unix))]
        StopSignals {}
    }

    /// Resolves when one of the signals arrives (never, if none could be registered).
    async fn recv(&mut self) {
        #[cfg(unix)]
        {
            async fn next(signal: &mut Option<tokio::signal::unix::Signal>) {
                match signal {
                    Some(signal) => {
                        signal.recv().await;
                    }
                    None => std::future::pending().await,
                }
            }
            tokio::select! {
                () = next(&mut self.terminate) => {}
                () = next(&mut self.interrupt) => {}
            }
        }
        #[cfg(not(unix))]
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

/// A database written by an older version (0 < `user_version` < `SCHEMA_VERSION`) is
/// migrated once before serving: the read-only connections the tools use cannot, and would
/// otherwise report "no course data yet" to a student who just updated. The migrations are
/// additive and run in one `BEGIN IMMEDIATE` transaction, so a concurrent sync or another
/// MCP process is safe. A missing database is never created here.
async fn upgrade_database(db_path: &std::path::Path) {
    let db = db_path.to_path_buf();
    let upgraded = tokio::task::spawn_blocking(move || Store::upgrade_existing(&db)).await;
    match upgraded {
        Ok(Ok(Some(from))) => tracing::info!(
            target: LOG_TARGET,
            "migrated the database from schema {from} to {}",
            pagelamp_core::store::SCHEMA_VERSION
        ),
        Ok(Ok(None)) => {}
        Ok(Err(pagelamp_core::Error::SchemaTooNew { found, supported })) => tracing::warn!(
            target: LOG_TARGET,
            "the database is from a newer version (schema {found}; this one reads {supported})"
        ),
        Ok(Err(err)) => tracing::warn!(
            target: LOG_TARGET,
            "could not update the database to schema {}; tools will ask to open the app: {err}",
            pagelamp_core::store::SCHEMA_VERSION
        ),
        Err(_) => tracing::warn!(target: LOG_TARGET, "could not migrate the database"),
    }
}

/// A fixed description of a failed handshake: at most the method name of the unexpected
/// first message, never its parameters.
fn handshake_failure(err: &rmcp::service::ServerInitializeError) -> String {
    use rmcp::service::ServerInitializeError as E;
    match err {
        E::ExpectedInitializeRequest(Some(message)) => serde_json::to_value(message)
            .ok()
            .and_then(|value| Some(clip(value.get("method")?.as_str()?)))
            .map_or_else(
                || "the first message was not an initialize request".to_string(),
                |method| format!("the first message was \"{method}\", not initialize"),
            ),
        E::ExpectedInitializeRequest(None) => "the client closed the connection first".into(),
        E::ConnectionClosed(_) => "the connection closed".into(),
        E::UnexpectedInitializeResponse(_) => "unexpected initialize response".into(),
        E::InitializeFailed(_) => "initialize failed".into(),
        E::TransportError { .. } => "transport error".into(),
        E::Cancelled => "cancelled".into(),
        _ => "unknown error".into(),
    }
}

/// Log target of this crate (`-v` / `PAGELAMP_LOG=debug` raise `pagelamp*` targets).
const LOG_TARGET: &str = "pagelamp::mcp";

/// Client-supplied text for the log: redacted first (so clipping can't cut a secret out of
/// the filter's reach), then control characters (line breaks, tabs) and Unicode line
/// separators become spaces, bidirectional overrides are dropped (they could disguise the
/// line), and at most 64 characters are kept.
fn clip(text: &str) -> String {
    let bidi = |c: char| matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}');
    redact(text)
        .chars()
        .filter(|&c| !bidi(c))
        .map(|c| {
            if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') {
                ' '
            } else {
                c
            }
        })
        .take(64)
        .collect()
}

/// The MCP server. Cheap to clone; every request opens its own short-lived read-only
/// connection inside `spawn_blocking` (docs/ARCHITECTURE.md §4).
#[derive(Clone)]
pub struct PageLampServer {
    db_path: Arc<PathBuf>,
    tool_router: ToolRouter<Self>,
    prompt_router: PromptRouter<Self>,
    /// Tool calls served (shared by clones), for the exit log line.
    tool_calls: Arc<AtomicU64>,
}

// ----- limits ---------------------------------------------------------------------------------

const DEFAULT_SEARCH_LIMIT: u32 = 8;
const MAX_SEARCH_LIMIT: u32 = 25;
const MIN_READ_CHARS: u32 = 500;
const MAX_DAYS: u32 = 365;
const MAX_ANNOUNCEMENT_CHARS: usize = 4_000;
const MAX_LISTED_MATERIALS: usize = 60;
const MAX_LISTED_DEADLINES: usize = 60;
const MAX_PLAN_DAYS: u32 = 120;

// ----- tool parameters ------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CourseArgs {
    #[schemars(description = text::PARAM_COURSE)]
    pub course: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WeekArgs {
    #[schemars(description = text::PARAM_COURSE)]
    pub course: String,
    #[schemars(description = text::PARAM_WEEK)]
    pub week: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadArgs {
    #[schemars(description = text::PARAM_MATERIAL_ID)]
    pub material_id: String,
    #[schemars(description = text::PARAM_FROM_CHUNK)]
    pub from_chunk: Option<u32>,
    #[schemars(description = text::PARAM_MAX_CHARS)]
    pub max_chars: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchArgs {
    #[schemars(description = text::PARAM_QUERY)]
    pub query: String,
    #[schemars(description = text::PARAM_COURSE_OPTIONAL)]
    pub course: Option<String>,
    #[schemars(description = text::PARAM_LIMIT)]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeadlineArgs {
    #[schemars(description = text::PARAM_COURSE_OPTIONAL)]
    pub course: Option<String>,
    #[schemars(description = text::PARAM_DAYS_AHEAD)]
    pub days_ahead: Option<u32>,
    #[schemars(description = text::PARAM_DAYS_BACK)]
    pub days_back: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AnnouncementArgs {
    #[schemars(description = text::PARAM_COURSE)]
    pub course: String,
    #[schemars(description = text::PARAM_DAYS)]
    pub days: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SavePlanArgs {
    #[schemars(description = text::PARAM_PLAN)]
    pub plan: StudyPlan,
}

// ----- prompt arguments (MCP prompt arguments are always strings) -----------------------------

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WeeklyReviewArgs {
    #[schemars(description = text::ARG_COURSE)]
    pub course: String,
    #[schemars(description = text::ARG_WEEK)]
    pub week: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CatchUpArgs {
    #[schemars(description = text::ARG_COURSE)]
    pub course: String,
    #[schemars(description = text::ARG_SINCE)]
    pub since: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StudyPlanArgs {
    #[schemars(description = text::ARG_DAYS)]
    pub days: Option<String>,
    #[schemars(description = text::ARG_HOURS)]
    pub hours_per_week: Option<String>,
}

// ----- tools ----------------------------------------------------------------------------------

#[tool_router]
impl PageLampServer {
    #[tool(description = text::LIST_COURSES, annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false))]
    async fn list_courses(&self) -> CallToolResult {
        let result = self
            .read(|store| {
                let courses = views::list_courses(store, false, AsOf::now_local())?;
                let status = views::sync_status(store, AsOf::now_local())?;
                Ok((courses, sync_hint(&status), status.sources.is_empty()))
            })
            .await;
        match result {
            // Nothing to sync yet: "press Sync" would not help.
            Ok((_, _, true)) => error_result(text::not_initialised()),
            Ok((courses, hint, false)) => json_result(&CourseList {
                courses: courses.iter().map(CourseLine::from).collect(),
                hint,
            }),
            Err(error) => error,
        }
    }

    #[tool(description = text::COURSE_OVERVIEW, annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false))]
    async fn course_overview(&self, Parameters(args): Parameters<CourseArgs>) -> CallToolResult {
        let result = self
            .read(move |store| {
                views::course_overview(store, &args.course, false, AsOf::now_local())
            })
            .await;
        match result {
            Ok(overview) => {
                let (recent_materials, _) =
                    cap_list(overview.recent_materials, MAX_LISTED_MATERIALS);
                json_result(&Overview {
                    guidance: text::guidance(),
                    data_as_of: text::data_as_of(text::DataAsOf {
                        materials: overview.last_synced_at,
                        deadlines: overview.deadlines_synced_at,
                        structure_pending: overview.structure_pending,
                    }),
                    structure_pending: overview.structure_pending.then(text::structure_pending),
                    course: CourseInfo::new(&overview.course, &overview.timeline),
                    ai_materials: overview.ai_materials,
                    note: withheld_note(overview.ai_materials),
                    lifecycle: overview.lifecycle.state,
                    timeline: TimelineInfo::from(overview.timeline),
                    current_modules: overview
                        .current_modules
                        .iter()
                        .map(|m| ModuleInfo {
                            id: m.id.clone(),
                            name: m.name.clone(),
                            week: m.week_hint,
                        })
                        .collect(),
                    recent_materials: recent_materials.iter().map(MaterialInfo::from).collect(),
                    upcoming_deadlines: overview
                        .upcoming_deadlines
                        .iter()
                        .map(DeadlineInfo::from)
                        .collect(),
                    recent_announcements: overview
                        .recent_announcements
                        .iter()
                        .map(|a| AnnouncementInfo {
                            id: a.id.clone(),
                            title: a.title.clone(),
                            posted_at: a.published_at,
                        })
                        .collect(),
                    source: overview.source_label,
                    last_synced_at: overview.last_synced_at,
                })
            }
            Err(error) => error,
        }
    }

    #[tool(description = text::WEEK_MATERIALS, annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false))]
    async fn week_materials(&self, Parameters(args): Parameters<WeekArgs>) -> CallToolResult {
        let result = self
            .read(move |store| {
                let week = views::week_materials(
                    store,
                    &args.course,
                    args.week,
                    false,
                    AsOf::now_local(),
                )?;
                let synced = data_as_of(store, &week.course)?;
                Ok((week, synced))
            })
            .await;
        match result {
            Ok((week, synced)) => {
                let (materials, omitted) = cap_list(week.materials, MAX_LISTED_MATERIALS);
                json_result(&Week {
                    guidance: text::guidance(),
                    data_as_of: text::data_as_of(synced),
                    structure_pending: synced.structure_pending.then(text::structure_pending),
                    course: CourseInfo::new(&week.course, &week.timeline),
                    ai_materials: week.ai_materials,
                    phase: week.timeline.phase,
                    week: week.week,
                    requested_week: week.requested_week,
                    available_weeks: week.available_weeks,
                    // (A waiting course's week isn't empty, it is unread: the note about an
                    // empty week would contradict `structure_pending`.)
                    note: week.note.filter(|_| !synced.structure_pending),
                    modules: week
                        .modules
                        .iter()
                        .map(|m| ModuleInfo {
                            id: m.id.clone(),
                            name: m.name.clone(),
                            week: m.week_hint,
                        })
                        .collect(),
                    materials: materials.iter().map(MaterialInfo::from).collect(),
                    more: (omitted > 0).then(|| text::output_capped(omitted)),
                })
            }
            Err(error) => error,
        }
    }

    #[tool(description = text::READ_MATERIAL, annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false))]
    async fn read_material(&self, Parameters(args): Parameters<ReadArgs>) -> CallToolResult {
        let max_chars = args
            .max_chars
            .unwrap_or(OUTPUT_CAP as u32)
            .clamp(MIN_READ_CHARS, OUTPUT_CAP as u32) as usize;
        let from = args.from_chunk.unwrap_or(0);
        let result = self
            .read(move |store| {
                let material = views::read_material(store, &args.material_id, from, max_chars)?;
                let course = store.resolve_course_with(&material.material.course_id, true)?;
                Ok((material, data_as_of(store, &course)?))
            })
            .await;
        let (material, synced) = match result {
            Ok(read) => read,
            Err(error) => return error,
        };
        let view = &material.material;
        let mut out = vec![text::guidance(), text::data_as_of(synced)];
        out.push(format!(
            "Material: \"{}\" ({}), course {}, {} part(s){}",
            format::escape_attr(&view.title),
            kind_name(view.kind),
            material.course_code.as_deref().unwrap_or(&view.course_id),
            material.total_chunks,
            public_url(view.url.as_deref())
                .map(|u| format!(", link: {u}"))
                .unwrap_or_default()
        ));
        if !material.ai_materials.is_readable() {
            out.push(text::withheld(
                material.ai_materials == AiMaterialsState::TurnedOff,
            ));
            return text_result(out.join("\n"));
        }
        if material.total_chunks == 0 {
            out.push(text::NO_TEXT.to_string());
            return text_result(out.join("\n"));
        }
        for chunk in &material.chunks {
            out.push(wrap(
                &[
                    ("id", Some(&view.id)),
                    ("title", Some(&view.title)),
                    ("course", material.course_code.as_deref()),
                    ("locator", chunk.locator.as_deref()),
                    ("part", Some(&chunk.ord.to_string())),
                ],
                &chunk.text,
            ));
        }
        out.push(match material.next_chunk {
            Some(next) => text::read_more(next),
            None => text::END_OF_MATERIAL.to_string(),
        });
        text_result(out.join("\n"))
    }

    #[tool(description = text::SEARCH_MATERIALS, annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false))]
    async fn search_materials(&self, Parameters(args): Parameters<SearchArgs>) -> CallToolResult {
        let limit = args
            .limit
            .unwrap_or(DEFAULT_SEARCH_LIMIT)
            .clamp(1, MAX_SEARCH_LIMIT);
        let query = args.query.clone();
        let result = self
            .read(move |store| {
                let results =
                    views::search_for_ai(store, &args.query, args.course.as_deref(), limit)?;
                // A course no full sync has read has nothing to find yet.
                let unread = match args.course.as_deref() {
                    Some(course) => auto_sync::light_sync(store)?
                        .structure_pending
                        .contains(&store.resolve_course(course)?.id),
                    None => false,
                };
                Ok((results, unread))
            })
            .await;
        let (results, unread) = match result {
            Ok(results) => results,
            Err(error) => return error,
        };
        if let Some(state) = results.course_ai_materials
            && !state.is_readable()
        {
            return text_result(text::withheld(state == AiMaterialsState::TurnedOff));
        }
        let mut out = vec![format!("Search results for \"{}\":", query.trim())];
        let mut used = 0;
        let mut shown = 0;
        for hit in &results.hits {
            let block = wrap(
                &[
                    ("id", Some(&hit.material_id)),
                    ("title", Some(&hit.material_title)),
                    ("course", hit.course_code.as_deref()),
                    ("locator", hit.locator.as_deref()),
                    ("part", Some(&hit.chunk_ord.to_string())),
                    ("url", public_url(hit.url.as_deref())),
                ],
                &hit.snippet,
            );
            if used + block.len() > OUTPUT_CAP && shown > 0 {
                break;
            }
            used += block.len();
            shown += 1;
            out.push(block);
        }
        if results.hits.is_empty() && unread {
            out.push(text::structure_pending());
        } else if results.hits.is_empty() {
            out.push(text::NO_HITS.to_string());
        } else if shown < results.hits.len() {
            out.push(text::output_capped(results.hits.len() - shown));
        }
        if !results.excluded_courses.is_empty() {
            out.push(text::excluded_courses(&results.excluded_courses.join(", ")));
        }
        text_result(out.join("\n"))
    }

    #[tool(description = text::LIST_DEADLINES, annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false))]
    async fn list_deadlines(&self, Parameters(args): Parameters<DeadlineArgs>) -> CallToolResult {
        let ahead = args
            .days_ahead
            .unwrap_or(views::UPCOMING_DAYS)
            .min(MAX_DAYS);
        let back = args.days_back.unwrap_or(0).min(MAX_DAYS);
        let result = self
            .read(move |store| {
                views::deadlines(
                    store,
                    args.course.as_deref(),
                    ahead,
                    back,
                    false,
                    AsOf::now_local(),
                )
            })
            .await;
        match result {
            Ok(deadlines) => {
                let (deadlines, omitted) = cap_list(deadlines, MAX_LISTED_DEADLINES);
                json_result(&DeadlineList {
                    deadlines: deadlines.iter().map(DeadlineInfo::from).collect(),
                    more: (omitted > 0).then(|| text::output_capped(omitted)),
                })
            }
            Err(error) => error,
        }
    }

    #[tool(description = text::GET_ANNOUNCEMENTS, annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false))]
    async fn get_announcements(
        &self,
        Parameters(args): Parameters<AnnouncementArgs>,
    ) -> CallToolResult {
        let days = args.days.unwrap_or(views::RECENT_DAYS).clamp(1, MAX_DAYS);
        let result = self
            .read(move |store| {
                views::announcements(
                    store,
                    &args.course,
                    days,
                    MAX_ANNOUNCEMENT_CHARS,
                    AsOf::now_local(),
                )
            })
            .await;
        let items = match result {
            Ok(items) => items,
            Err(error) => return error,
        };
        if items.is_empty() {
            return text_result(text::NO_ANNOUNCEMENTS);
        }
        let mut out = Vec::new();
        if let Some(first) = items.first()
            && !first.ai_materials.is_readable()
        {
            out.push(text::withheld(
                first.ai_materials == AiMaterialsState::TurnedOff,
            ));
            // Structure stays available: titles and dates only.
            for item in &items {
                out.push(format!(
                    "- {} ({})",
                    item.material.title,
                    item.material
                        .published_at
                        .map(|p| p.to_rfc3339())
                        .unwrap_or_default()
                ));
            }
            return text_result(out.join("\n"));
        }
        let mut used = 0;
        for (shown, item) in items.iter().enumerate() {
            let posted = item.material.published_at.map(|p| p.to_rfc3339());
            let block = wrap(
                &[
                    ("id", Some(&item.material.id)),
                    ("title", Some(&item.material.title)),
                    ("kind", Some("announcement")),
                    ("posted", posted.as_deref()),
                    ("url", public_url(item.material.url.as_deref())),
                ],
                &item.text,
            );
            if used + block.len() > OUTPUT_CAP && shown > 0 {
                out.push(text::output_capped(items.len() - shown));
                break;
            }
            used += block.len();
            out.push(block);
        }
        text_result(out.join("\n"))
    }

    #[tool(description = text::GET_STUDY_PLAN, annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false))]
    async fn get_study_plan(&self) -> CallToolResult {
        match self.read(|store| store.latest_study_plan()).await {
            Ok(Some(plan)) => match serde_json::to_string(&plan) {
                Ok(json) => text_result(wrap_plan(text::PLAN_PREFACE, &json)),
                Err(err) => error_result(format!("internal error: {err}")),
            },
            Ok(None) => text_result(text::NO_PLAN),
            Err(error) => error,
        }
    }

    #[tool(description = text::SAVE_STUDY_PLAN, annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = false, open_world_hint = false))]
    async fn save_study_plan(&self, Parameters(args): Parameters<SavePlanArgs>) -> CallToolResult {
        let db = Arc::clone(&self.db_path);
        let saved = tokio::task::spawn_blocking(move || {
            // The one MCP write: a short read-write transaction (busy_timeout applies).
            if !db.is_file() {
                return Err(pagelamp_core::Error::NotInitialised(
                    db.display().to_string(),
                ));
            }
            Store::open(&db)?.save_study_plan(&args.plan)
        })
        .await;
        match saved {
            Ok(Ok(stored)) => json_result(&SavedPlan {
                saved: true,
                id: stored.id,
                created_at: stored.created_at,
                items: stored.plan.items.len(),
            }),
            Ok(Err(err)) => core_error(err),
            Err(join) => error_result(format!("internal error: {join}")),
        }
    }

    #[tool(description = text::sync_status_description(), annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false))]
    async fn sync_status(&self) -> CallToolResult {
        match self
            .read(|store| views::sync_status(store, AsOf::now_local()))
            .await
        {
            Ok(status) if status.sources.is_empty() => error_result(text::not_initialised()),
            Ok(status) => json_result(&SyncInfo {
                hint: sync_hint(&status),
                sources: status
                    .sources
                    .iter()
                    .map(|s| SourceInfo {
                        id: s.source.id.clone(),
                        label: s.source.label.clone(),
                        kind: s.source.kind,
                        last_synced_at: s.source.last_synced_at,
                        // Error texts can quote paths or server answers: never pass on the
                        // home folder (user name) or anything secret-shaped.
                        last_error: s.source.last_error.as_deref().map(redact),
                        last_error_kind: s.source.last_error_kind,
                        stale: s.stale,
                        state: freshness(s),
                        deadlines_synced_at: later(s.deadlines_synced_at, s.source.last_synced_at),
                    })
                    .collect(),
                counts: status.counts,
                last_synced_at: status.last_synced_at,
                stale: status.stale,
                auto_sync: status.auto_sync,
                last_automatic_sync_at: status.last_automatic_sync_at,
            }),
            Err(error) => error,
        }
    }
}

// ----- prompts --------------------------------------------------------------------------------

#[prompt_router]
impl PageLampServer {
    #[prompt(name = "weekly_review", description = text::PROMPT_WEEKLY_REVIEW)]
    async fn weekly_review(
        &self,
        Parameters(args): Parameters<WeeklyReviewArgs>,
    ) -> Result<Vec<PromptMessage>, ErrorData> {
        let week = parse_number(args.week.as_deref(), "week")?;
        let course = self.course_state(&args.course).await?;
        let mut message = text::weekly_review(&course.label, &course.reference, week);
        append_withheld(&mut message, &course.label, course.state);
        Ok(vec![PromptMessage::new_text(Role::User, message)])
    }

    #[prompt(name = "catch_up", description = text::PROMPT_CATCH_UP)]
    async fn catch_up(
        &self,
        Parameters(args): Parameters<CatchUpArgs>,
    ) -> Result<Vec<PromptMessage>, ErrorData> {
        let since = match args
            .since
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(text) => NaiveDate::parse_from_str(text, "%Y-%m-%d").map_err(|_| {
                ErrorData::invalid_params("since must be a date like 2026-09-01", None)
            })?,
            None => Local::now().date_naive() - TimeDelta::days(i64::from(views::RECENT_DAYS)),
        };
        let course = self.course_state(&args.course).await?;
        let since = since.format("%Y-%m-%d").to_string();
        let mut message = text::catch_up(&course.label, &course.reference, &since);
        append_withheld(&mut message, &course.label, course.state);
        Ok(vec![PromptMessage::new_text(Role::User, message)])
    }

    #[prompt(name = "study_plan", description = text::PROMPT_STUDY_PLAN)]
    async fn study_plan(
        &self,
        Parameters(args): Parameters<StudyPlanArgs>,
    ) -> Result<Vec<PromptMessage>, ErrorData> {
        let days = parse_number(args.days.as_deref(), "days")?
            .unwrap_or(14)
            .clamp(1, MAX_PLAN_DAYS);
        let hours = parse_number(args.hours_per_week.as_deref(), "hours_per_week")?;
        Ok(vec![PromptMessage::new_text(
            Role::User,
            text::study_plan(days, hours),
        )])
    }
}

#[tool_handler(router = self.tool_router)]
#[prompt_handler(router = self.prompt_router)]
impl ServerHandler for PageLampServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_prompts()
                .build(),
        )
        .with_server_info(
            Implementation::new(brand::MCP_SERVER_KEY, env!("CARGO_PKG_VERSION"))
                .with_title(brand::PRODUCT_NAME)
                .with_description(brand::TAGLINE)
                .with_website_url(brand::HOMEPAGE),
        )
        .with_instructions(text::instructions())
    }

    /// The router's dispatch plus a debug log line: tool name, time, ok/error and output size.
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        self.tool_calls.fetch_add(1, Ordering::Relaxed);
        let started = Instant::now();
        let name = clip(&request.name);
        let response = self
            .tool_router
            .call(ToolCallContext::new(self, request, context))
            .await;
        if tracing::enabled!(target: LOG_TARGET, tracing::Level::DEBUG) {
            let outcome = match &response {
                Ok(CallToolResponse::Complete(result)) if result.is_error == Some(true) => "error",
                Ok(_) => "ok",
                Err(_) => "protocol error",
            };
            let bytes = match &response {
                Ok(CallToolResponse::Complete(result)) => {
                    serde_json::to_vec(result).map_or(0, |json| json.len())
                }
                _ => 0,
            };
            tracing::debug!(
                target: LOG_TARGET,
                "tool {name:?}: {outcome}, {} ms, {bytes} bytes",
                started.elapsed().as_millis()
            );
        }
        response
    }
}

impl PageLampServer {
    /// A server over the database at `db_path` (which may not exist yet).
    pub fn new(db_path: PathBuf) -> Self {
        PageLampServer {
            db_path: Arc::new(db_path),
            tool_router: Self::tool_router(),
            prompt_router: Self::prompt_router(),
            tool_calls: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Run `f` on a fresh read-only connection on the blocking pool. Errors become a
    /// ready-to-return tool error result.
    async fn read<T: Send + 'static>(
        &self,
        f: impl FnOnce(&Store) -> pagelamp_core::Result<T> + Send + 'static,
    ) -> Result<T, CallToolResult> {
        let db = Arc::clone(&self.db_path);
        match tokio::task::spawn_blocking(move || f(&Store::open_read_only(&db)?)).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(err)) => Err(core_error(err)),
            Err(join) => Err(error_result(format!("internal error: {join}"))),
        }
    }

    /// Display label and AI-materials state of a course, for prompts. Before the first sync
    /// the prompt still works (the tools will explain the missing data).
    async fn course_state(&self, course: &str) -> Result<PromptCourse, ErrorData> {
        let db = Arc::clone(&self.db_path);
        let query = course.to_string();
        let found = tokio::task::spawn_blocking(move || {
            let store = Store::open_read_only(&db)?;
            let course = store.resolve_course(&query)?;
            // What the prompt tells the AI to pass to the tools: the code when it names only
            // this course, else the id (both resolve; the display name may not).
            let reference = match &course.code {
                Some(code) if store.resolve_course(code).is_ok_and(|c| c.id == course.id) => {
                    code.clone()
                }
                _ => course.id.clone(),
            };
            Ok::<_, pagelamp_core::Error>(PromptCourse {
                label: course.display_name(),
                reference,
                state: course.ai_materials(),
            })
        })
        .await
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
        match found {
            Ok(course) => Ok(course),
            Err(
                pagelamp_core::Error::NotInitialised(_) | pagelamp_core::Error::SchemaTooOld { .. },
            ) => Ok(PromptCourse {
                label: course.to_string(),
                reference: course.to_string(),
                state: AiMaterialsState::Readable,
            }),
            // Fixed texts: rmcp logs error responses, and neither the query nor the course
            // list belongs in a log.
            Err(pagelamp_core::Error::NotFound(_)) => Err(ErrorData::invalid_params(
                "No course matches that name. Call list_courses for the course codes.",
                None,
            )),
            Err(pagelamp_core::Error::Ambiguous { .. }) => Err(ErrorData::invalid_params(
                "That name matches several courses. Use a course code from list_courses.",
                None,
            )),
            Err(_) => Err(ErrorData::internal_error(
                "Could not read the course list.",
                None,
            )),
        }
    }
}

// ----- helpers --------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::clip;

    #[test]
    fn client_text_is_redacted_flattened_and_clipped() {
        assert_eq!(
            clip("demo\u{202E}\tBearer\txyzSECRET\u{2028}next\nline"),
            "demo Bearer <redacted> next line"
        );
        assert_eq!(clip(&"x".repeat(100)).chars().count(), 64);
    }
}

/// A course as a prompt names it.
struct PromptCourse {
    /// For the prose ("DEMO101 — Intro to Demo Studies").
    label: String,
    /// For tool arguments: a code or id that resolves to this course.
    reference: String,
    state: AiMaterialsState,
}

/// A core error as a tool error the model can explain to the student.
fn core_error(err: pagelamp_core::Error) -> CallToolResult {
    use pagelamp_core::Error as E;
    match err {
        E::NotInitialised(_) => error_result(text::not_initialised()),
        E::SchemaTooOld { .. } => error_result(text::needs_database_update()),
        E::NotFound(what) => error_result(format!("Not found: {what}")),
        E::Invalid(what) => error_result(format!("Invalid input: {what}")),
        err @ E::Ambiguous { .. } => error_result(err.to_string()),
        err @ E::SchemaTooNew { .. } => error_result(err.to_string()),
        other => {
            tracing::error!(target: LOG_TARGET, "tool failed: {other}");
            error_result(format!("Internal error: {other}"))
        }
    }
}

fn append_withheld(message: &mut String, label: &str, state: AiMaterialsState) {
    if !state.is_readable() {
        message.push_str("\n\n");
        message.push_str(&text::prompt_withheld(
            label,
            state == AiMaterialsState::TurnedOff,
        ));
    }
}

/// Parse an optional numeric prompt argument ("" = absent).
fn parse_number(value: Option<&str>, name: &str) -> Result<Option<u32>, ErrorData> {
    match value.map(str::trim).filter(|v| !v.is_empty()) {
        None => Ok(None),
        Some(text) => text
            .parse()
            .map(Some)
            .map_err(|_| ErrorData::invalid_params(format!("{name} must be a whole number"), None)),
    }
}

/// Withheld explanation for overview-style results.
fn withheld_note(state: AiMaterialsState) -> Option<String> {
    (!state.is_readable()).then(|| text::withheld(state == AiMaterialsState::TurnedOff))
}

/// URLs worth giving the AI app: LMS links yes, `file://` paths no (they reveal local paths,
/// e.g. the student's user name, and the AI app can't open them anyway).
fn public_url(url: Option<&str>) -> Option<&str> {
    url.filter(|u| !u.trim_start().to_ascii_lowercase().starts_with("file:"))
}

fn kind_name(kind: MaterialKind) -> &'static str {
    kind.as_str()
}

// ----- compact output shapes ------------------------------------------------------------------
// Tool results use these instead of the full view types: fewer tokens, and nothing private
// (source configs, local paths) reaches the AI provider.

#[derive(Serialize)]
struct CourseList {
    courses: Vec<CourseLine>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<String>,
}

#[derive(Serialize)]
struct CourseLine {
    id: String,
    code: Option<String>,
    name: String,
    /// Null for a course whose lifecycle is ended, inactive or upcoming (an upcoming course
    /// keeps the week the student's own term dates give).
    current_week: Option<u32>,
    week_confidence: Confidence,
    phase: CoursePhase,
    /// Ended courses stay listed (a finished course is useful for reviewing a prerequisite).
    lifecycle: LifecycleState,
    outside_term: bool,
    ai_policy: AiPolicy,
    #[serde(skip_serializing_if = "Option::is_none")]
    ai_policy_note: Option<String>,
    ai_materials: AiMaterialsState,
    materials: u32,
    readable_materials: u32,
    next_deadline: Option<DeadlineInfo>,
    source: String,
    /// The last full sync of the course's source (modules, materials and their text).
    last_synced_at: Option<Timestamp>,
    /// When its deadlines and announcements were last read, when that is later.
    #[serde(skip_serializing_if = "Option::is_none")]
    deadlines_synced_at: Option<Timestamp>,
    /// Present for a course whose modules and materials haven't been read yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    structure_pending: Option<String>,
}

/// `deadlines` when it says more than `materials` (it is later).
fn later(deadlines: Option<Timestamp>, materials: Option<Timestamp>) -> Option<Timestamp> {
    deadlines.filter(|at| materials.is_none_or(|full| *at > full))
}

impl From<&views::CourseSummary> for CourseLine {
    fn from(summary: &views::CourseSummary) -> Self {
        CourseLine {
            id: summary.course.id.clone(),
            code: summary.course.code.clone(),
            name: summary.course.name.clone(),
            current_week: summary.timeline.current_week,
            week_confidence: summary.timeline.confidence,
            phase: summary.timeline.phase,
            lifecycle: summary.lifecycle.state,
            outside_term: summary.timeline.outside_term,
            ai_policy: summary.course.ai_policy,
            ai_policy_note: summary.course.ai_policy_note.clone(),
            ai_materials: summary.ai_materials,
            materials: summary.counts.materials,
            readable_materials: summary.counts.indexed_materials,
            next_deadline: summary.next_deadline.as_ref().map(DeadlineInfo::from),
            source: summary.source_label.clone(),
            last_synced_at: summary.last_synced_at,
            deadlines_synced_at: later(summary.deadlines_synced_at, summary.last_synced_at),
            structure_pending: summary.structure_pending.then(text::structure_pending),
        }
    }
}

#[derive(Serialize)]
struct CourseInfo {
    id: String,
    code: Option<String>,
    name: String,
    /// The dates that count the course's weeks: the first class and the end of exams (or the
    /// last class), or null. An LMS term that isn't used (e.g. an enrollment window) never
    /// shows here (calendar design §7.13, CAL-18).
    term_start: Option<NaiveDate>,
    term_end: Option<NaiveDate>,
    /// Where those dates come from (`none` when no dates are known).
    term_dates_source: TermAnchorSource,
    url: Option<String>,
    ai_policy: AiPolicy,
    #[serde(skip_serializing_if = "Option::is_none")]
    ai_policy_note: Option<String>,
}

impl CourseInfo {
    fn new(course: &pagelamp_core::model::Course, timeline: &CourseTimeline) -> Self {
        let term = &timeline.term;
        let first = term.teaching.first();
        let last = term.teaching.last();
        CourseInfo {
            id: course.id.clone(),
            code: course.code.clone(),
            name: course.name.clone(),
            term_start: first.map(|segment| segment.first_class),
            term_end: term
                .exams_end
                .or(last.and_then(|segment| segment.last_class)),
            term_dates_source: term.anchor,
            url: public_url(course.url.as_deref()).map(str::to_string),
            ai_policy: course.ai_policy,
            ai_policy_note: course.ai_policy_note.clone(),
        }
    }
}

/// Where the course is. Structure only: dates, enums and numbers, never break labels, week
/// topics or quotes (calendar design §7.11, rule 8).
#[derive(Serialize)]
struct TimelineInfo {
    current_week: Option<u32>,
    confidence: Confidence,
    outside_term: bool,
    phase: CoursePhase,
    /// The week to use by default (e.g. the week before a break); null in the exam period.
    #[serde(skip_serializing_if = "Option::is_none")]
    default_week: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    break_after_week: Option<u32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    teaching: Vec<SegmentInfo>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    breaks: Vec<BreakInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exams_end: Option<NaiveDate>,
    anchor: TermAnchorSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    anchor_origin: Option<CalendarOrigin>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ai_label: Option<AiLabel>,
    calendar_status: CalendarStatus,
    /// Reasons for the week (may quote module/material titles — course data).
    evidence: Vec<String>,
}

impl From<CourseTimeline> for TimelineInfo {
    fn from(timeline: CourseTimeline) -> Self {
        let term = timeline.term;
        TimelineInfo {
            current_week: timeline.current_week,
            confidence: timeline.confidence,
            outside_term: timeline.outside_term,
            phase: timeline.phase,
            default_week: timeline.default_week,
            break_after_week: timeline.break_after_week,
            teaching: term
                .teaching
                .iter()
                .map(|segment| SegmentInfo {
                    first_class: segment.first_class,
                    last_class: segment.last_class,
                    first_week: segment.first_week_number,
                })
                .collect(),
            breaks: term
                .breaks
                .iter()
                .map(|b| BreakInfo {
                    kind: b.kind,
                    start: b.span.start,
                    end: b.span.end,
                    numbered: b.numbered,
                })
                .collect(),
            exams_end: term.exams_end,
            anchor: term.anchor,
            anchor_origin: term.anchor_origin,
            ai_label: term.ai_label,
            calendar_status: timeline.calendar,
            evidence: timeline.evidence,
        }
    }
}

#[derive(Serialize)]
struct SegmentInfo {
    first_class: NaiveDate,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_class: Option<NaiveDate>,
    first_week: u32,
}

/// A break as structure: its kind and dates; never its label (material text).
#[derive(Serialize)]
struct BreakInfo {
    kind: BreakKind,
    start: NaiveDate,
    end: NaiveDate,
    numbered: bool,
}

#[derive(Serialize)]
struct ModuleInfo {
    id: String,
    name: String,
    week: Option<u32>,
}

#[derive(Serialize)]
struct MaterialInfo {
    id: String,
    title: String,
    kind: MaterialKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    week: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    module: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    published_at: Option<Timestamp>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    /// Parts of text available to read_material (0 = no text).
    parts: u32,
    text: TextStatus,
}

impl From<&MaterialView> for MaterialInfo {
    fn from(view: &MaterialView) -> Self {
        MaterialInfo {
            id: view.id.clone(),
            title: view.title.clone(),
            kind: view.kind,
            week: view.week_hint,
            module: view.module_name.clone(),
            published_at: view.published_at,
            url: public_url(view.url.as_deref()).map(str::to_string),
            parts: view.chunk_count,
            text: view.text_status,
        }
    }
}

#[derive(Serialize)]
struct DeadlineInfo {
    title: String,
    kind: EventKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    due_at: Option<Timestamp>,
    #[serde(skip_serializing_if = "Option::is_none")]
    starts_at: Option<Timestamp>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ends_at: Option<Timestamp>,
    #[serde(skip_serializing_if = "Option::is_none")]
    course: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

impl From<&Deadline> for DeadlineInfo {
    fn from(deadline: &Deadline) -> Self {
        let event = &deadline.event;
        DeadlineInfo {
            title: event.title.clone(),
            kind: event.kind,
            due_at: event.due_at,
            starts_at: event.starts_at,
            ends_at: event.ends_at,
            course: deadline
                .course_code
                .clone()
                .or_else(|| deadline.course_name.clone()),
            url: event.url.clone(),
        }
    }
}

#[derive(Serialize)]
struct AnnouncementInfo {
    id: String,
    title: String,
    posted_at: Option<Timestamp>,
}

#[derive(Serialize)]
struct Overview {
    guidance: String,
    data_as_of: String,
    /// Present for a course whose modules and materials haven't been read yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    structure_pending: Option<String>,
    course: CourseInfo,
    ai_materials: AiMaterialsState,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
    lifecycle: LifecycleState,
    timeline: TimelineInfo,
    current_modules: Vec<ModuleInfo>,
    recent_materials: Vec<MaterialInfo>,
    upcoming_deadlines: Vec<DeadlineInfo>,
    recent_announcements: Vec<AnnouncementInfo>,
    source: String,
    last_synced_at: Option<Timestamp>,
}

#[derive(Serialize)]
struct Week {
    guidance: String,
    data_as_of: String,
    /// Present for a course whose modules and materials haven't been read yet: an empty week
    /// then means "not read", not "nothing there".
    #[serde(skip_serializing_if = "Option::is_none")]
    structure_pending: Option<String>,
    course: CourseInfo,
    ai_materials: AiMaterialsState,
    phase: CoursePhase,
    week: Option<u32>,
    requested_week: Option<u32>,
    available_weeks: Vec<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
    modules: Vec<ModuleInfo>,
    materials: Vec<MaterialInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    more: Option<String>,
}

#[derive(Serialize)]
struct DeadlineList {
    deadlines: Vec<DeadlineInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    more: Option<String>,
}

#[derive(Serialize)]
struct SavedPlan {
    saved: bool,
    id: i64,
    created_at: Timestamp,
    items: usize,
}

#[derive(Serialize)]
struct SyncInfo {
    sources: Vec<SourceInfo>,
    counts: StoreCounts,
    last_synced_at: Option<Timestamp>,
    stale: bool,
    /// How often PageLamp syncs by itself while its app is open ("off": only when the student
    /// starts a sync). Read-only here: nothing an MCP client does changes it or starts a sync.
    auto_sync: AutoSync,
    /// When an automatic sync last ended with every source it could sync synced.
    last_automatic_sync_at: Option<Timestamp>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<String>,
}

/// How fresh one source's data is: what only the student can fix first.
fn freshness(status: &views::SourceStatus) -> text::Freshness {
    let source = &status.source;
    if auto_sync::needs_the_student(source) {
        text::Freshness::NeedsStudent
    } else if source.last_synced_at.is_none() {
        text::Freshness::NeverSynced
    } else if source.last_error.is_some() {
        text::Freshness::Failed
    } else if status.stale && !status.deadlines_stale {
        text::Freshness::MaterialsOld
    } else if status.stale {
        text::Freshness::Old
    } else {
        text::Freshness::Fresh
    }
}

/// What to tell the student when some data isn't fresh (the worst source decides).
fn sync_hint(status: &views::SyncStatus) -> Option<String> {
    let worst = status
        .sources
        .iter()
        .map(freshness)
        .min()
        .unwrap_or(text::Freshness::Fresh);
    text::freshness_hint(worst, status.auto_sync != AutoSync::Off)
}

/// What the "data as of" line of the read tools says about `course`: the two clocks of its
/// source, and whether the course still waits for its first full sync.
fn data_as_of(store: &Store, course: &Course) -> pagelamp_core::Result<text::DataAsOf> {
    let light = auto_sync::light_sync(store)?;
    let structure_pending = light.structure_pending.contains(&course.id);
    let Some(source) = store.get_source(&course.source_id)? else {
        return Ok(text::DataAsOf {
            structure_pending,
            ..text::DataAsOf::default()
        });
    };
    Ok(text::DataAsOf {
        materials: source.last_synced_at,
        deadlines: light.deadlines_synced_at(&source),
        structure_pending,
    })
}

#[derive(Serialize)]
struct SourceInfo {
    id: String,
    label: String,
    kind: SourceKind,
    last_synced_at: Option<Timestamp>,
    last_error: Option<String>,
    last_error_kind: Option<SourceErrorKind>,
    stale: bool,
    /// fresh / old / never_synced / failed / needs_student (only the student can fix it).
    state: text::Freshness,
    /// When deadlines and announcements were last read, when that is later than the last
    /// full sync (`last_synced_at`).
    #[serde(skip_serializing_if = "Option::is_none")]
    deadlines_synced_at: Option<Timestamp>,
}
