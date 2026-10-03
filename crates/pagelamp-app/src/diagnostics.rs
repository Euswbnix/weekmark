//! Diagnostics for beta testers: logging setup, crash records, `doctor` and the shareable
//! report. The free functions resolve the default data dir themselves, so they also work
//! when `App::open()` failed (a locked or damaged database) — exactly when a report is
//! needed. `App` has methods of the same names for an explicit data dir.
//!
//! Nothing here sends anything anywhere: the report is text the student reads and may
//! paste into an issue. It never contains tokens, feed URLs, course material text, course
//! codes or names (pseudonymised as "Course 1", "Course 2", …), or the user name in paths.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use chrono::{Local, NaiveDate, TimeDelta};
use pagelamp_core::brand;
use pagelamp_core::diagnostics as core_diag;
use pagelamp_core::model::{
    Course, MigrationBackupOutcome, MigrationBackupRecord, SourceErrorKind, SourceKind,
    TextErrorKind, Timestamp,
};
use pagelamp_core::paths;
use pagelamp_core::removal::TombstoneState;
use pagelamp_core::secrets::{KeychainSecrets, SecretBackend};
use pagelamp_core::store::Store;
use pagelamp_core::views::AsOf;
use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use pagelamp_core::diagnostics::{
    CrashReport, ExpectPanics, ProcessKind, expect_panics, expect_panics_in,
};

use crate::{AppError, AppErrorKind, Result};

/// Log lines included in a report.
const REPORT_LOG_LINES: usize = 200;
const MAX_UI_MESSAGE_CHARS: usize = 2_000;
const MAX_UI_STACK_CHARS: usize = 8_000;

/// A source as `doctor` shows it: no ids, URLs or labels.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct DoctorSource {
    pub kind: SourceKind,
    /// The last sync succeeded (false when it failed or never ran).
    pub ok: bool,
    pub last_synced_at: Option<Timestamp>,
    pub last_error_kind: Option<SourceErrorKind>,
}

/// Whether each AI app's config has a PageLamp entry (presence only; nothing else is read
/// out of those files).
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct McpClientPresence {
    pub claude_desktop: bool,
    pub claude_code: bool,
    pub codex: bool,
}

/// Whether the extraction worker (`pagelamp extract-worker`, v0.3 M0.5) works.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExtractWorkerStatus {
    /// It started and answered.
    Ok,
    /// This surface set no worker (`App::set_extract_worker`, or `doctor` ran without an
    /// `App`): files are read in the app's own process.
    NotSet,
    /// It could not be started: missing, or blocked by antivirus / Smart App Control. Syncs
    /// then read only small non-PDF files and retry the others on the next sync.
    SpawnFailed,
    /// A worker of another PageLamp version answered (a broken install; reinstalling fixes it).
    ProtocolMismatch,
    /// It started but did not answer properly.
    Failed,
}

/// `doctor`'s check of the extraction worker: it is started once.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExtractWorkerCheck {
    pub status: ExtractWorkerStatus,
    /// How long starting it and getting its answer took (`ok` only).
    pub spawn_ms: Option<u32>,
}

/// How many files the extraction worker could not read for one reason.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UnreadableFiles {
    pub kind: TextErrorKind,
    pub count: u32,
}

/// `pagelamp doctor`: the facts a maintainer needs to help, and nothing personal.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct DoctorReport {
    pub version: String,
    pub os: String,
    pub arch: String,
    /// With the home directory shortened to `~`.
    pub data_dir: String,
    pub logs_dir: String,
    pub schema_version: Option<i64>,
    /// Why the database could not be read (then the counts are 0).
    pub database_error: Option<String>,
    pub keychain_available: bool,
    pub keychain_error: Option<String>,
    pub sources: Vec<DoctorSource>,
    pub courses: u32,
    pub hidden_courses: u32,
    pub materials: u32,
    pub events: u32,
    pub mcp_clients: McpClientPresence,
    pub last_crash: Option<CrashReport>,
    pub extract_worker: ExtractWorkerCheck,
    /// Files the extraction worker could not read, per reason (reasons without files left
    /// out; empty when the database can't be read).
    pub unreadable_files: Vec<UnreadableFiles>,
    /// Models PageLamp calls itself: keys present (never the keys), local servers running.
    #[serde(default)]
    pub ai: crate::ai::AiDoctor,
    /// Courses whose LMS term looks like an enrollment window, so it isn't used to count weeks
    /// (calendar design §6.3). A count, never names.
    #[serde(default)]
    pub enrollment_window_terms: u32,
    /// Removed courses still waiting: for their purge (the undo window) or for their downloaded
    /// files to go to the Trash (calendar design §8.3). A count, never names.
    #[serde(default)]
    pub removals_waiting: u32,
}

fn data_dir() -> Result<PathBuf> {
    Ok(paths::data_dir()?)
}

/// Set up log files, redacted stderr logging and the panic hook for this process. Call once,
/// first thing (before `App::open`). `verbose` = `PAGELAMP_LOG=debug`.
pub fn init(kind: ProcessKind, verbose: bool) {
    let dir = paths::data_dir().ok();
    core_diag::init(dir.as_deref(), kind, verbose);
}

/// `<default data dir>/logs` (created if missing).
pub fn logs_dir() -> Result<PathBuf> {
    logs_dir_in(&data_dir()?)
}

pub fn last_crash() -> Result<Option<CrashReport>> {
    Ok(core_diag::last_crash(&data_dir()?)?)
}

pub fn clear_last_crash() -> Result<()> {
    Ok(core_diag::clear_last_crash(&data_dir()?)?)
}

/// Without an `App` there is no extraction worker to check (`ExtractWorkerStatus::NotSet`).
pub fn doctor() -> Result<DoctorReport> {
    Ok(doctor_in(&data_dir()?, &KeychainSecrets, None))
}

/// Markdown for an issue: doctor + last crash + the last ~200 log lines, redacted and with
/// course codes/names pseudonymised.
pub fn diagnostic_report() -> Result<String> {
    Ok(report_in(&data_dir()?, &KeychainSecrets, None))
}

/// One ERROR line from the desktop UI (message + optional stack, capped and redacted).
pub fn log_ui_error(message: &str, stack: Option<&str>) {
    let message: String = message.chars().take(MAX_UI_MESSAGE_CHARS).collect();
    match stack {
        Some(stack) => {
            let stack: String = stack.chars().take(MAX_UI_STACK_CHARS).collect();
            tracing::error!(target: "pagelamp::ui", "{message}\n{stack}");
        }
        None => tracing::error!(target: "pagelamp::ui", "{message}"),
    }
}

pub(crate) fn logs_dir_in(data_dir: &Path) -> Result<PathBuf> {
    let dir = core_diag::logs_dir_in(data_dir);
    paths::create_private_dir_all(&dir).map_err(|err| {
        AppError::new(
            AppErrorKind::Internal,
            format!("could not create the logs folder: {err}"),
        )
    })?;
    Ok(dir)
}

/// `worker`: the extraction worker to check (`App::extract_worker`).
pub(crate) fn doctor_in(
    data_dir: &Path,
    secrets: &dyn SecretBackend,
    worker: Option<&Path>,
) -> DoctorReport {
    let keychain = secrets.check();
    let mut report = DoctorReport {
        version: env!("CARGO_PKG_VERSION").to_string(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        data_dir: core_diag::shorten_home(&data_dir.display().to_string()),
        logs_dir: core_diag::shorten_home(&core_diag::logs_dir_in(data_dir).display().to_string()),
        schema_version: None,
        database_error: None,
        keychain_available: keychain.is_ok(),
        keychain_error: keychain.err().map(|e| core_diag::redact(&e)),
        sources: Vec::new(),
        courses: 0,
        hidden_courses: 0,
        materials: 0,
        events: 0,
        mcp_clients: mcp_client_presence(),
        last_crash: core_diag::last_crash(data_dir).ok().flatten(),
        extract_worker: check_extract_worker(worker),
        unreadable_files: Vec::new(),
        ai: crate::ai::AiDoctor::default(),
        enrollment_window_terms: 0,
        removals_waiting: 0,
    };
    let db = paths::db_path_in(data_dir);
    let read = Store::open_read_only(&db).and_then(|store| {
        Ok((
            store.schema_version()?,
            store.counts()?,
            store.list_sources()?,
            store.unreadable_file_counts()?,
            course_notes(&store)?,
        ))
    });
    match read {
        Ok((version, counts, sources, unreadable, (enrollment_window_terms, removals_waiting))) => {
            report.enrollment_window_terms = enrollment_window_terms;
            report.removals_waiting = removals_waiting;
            report.unreadable_files = unreadable
                .into_iter()
                .map(|(kind, count)| UnreadableFiles { kind, count })
                .collect();
            report.schema_version = Some(version);
            report.courses = counts.courses;
            report.hidden_courses = counts.hidden_courses;
            report.materials = counts.materials;
            report.events = counts.events;
            report.sources = sources
                .into_iter()
                .map(|s| DoctorSource {
                    kind: s.kind,
                    ok: s.last_synced_at.is_some() && s.last_error.is_none(),
                    last_synced_at: s.last_synced_at,
                    last_error_kind: s.last_error_kind,
                })
                .collect();
        }
        Err(err) => report.database_error = Some(core_diag::redact(&err.to_string())),
    }
    report.ai = crate::ai::doctor_checks(Store::open_read_only(&db).ok().as_ref(), secrets);
    report
}

/// How many courses have an enrollment-window term (`RejectedDates::is_enrollment_window`; hidden
/// courses included, removed ones not), and how many removed courses still wait for their purge
/// or their files' move to the Trash.
fn course_notes(store: &Store) -> pagelamp_core::Result<(u32, u32)> {
    let windows = pagelamp_core::views::list_courses(store, true, AsOf::now_local())?
        .iter()
        .filter(|summary| {
            summary
                .timeline
                .term
                .not_used
                .iter()
                .any(|rejected| rejected.is_enrollment_window())
        })
        .count();
    let waiting = store
        .tombstones()?
        .iter()
        .filter(|tombstone| tombstone.state == TombstoneState::Pending || tombstone.files_pending)
        .count();
    Ok((
        u32::try_from(windows).unwrap_or(u32::MAX),
        u32::try_from(waiting).unwrap_or(u32::MAX),
    ))
}

/// Start the extraction worker once (`pagelamp_extract::worker::check`).
fn check_extract_worker(worker: Option<&Path>) -> ExtractWorkerCheck {
    use pagelamp_extract::worker::{self, WorkerFailure};
    let Some(exe) = worker else {
        return ExtractWorkerCheck {
            status: ExtractWorkerStatus::NotSet,
            spawn_ms: None,
        };
    };
    let (status, spawn_ms) = match worker::check(exe) {
        Ok(took) => (
            ExtractWorkerStatus::Ok,
            Some(u32::try_from(took.as_millis()).unwrap_or(u32::MAX)),
        ),
        Err(WorkerFailure::SpawnFailed) => (ExtractWorkerStatus::SpawnFailed, None),
        Err(WorkerFailure::ProtocolMismatch) => (ExtractWorkerStatus::ProtocolMismatch, None),
        Err(_) => (ExtractWorkerStatus::Failed, None),
    };
    ExtractWorkerCheck { status, spawn_ms }
}

/// "openai (key present), ollama (on this computer, running)": the providers in one line.
pub fn describe_ai_providers(ai: &crate::ai::AiDoctor) -> String {
    if ai.providers.is_empty() {
        return "none".to_string();
    }
    ai.providers
        .iter()
        .map(|p| {
            let mut facts = Vec::new();
            match p.key_present {
                Some(true) => facts.push("key present"),
                Some(false) => facts.push("key MISSING"),
                None => {}
            }
            if p.on_device {
                facts.push("on this computer");
            }
            match p.reachable {
                Some(true) => facts.push("running"),
                Some(false) => facts.push("not running"),
                None => {}
            }
            format!("{} ({})", p.preset, facts.join(", "))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// "Ollama running · LM Studio not running".
pub fn describe_local_servers(ai: &crate::ai::AiDoctor) -> String {
    if ai.local_servers.is_empty() {
        return "not checked".to_string();
    }
    ai.local_servers
        .iter()
        .map(|server| {
            format!(
                "{} {}",
                match server.kind {
                    crate::ai::LocalServerKind::Ollama => "Ollama",
                    crate::ai::LocalServerKind::LmStudio => "LM Studio",
                },
                if server.running {
                    "running"
                } else {
                    "not running"
                }
            )
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// "ok (14 ms)", "could not start (spawn_failed; …)": the worker check in one line.
pub fn describe_worker_check(check: &ExtractWorkerCheck) -> String {
    match check.status {
        ExtractWorkerStatus::Ok => match check.spawn_ms {
            Some(ms) => format!("ok ({ms} ms)"),
            None => "ok".to_string(),
        },
        ExtractWorkerStatus::NotSet => "not set (files are read in the app's process)".to_string(),
        ExtractWorkerStatus::SpawnFailed => {
            "could not start (spawn_failed; security software may block it)".to_string()
        }
        ExtractWorkerStatus::ProtocolMismatch => {
            "another version answered (protocol_mismatch; reinstall)".to_string()
        }
        ExtractWorkerStatus::Failed => "started but did not answer properly (failed)".to_string(),
    }
}

/// "2 timed_out, 1 memory_limit" (codes only).
pub fn describe_unreadable(files: &[UnreadableFiles]) -> String {
    let counts: Vec<String> = files
        .iter()
        .map(|files| format!("{} {}", files.count, files.kind.as_str()))
        .collect();
    counts.join(", ")
}

/// The last migration's backup record (`Store::last_migration_backup`), if the database can
/// be read and was ever migrated.
pub(crate) fn last_migration_backup_in(data_dir: &Path) -> Option<MigrationBackupRecord> {
    Store::open_read_only(&paths::db_path_in(data_dir))
        .ok()?
        .last_migration_backup()
        .ok()?
}

/// "schema 2 → 3 on 2026-10-01, backup ok" (codes only).
pub fn describe_update(record: &MigrationBackupRecord) -> String {
    let outcome = match &record.outcome {
        MigrationBackupOutcome::Ok => "backup ok".to_string(),
        MigrationBackupOutcome::Skipped => "backup skipped (another process migrated)".to_string(),
        MigrationBackupOutcome::Failed { code } => format!("NO backup ({code})"),
    };
    format!(
        "schema {} → {} on {}, {outcome}",
        record.from_version,
        record.to_version,
        record.at.format("%Y-%m-%d")
    )
}

pub(crate) fn report_in(
    data_dir: &Path,
    secrets: &dyn SecretBackend,
    worker: Option<&Path>,
) -> String {
    let doctor = doctor_in(data_dir, secrets, worker);
    let names = course_names(data_dir);
    let yes = |b: bool| if b { "yes" } else { "no" };
    let mut out = String::new();
    out.push_str(&format!("# {} diagnostic report\n\n", brand::PRODUCT_NAME));
    out.push_str(&format!(
        "Generated {}. Please read it before sharing: it contains no tokens, calendar-feed \
         links, course material text or course names, but check anyway.\n\n",
        chrono::Utc::now().format("%Y-%m-%d %H:%M UTC")
    ));
    out.push_str("## System\n\n");
    out.push_str(&format!("- Version: {}\n", doctor.version));
    out.push_str(&format!("- OS: {} ({})\n", doctor.os, doctor.arch));
    out.push_str(&format!("- Data folder: {}\n", doctor.data_dir));
    match (&doctor.schema_version, &doctor.database_error) {
        (Some(v), _) => out.push_str(&format!("- Database: schema {v}\n")),
        (None, Some(err)) => out.push_str(&format!("- Database: not readable ({err})\n")),
        (None, None) => out.push_str("- Database: unknown\n"),
    }
    if let Some(update) = last_migration_backup_in(data_dir) {
        out.push_str(&format!(
            "- Last database update: {}\n",
            describe_update(&update)
        ));
    }
    let last_check: Option<crate::UpdateCheckRecord> =
        Store::open_read_only(&paths::db_path_in(data_dir))
            .ok()
            .and_then(|store| store.setting("updates.last_check").unwrap_or(None));
    if let Some(check) = last_check {
        out.push_str(&format!(
            "- Last update check: {}\n",
            crate::updates::describe_check(&check)
        ));
    }
    match &doctor.keychain_error {
        None => out.push_str("- Keychain: available\n"),
        Some(err) => out.push_str(&format!("- Keychain: NOT available ({err})\n")),
    }
    out.push_str(&format!(
        "- Courses: {} ({} hidden) · materials: {} · events: {}\n",
        doctor.courses, doctor.hidden_courses, doctor.materials, doctor.events
    ));
    if doctor.enrollment_window_terms > 0 {
        out.push_str(&format!(
            "- Course terms not used to count weeks (they look like enrollment windows): {}\n",
            doctor.enrollment_window_terms
        ));
    }
    if doctor.removals_waiting > 0 {
        out.push_str(&format!(
            "- Removed courses waiting (for their purge or for files to go to the Trash): {}\n",
            doctor.removals_waiting
        ));
    }
    out.push_str(&format!(
        "- Extraction worker: {}\n",
        describe_worker_check(&doctor.extract_worker)
    ));
    if !doctor.unreadable_files.is_empty() {
        out.push_str(&format!(
            "- Unreadable files: {}\n",
            describe_unreadable(&doctor.unreadable_files)
        ));
    }
    out.push_str(&format!(
        "- AI providers: {}\n- Local model servers: {}\n",
        describe_ai_providers(&doctor.ai),
        describe_local_servers(&doctor.ai)
    ));
    out.push_str(&format!(
        "- AI apps configured: Claude Desktop {} · Claude Code {} · Codex {}\n",
        yes(doctor.mcp_clients.claude_desktop),
        yes(doctor.mcp_clients.claude_code),
        yes(doctor.mcp_clients.codex)
    ));
    out.push_str("\n## Sources\n\n");
    if doctor.sources.is_empty() {
        out.push_str("- none\n");
    }
    for source in &doctor.sources {
        out.push_str(&format!(
            "- {}: {}{}{}\n",
            source.kind.as_str(),
            if source.ok { "ok" } else { "not ok" },
            source
                .last_synced_at
                .map(|t| format!(", last synced {}", t.format("%Y-%m-%d %H:%M UTC")))
                .unwrap_or_default(),
            source
                .last_error_kind
                .map(|k| format!(", last error: {}", k.as_str()))
                .unwrap_or_default(),
        ));
    }
    out.push_str("\n## Last crash\n\n");
    match &doctor.last_crash {
        None => out.push_str("none\n"),
        Some(crash) => out.push_str(&format!(
            "{} · version {} · {} process · {}{}\n",
            crash.time.format("%Y-%m-%d %H:%M UTC"),
            crash.version,
            match crash.process {
                ProcessKind::App => "app",
                ProcessKind::Mcp => "MCP",
            },
            pseudonymise(&crash.message, &names),
            crash
                .location
                .as_deref()
                .map(|l| format!(" (at {l})"))
                .unwrap_or_default()
        )),
    }
    out.push_str(&format!(
        "\n## Recent log (last {REPORT_LOG_LINES} lines)\n\n```text\n"
    ));
    let lines = core_diag::recent_log_lines(data_dir, REPORT_LOG_LINES);
    if lines.is_empty() {
        out.push_str("(no log lines yet)\n");
    }
    for line in lines {
        // Keep the fence intact whatever the line contains.
        out.push_str(&pseudonymise(&line, &names).replace("```", "'''"));
        out.push('\n');
    }
    out.push_str("```\n");
    core_diag::redact(&out)
}

// ----- course names: pseudonymisation --------------------------------------------------------

/// `<data_dir>/course-aliases.json` (`paths::course_aliases_path_in`, mode 0600): the names,
/// codes and folder names of every course seen, by course id, kept `ALIAS_RETENTION_DAYS`
/// after it was last seen (longer than the 7 days logs are kept). A report uses it to hide
/// courses that were renamed or removed after their names were logged. Local only, never
/// part of a report, and outside `logs/` (which students are invited to share).
const ALIAS_RETENTION_DAYS: i64 = 30;

#[derive(Default, PartialEq, Serialize, Deserialize)]
struct RememberedCourses {
    /// By course id.
    courses: BTreeMap<String, RememberedCourse>,
}

#[derive(PartialEq, Serialize, Deserialize)]
struct RememberedCourse {
    names: BTreeSet<String>,
    last_seen: NaiveDate,
}

/// The texts a course may appear as in logs: display name, name, code, and the folder name
/// of a folder course (an id with letters in it; numeric LMS ids are left alone, they would
/// match unrelated numbers).
fn names_of(course: &Course) -> impl Iterator<Item = String> {
    let folder_name =
        Some(course.external_id.clone()).filter(|id| id.chars().any(char::is_alphabetic));
    [
        Some(course.display_name()),
        Some(course.name.clone()),
        course.code.clone(),
        folder_name,
    ]
    .into_iter()
    .flatten()
}

/// The remembered courses still within `ALIAS_RETENTION_DAYS`, and whether expired entries
/// were dropped (the file then needs rewriting). Also reads the file where versions before
/// 0.1.0-beta.1 kept it (`logs/`).
fn read_remembered(data_dir: &Path) -> (RememberedCourses, bool) {
    let read = |path: PathBuf| -> RememberedCourses {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    };
    let mut remembered = read(paths::course_aliases_path_in(data_dir));
    for (id, old) in read(legacy_alias_path(data_dir)).courses {
        let entry = remembered.courses.entry(id).or_insert(RememberedCourse {
            names: BTreeSet::new(),
            last_seen: old.last_seen,
        });
        entry.names.extend(old.names);
        entry.last_seen = entry.last_seen.max(old.last_seen);
    }
    let oldest = Local::now().date_naive() - TimeDelta::days(ALIAS_RETENTION_DAYS);
    let before = remembered.courses.len();
    remembered.courses.retain(|_, c| c.last_seen >= oldest);
    let expired = remembered.courses.len() != before;
    (remembered, expired)
}

fn legacy_alias_path(data_dir: &Path) -> PathBuf {
    core_diag::logs_dir_in(data_dir).join("course-aliases.json")
}

/// Add `courses` (all current courses) to the remembered names. Called at startup, and
/// before and after every source sync or removal. Writes only when something changed.
pub(crate) fn remember_courses(data_dir: &Path, courses: &[Course]) -> std::io::Result<()> {
    let today = Local::now().date_naive();
    let (mut remembered, expired) = read_remembered(data_dir);
    let unchanged = courses.iter().all(|course| {
        remembered.courses.get(&course.id).is_some_and(|known| {
            known.last_seen == today && names_of(course).all(|name| known.names.contains(&name))
        })
    });
    for course in courses {
        let entry = remembered
            .courses
            .entry(course.id.clone())
            .or_insert_with(|| RememberedCourse {
                names: BTreeSet::new(),
                last_seen: today,
            });
        entry.names.extend(names_of(course));
        entry.last_seen = today;
    }
    let legacy = legacy_alias_path(data_dir);
    let path = paths::course_aliases_path_in(data_dir);
    // Expired entries must leave the file too (PRIVACY.md), even when nothing else changed.
    if !unchanged || expired || !path.exists() || legacy.exists() {
        paths::create_private_dir_all(data_dir)?;
        // Per process: two processes may remember at the same time.
        let temp = data_dir.join(format!("course-aliases.json.{}.tmp", std::process::id()));
        let mut options = std::fs::OpenOptions::new();
        options.create(true).write(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        std::io::Write::write_all(&mut options.open(&temp)?, &serde_json::to_vec(&remembered)?)?;
        std::fs::rename(&temp, &path)?;
        let _ = std::fs::remove_file(legacy);
    }
    Ok(())
}

/// Course names for pseudonymisation ("Course 1", "Course 2", …): the current courses first,
/// then remembered ones that no longer exist. Each name matches case-insensitively, also in
/// its Debug-escaped and JSON-escaped forms (how log lines quote it).
fn course_names(data_dir: &Path) -> Vec<(Regex, String)> {
    let current = Store::open_read_only(&paths::db_path_in(data_dir))
        .and_then(|store| store.list_courses(true))
        .unwrap_or_default();
    let mut remembered = read_remembered(data_dir).0.courses;
    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut add = |names: &mut dyn Iterator<Item = String>, alias: &str| {
        pairs.extend(names.map(|name| (name, alias.to_string())));
    };
    for (index, course) in current.iter().enumerate() {
        let alias = format!("Course {}", index + 1);
        add(&mut names_of(course), &alias);
        if let Some(old) = remembered.remove(&course.id) {
            add(&mut old.names.into_iter(), &alias);
        }
    }
    for (index, old) in remembered.into_values().enumerate() {
        let alias = format!("Course {}", current.len() + index + 1);
        add(&mut old.names.into_iter(), &alias);
    }
    let mut variants: Vec<(String, String)> = Vec::new();
    // Names of digits and punctuation only ("2026", "101") would match timestamps and ids.
    pairs.retain(|(name, _)| name.chars().any(char::is_alphabetic));
    for (name, alias) in pairs {
        let json = serde_json::to_string(&name).unwrap_or_default();
        let json = json.trim_matches('"').to_string();
        for variant in [name.escape_debug().to_string(), json, name] {
            if variant.trim().chars().count() >= 3 && !variants.iter().any(|(v, _)| *v == variant) {
                variants.push((variant, alias.clone()));
            }
        }
    }
    // Longest first, so "DEMO101 — Intro" is replaced before "DEMO101".
    variants.sort_by_key(|(variant, _)| std::cmp::Reverse(variant.len()));
    variants
        .into_iter()
        .filter_map(|(variant, alias)| {
            let pattern = format!("(?i){}", regex::escape(&variant));
            Some((Regex::new(&pattern).ok()?, alias))
        })
        .collect()
}

/// Anything shaped like a course code ("DEMO101", "MAT 137Y1") that no known name covered,
/// e.g. a course removed before its name was remembered, or text an AI app sent.
static COURSE_CODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b([A-Z]{2,4})\s?\d{3}[A-Z0-9]*\b").expect("valid regex"));

/// Upper-case words followed by a number that are not course codes.
const NOT_COURSE_PREFIXES: &[&str] = &[
    "API", "CPU", "GB", "GET", "GMT", "HTTP", "ISO", "KB", "MB", "MS", "OS", "PID", "RFC", "SHA",
    "SSL", "TB", "TLS", "URL", "UTC", "UTF",
];

fn pseudonymise(text: &str, names: &[(Regex, String)]) -> String {
    let mut text = text.to_string();
    for (name, alias) in names {
        if !name.is_match(&text) {
            continue;
        }
        // A whole-word match only: where the name starts or ends with a word character, the
        // text next to it must not be one ("Art" is not replaced inside "Started").
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        for found in name.find_iter(&text) {
            let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
            let (start, end) = (found.start(), found.end());
            let inner_start = text[start..].chars().next();
            let inner_end = text[..end].chars().next_back();
            let outer_start = text[..start].chars().next_back();
            let outer_end = text[end..].chars().next();
            let whole =
                !(word(inner_start) && word(outer_start)) && !(word(inner_end) && word(outer_end));
            out.push_str(&text[last..start]);
            out.push_str(if whole { alias } else { found.as_str() });
            last = end;
        }
        out.push_str(&text[last..]);
        text = out;
    }
    COURSE_CODE
        .replace_all(&text, |caps: &regex::Captures<'_>| {
            if NOT_COURSE_PREFIXES.contains(&&caps[1]) {
                caps[0].to_string()
            } else {
                "[course code]".to_string()
            }
        })
        .into_owned()
}

// ----- AI app config presence ------------------------------------------------------------------

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Whether each AI app's config file mentions a `pagelamp` server entry. Only the presence
/// of that key is checked; nothing else from these files is kept or shown.
fn mcp_client_presence() -> McpClientPresence {
    let key = brand::MCP_SERVER_KEY;
    let json_has_server = |path: PathBuf, check_projects: bool| -> bool {
        let Ok(text) = std::fs::read_to_string(path) else {
            return false;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
            return false;
        };
        let top = json.get("mcpServers").and_then(|s| s.get(key)).is_some();
        let in_project = check_projects
            && json
                .get("projects")
                .and_then(|p| p.as_object())
                .is_some_and(|projects| {
                    projects
                        .values()
                        .any(|p| p.get("mcpServers").and_then(|s| s.get(key)).is_some())
                });
        top || in_project
    };
    let desktop_config = if cfg!(target_os = "macos") {
        home().map(|h| h.join("Library/Application Support/Claude/claude_desktop_config.json"))
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(|a| {
            PathBuf::from(a)
                .join("Claude")
                .join("claude_desktop_config.json")
        })
    } else {
        home().map(|h| h.join(".config/Claude/claude_desktop_config.json"))
    };
    let codex_config = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| home().map(|h| h.join(".codex")))
        .map(|dir| dir.join("config.toml"));
    McpClientPresence {
        claude_desktop: desktop_config.is_some_and(|p| json_has_server(p, false)),
        claude_code: home().is_some_and(|h| json_has_server(h.join(".claude.json"), true)),
        codex: codex_config.is_some_and(|p| {
            std::fs::read_to_string(p).is_ok_and(|text| {
                text.lines().any(|line| {
                    let line = line.trim();
                    line == format!("[mcp_servers.{key}]")
                        || line == format!("[mcp_servers.\"{key}\"]")
                })
            })
        }),
    }
}

#[cfg(test)]
mod tests {
    use pagelamp_core::model::*;
    use pagelamp_core::secrets::MemorySecrets;
    use serde_json::json;

    use super::*;

    fn seeded(dir: &Path) {
        let store = Store::open(&paths::db_path_in(dir)).unwrap();
        store
            .upsert_source(&SourceRecord {
                id: "ical:secretid".into(),
                kind: SourceKind::Ical,
                label: "Demo calendar https://calendar.example.edu/feeds/x.ics".into(),
                config: json!({}),
                last_synced_at: None,
                last_error: None,
                last_error_kind: None,
            })
            .unwrap();
        store
            .record_sync(
                "ical:secretid",
                chrono::Utc::now(),
                Some((SourceErrorKind::AuthExpiredOrRevoked, "feed rejected")),
            )
            .unwrap();
        let folder = SourceRecord {
            id: "folder:x".into(),
            kind: SourceKind::Folder,
            label: "Courses".into(),
            config: json!({}),
            last_synced_at: None,
            last_error: None,
            last_error_kind: None,
        };
        store.upsert_source(&folder).unwrap();
        store
            .upsert_course(&CourseUpsert {
                id: "folder:x/course/DEMO101".into(),
                source_id: "folder:x".into(),
                external_id: "DEMO101".into(),
                code: Some("DEMO101".into()),
                name: "Intro to Demo Studies".into(),
                term_start: None,
                term_end: None,
                url: None,
                syllabus_text: None,
                lms: Default::default(),
            })
            .unwrap();
    }

    /// The doctor note (calendar design §6.3, §8.3): counts of enrollment-window terms and of
    /// removed courses still waiting, never their names; 0 when the database can't be read.
    #[test]
    fn doctor_counts_enrollment_window_terms_and_waiting_removals() {
        let temp = tempfile::tempdir().unwrap();
        seeded(temp.path());
        let doctor = doctor_in(temp.path(), &MemorySecrets::new(), None);
        assert_eq!(
            (doctor.enrollment_window_terms, doctor.removals_waiting),
            (0, 0)
        );

        let store = Store::open(&paths::db_path_in(temp.path())).unwrap();
        store
            .upsert_source(&SourceRecord {
                id: "canvas:lms.example.edu".into(),
                kind: SourceKind::Canvas,
                label: "lms.example.edu".into(),
                config: json!({ "base_url": "https://lms.example.edu" }),
                last_synced_at: None,
                last_error: None,
                last_error_kind: None,
            })
            .unwrap();
        let date = |text: &str| NaiveDate::parse_from_str(text, "%Y-%m-%d").ok();
        for (external, name) in [("404", "Demo Methods"), ("505", "Demo Seminar")] {
            store
                .upsert_course(&CourseUpsert {
                    id: format!("canvas:lms.example.edu/course/{external}"),
                    source_id: "canvas:lms.example.edu".into(),
                    external_id: external.into(),
                    code: Some(format!("DEMO{external}")),
                    name: name.into(),
                    term_start: date("2026-05-04"),
                    term_end: date("2027-01-31"),
                    url: None,
                    syllabus_text: None,
                    // A UofT-like "Fall 2026" term (CAL-1): May to January.
                    lms: LmsCourseInfo {
                        term_name: Some("Fall 2026".into()),
                        term_start: date("2026-05-04"),
                        term_end: date("2027-01-31"),
                        ..LmsCourseInfo::default()
                    },
                })
                .unwrap();
        }
        store
            .set_course_hidden("canvas:lms.example.edu/course/404", true)
            .unwrap();
        store
            .remove_course(
                "canvas:lms.example.edu/course/505",
                pagelamp_core::removal::RemovalReason::Ended,
                false,
                false,
                chrono::Utc::now(),
            )
            .unwrap();
        let doctor = doctor_in(temp.path(), &MemorySecrets::new(), None);
        // The hidden course counts; the removed one is gone from every list.
        assert_eq!(doctor.enrollment_window_terms, 1);
        assert_eq!(doctor.removals_waiting, 1);
        let text = report_in(temp.path(), &MemorySecrets::new(), None);
        assert!(text.contains("look like enrollment windows): 1"), "{text}");
        assert!(text.contains("for files to go to the Trash): 1"), "{text}");
        assert!(!text.contains("Demo Methods") && !text.contains("Demo Seminar"));

        // An unreadable database: the counts stay 0.
        std::fs::write(paths::db_path_in(temp.path()), b"not a database").unwrap();
        let doctor = doctor_in(temp.path(), &MemorySecrets::new(), None);
        assert!(doctor.database_error.is_some());
        assert_eq!(
            (doctor.enrollment_window_terms, doctor.removals_waiting),
            (0, 0)
        );
    }

    #[test]
    fn doctor_reports_facts_but_no_urls_ids_or_labels() {
        let temp = tempfile::tempdir().unwrap();
        seeded(temp.path());
        let doctor = doctor_in(temp.path(), &MemorySecrets::new(), None);
        assert_eq!(
            doctor.schema_version,
            Some(pagelamp_core::store::SCHEMA_VERSION)
        );
        assert!(doctor.keychain_available);
        assert_eq!(doctor.courses, 1);
        assert_eq!(doctor.sources.len(), 2);
        let ical = doctor
            .sources
            .iter()
            .find(|s| s.kind == SourceKind::Ical)
            .unwrap();
        assert!(!ical.ok);
        assert_eq!(
            ical.last_error_kind,
            Some(SourceErrorKind::AuthExpiredOrRevoked)
        );
        let json = serde_json::to_string(&doctor).unwrap();
        assert!(
            !json.contains("secretid") && !json.contains("calendar.example.edu"),
            "{json}"
        );

        // Without a database it still answers, and says why.
        let empty = tempfile::tempdir().unwrap();
        let doctor = doctor_in(empty.path(), &MemorySecrets::new(), None);
        assert!(doctor.database_error.is_some() && doctor.schema_version.is_none());
    }

    #[test]
    fn report_is_redacted_and_pseudonymised() {
        let temp = tempfile::tempdir().unwrap();
        seeded(temp.path());
        let logs = core_diag::logs_dir_in(temp.path());
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(
            logs.join(format!("app-{}.log", chrono::Local::now().format("%Y-%m-%d"))),
            "2026-09-26T10:00:00Z pid=1 INFO pagelamp: synced DEMO101 — Intro to Demo Studies\n\
             2026-09-26T10:00:01Z pid=1 DEBUG pagelamp_canvas::http: GET /files/1/download?verifier=S3CR3T → 302\n\
             2026-09-26T10:00:02Z pid=1 WARN pagelamp: Bearer 1234~AbCdEfGhIjKlMnOpQrStUvWx rejected\n\
             2026-09-26T10:00:03Z pid=1 INFO pagelamp: feed https://calendar.example.edu/feeds/calendars/user_T0K3N.ics\n",
        )
        .unwrap();
        let report = report_in(temp.path(), &MemorySecrets::new(), None);
        for secret in [
            "S3CR3T",
            "AbCdEfGh",
            "T0K3N",
            "DEMO101",
            "Intro to Demo Studies",
            "secretid",
        ] {
            assert!(!report.contains(secret), "{secret} leaked:\n{report}");
        }
        assert!(report.contains("synced Course 1"), "{report}");
        assert!(report.contains("- ical: not ok"), "{report}");
        assert!(report.contains("## Recent log"));
    }

    #[test]
    fn the_report_says_how_the_last_database_update_went() {
        let temp = tempfile::tempdir().unwrap();
        let db = paths::db_path_in(temp.path());
        let plain = rusqlite::Connection::open(&db).unwrap();
        plain
            .execute_batch(pagelamp_core::store::SCHEMA_V1)
            .unwrap();
        plain
            .execute_batch(pagelamp_core::store::SCHEMA_V2)
            .unwrap();
        plain.pragma_update(None, "user_version", 2).unwrap();
        drop(plain);
        drop(Store::open(&db).unwrap()); // migrates, with a backup
        let report = report_in(temp.path(), &MemorySecrets::new(), None);
        let line = report
            .lines()
            .find(|l| l.starts_with("- Last database update:"))
            .unwrap_or_else(|| panic!("no update line:\n{report}"));
        let expected = format!("schema 2 → {}", pagelamp_core::store::SCHEMA_VERSION);
        assert!(line.contains(&expected), "{line}");
        assert!(line.ends_with("backup ok"), "{line}");
        assert!(
            !report.contains(".bak"),
            "no backup path in reports:\n{report}"
        );
    }

    #[test]
    fn short_or_numeric_course_names_never_mangle_other_words() {
        let temp = tempfile::tempdir().unwrap();
        seeded(temp.path());
        let store = Store::open(&paths::db_path_in(temp.path())).unwrap();
        for (dir, name) in [("Art", "Art"), ("2026", "2026")] {
            store
                .upsert_course(&CourseUpsert {
                    id: format!("folder:x/course/{dir}"),
                    source_id: "folder:x".into(),
                    external_id: dir.into(),
                    code: None,
                    name: name.into(),
                    term_start: None,
                    term_end: None,
                    url: None,
                    syllabus_text: None,
                    lms: Default::default(),
                })
                .unwrap();
        }
        let names = course_names(temp.path());
        let line = "2026-09-26T10:00:00Z Started the Art sync (Artwork, art.)";
        let out = pseudonymise(line, &names);
        assert!(
            out.starts_with("2026-09-26T10:00:00Z Started the Course"),
            "{out}"
        );
        assert!(out.contains("(Artwork, Course"), "{out}");
    }

    #[test]
    fn expired_aliases_leave_the_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = paths::course_aliases_path_in(temp.path());
        std::fs::write(
            &path,
            r#"{"courses":{"folder:x/course/OLD":{"names":["Old Demo Course"],"last_seen":"2000-01-01"}}}"#,
        )
        .unwrap();
        remember_courses(temp.path(), &[]).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("Old Demo Course"), "{text}");
        assert!(
            std::fs::read_dir(temp.path())
                .unwrap()
                .flatten()
                .all(|e| !e.file_name().to_string_lossy().ends_with(".tmp"))
        );
    }

    #[test]
    fn course_names_match_in_any_case_escaped_or_as_folder_names() {
        let temp = tempfile::tempdir().unwrap();
        seeded(temp.path());
        let store = Store::open(&paths::db_path_in(temp.path())).unwrap();
        store
            .upsert_course(&CourseUpsert {
                id: "folder:x/course/Café Seminar".into(),
                source_id: "folder:x".into(),
                external_id: "Café Seminar folder".into(),
                code: None,
                name: "Café \"Demo\" Seminar".into(),
                term_start: None,
                term_end: None,
                url: None,
                syllabus_text: None,
                lms: Default::default(),
            })
            .unwrap();
        let names = course_names(temp.path());
        for line in [
            // How `{:?}` quotes it (printable Unicode stays, quotes are escaped).
            &format!("warning: {:?}: file skipped", "Café \"Demo\" Seminar"),
            r#"{"course":"Café \"Demo\" Seminar"}"#,
            "could not read Café Seminar folder/Week 1",
            "CAFÉ \"DEMO\" SEMINAR",
        ] {
            let out = pseudonymise(line, &names);
            assert!(
                !out.contains("Seminar") && !out.contains("SEMINAR"),
                "{line} → {out}"
            );
        }
    }

    #[test]
    fn report_hides_courses_removed_after_they_were_logged() {
        let temp = tempfile::tempdir().unwrap();
        seeded(temp.path());
        let store = Store::open(&paths::db_path_in(temp.path())).unwrap();
        remember_courses(temp.path(), &store.list_courses(true).unwrap()).unwrap();
        // Removed with its source: the DB no longer knows the name.
        store.remove_source("folder:x").unwrap();
        let logs = core_diag::logs_dir_in(temp.path());
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(
            logs.join(format!(
                "mcp-{}.log",
                chrono::Local::now().format("%Y-%m-%d")
            )),
            "2026-09-26T10:00:00Z pid=1 WARN rmcp: no course matches 'Intro to Demo Studies'\n\
             2026-09-26T10:00:00Z pid=1 WARN rmcp: no course matches 'intro TO demo studies'\n\
             2026-09-26T10:00:01Z pid=1 WARN rmcp: no course matches 'XYZ 204H1'\n\
             2026-09-26T10:00:02Z pid=1 WARN pagelamp: Canvas answered HTTP 404 at 12:00 UTC\n",
        )
        .unwrap();
        let report = report_in(temp.path(), &MemorySecrets::new(), None);
        for name in ["Intro to Demo Studies", "intro TO demo", "XYZ 204H1", "XYZ"] {
            assert!(!report.contains(name), "{name} leaked:\n{report}");
        }
        assert!(report.contains("matches 'Course 1'"), "{report}");
        assert!(report.contains("matches '[course code]'"), "{report}");
        assert!(report.contains("HTTP 404"), "{report}");
        // Kept outside logs/ (which students may share), private, never in a report.
        let aliases = paths::course_aliases_path_in(temp.path());
        assert!(aliases.is_file());
        assert!(!logs.join("course-aliases.json").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&aliases).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0);
        }
    }
}
