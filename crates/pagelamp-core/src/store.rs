//! SQLite store.
//!
//! Concurrency model (deliberately simple — see docs/ARCHITECTURE.md "Concurrency"):
//! - One writer for synced data: the `pagelamp sync` process (CLI / future desktop app).
//! - Any number of MCP server processes (one per AI client) open the DB with
//!   `Store::open_read_only`. WAL mode lets readers proceed while a sync writes.
//! - The only write an MCP process performs is `save_study_plan`, via a short-lived
//!   read-write connection (`Store::open`) — short transactions + `busy_timeout` suffice.
//! - `Store` wraps a single `rusqlite::Connection` (not `Sync`). Async callers open one
//!   connection per request inside `spawn_blocking`; opening SQLite is sub-millisecond.
//!   There is intentionally no connection pool and no global mutex.
//!
//! Schema versioning: `PRAGMA user_version`. `open` migrates forward; `open_read_only`
//! refuses a DB whose version is newer than `SCHEMA_VERSION` (`Error::SchemaTooNew`) or older
//! (`Error::SchemaTooOld`, until a read-write `open` migrated it) and returns
//! `Error::NotInitialised` for a missing file or version 0.
//!
//! How values are stored as TEXT (each direction goes through exactly one helper, see the
//! "text encodings" section at the bottom of this file):
//! - Instants: RFC 3339, UTC, whole seconds, `Z` suffix — `2026-09-25T12:00:00Z`. Every
//!   stored instant has this exact shape, so comparing/sorting the TEXT in SQL (`BETWEEN`,
//!   `ORDER BY`) gives time order. Never bind a `DateTime` directly: rusqlite's chrono support
//!   would write a different format (`2026-09-25 12:00:00.000+00:00`). Instants outside
//!   years 0000–9999 are clamped into that range (see `ts_text`).
//! - Calendar dates: `YYYY-MM-DD`.
//! - Enums: their `as_str()` text. An unknown stored value is a read error, never a panic.
//!
//! Atomicity: `in_transaction` groups several calls into one `BEGIN IMMEDIATE` transaction.
//! Methods that run several statements (`replace_*`, `prune_*`) also use a SAVEPOINT, so they
//! are all-or-nothing both when called on their own and inside `in_transaction`.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, NaiveDate, SecondsFormat, SubsecRound, Utc};
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ValueRef};
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Params, Row, Transaction, TransactionBehavior, params,
};

use crate::model::*;
use crate::{Error, Result};

mod ai;
mod calendars;
mod generations;
mod migrate_v4;
mod tombstones;

pub use ai::USAGE_KEEP_DAYS;
pub use calendars::{
    CalendarChecks, CalendarProvenance, CalendarRow, CalendarState, KEEP_DISMISSED_DAYS,
    KEEP_SUPERSEDED, NewCalendarRow, Staleness,
};
pub use generations::{GenerationRecord, GenerationStatus, KEEP_GENERATIONS};
pub use migrate_v4::{COURSE_DATES_CONFIRMED, MIGRATED_FINGERPRINT};

pub const SCHEMA_VERSION: i64 = 4;

/// Version-1 schema. Applied by `open` when `user_version` is 0.
pub const SCHEMA_V1: &str = r#"
CREATE TABLE sources (
    id              TEXT PRIMARY KEY,
    kind            TEXT NOT NULL,              -- canvas | folder | ical
    label           TEXT NOT NULL,
    config_json     TEXT NOT NULL DEFAULT '{}',
    last_synced_at  TEXT,
    last_error      TEXT,
    last_error_kind TEXT                        -- SourceErrorKind (snake_case) or NULL
);

CREATE TABLE courses (
    id               TEXT PRIMARY KEY,
    source_id        TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    external_id      TEXT NOT NULL,
    code             TEXT,
    name             TEXT NOT NULL,
    term_start       TEXT,                      -- synced (YYYY-MM-DD)
    term_end         TEXT,
    user_term_start  TEXT,                      -- user override, wins over synced
    user_term_end    TEXT,
    url              TEXT,
    syllabus_text    TEXT,
    ai_policy        TEXT NOT NULL DEFAULT 'unknown',
    ai_policy_note   TEXT,
    ai_access        INTEGER NOT NULL DEFAULT 1, -- student's "AI may read materials" switch
    hidden           INTEGER NOT NULL DEFAULT 0,
    enrollment_active INTEGER NOT NULL DEFAULT 1, -- 0: no longer in the LMS's active list
    updated_at       TEXT NOT NULL
);
CREATE INDEX courses_source ON courses(source_id);

CREATE TABLE modules (
    id          TEXT PRIMARY KEY,
    course_id   TEXT NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    position    INTEGER,
    unlock_at   TEXT,
    week_hint   INTEGER
);
CREATE INDEX modules_course ON modules(course_id);

CREATE TABLE materials (
    id            TEXT PRIMARY KEY,
    course_id     TEXT NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
    module_id     TEXT REFERENCES modules(id) ON DELETE SET NULL,
    kind          TEXT NOT NULL,
    title         TEXT NOT NULL,
    url           TEXT,
    local_path    TEXT,
    mime          TEXT,
    published_at  TEXT,
    week_hint     INTEGER,
    content_hash  TEXT,
    text_status   TEXT NOT NULL DEFAULT 'pending',
    text_error    TEXT,
    updated_at    TEXT NOT NULL
);
CREATE INDEX materials_course ON materials(course_id);

CREATE TABLE chunks (
    id           INTEGER PRIMARY KEY,
    material_id  TEXT NOT NULL REFERENCES materials(id) ON DELETE CASCADE,
    ord          INTEGER NOT NULL,
    locator      TEXT,
    text         TEXT NOT NULL,
    UNIQUE(material_id, ord)
);

-- External-content FTS5 index over chunks.text, kept in sync by triggers.
CREATE VIRTUAL TABLE chunks_fts USING fts5(
    text,
    content = 'chunks',
    content_rowid = 'id',
    tokenize = 'porter unicode61 remove_diacritics 2'
);
CREATE TRIGGER chunks_ai AFTER INSERT ON chunks BEGIN
    INSERT INTO chunks_fts(rowid, text) VALUES (new.id, new.text);
END;
CREATE TRIGGER chunks_ad AFTER DELETE ON chunks BEGIN
    INSERT INTO chunks_fts(chunks_fts, rowid, text) VALUES ('delete', old.id, old.text);
END;
CREATE TRIGGER chunks_au AFTER UPDATE ON chunks BEGIN
    INSERT INTO chunks_fts(chunks_fts, rowid, text) VALUES ('delete', old.id, old.text);
    INSERT INTO chunks_fts(rowid, text) VALUES (new.id, new.text);
END;

CREATE TABLE events (
    id          TEXT PRIMARY KEY,
    source_id   TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    course_id   TEXT REFERENCES courses(id) ON DELETE SET NULL,
    kind        TEXT NOT NULL,
    title       TEXT NOT NULL,
    starts_at   TEXT,
    ends_at     TEXT,
    due_at      TEXT,
    url         TEXT,
    updated_at  TEXT NOT NULL
);
CREATE INDEX events_course ON events(course_id);

CREATE TABLE study_plans (
    id          INTEGER PRIMARY KEY,
    created_at  TEXT NOT NULL,
    plan_json   TEXT NOT NULL
);
"#;

/// Version 2: `events.course_hint` (the course text a calendar feed gave, for relinking) and
/// `materials.download_blocked` (`DownloadBlock` or NULL).
pub const SCHEMA_V2: &str = r#"
ALTER TABLE events ADD COLUMN course_hint TEXT;
ALTER TABLE materials ADD COLUMN download_blocked TEXT;
"#;

/// Version 3 (v0.3 M0.8; docs/design/v0.3-model-access.md §5.4, v0.3-course-calendar.md §3.1):
/// - extraction worker failures: `materials.text_error_kind` (`TextErrorKind`) and
///   `text_error_fingerprint` (the worker protocol and app version that failed, kept next to
///   `content_hash`), so a hard failure is not retried until one of the three changes;
/// - `schema_meta` (key → value): `min_reader_version`, the oldest schema whose readers can
///   still read this database (the reader rule, `Store::open_read_only`);
/// - `settings` (key → JSON): app preferences such as the update settings (`Store::setting`);
/// - course columns: raw LMS dates, time zone and state (written only by sync) and the
///   student's `keep_current_until` / `removal_snoozed_until` (never touched by sync);
/// - clears v0.1 term overrides that are only the untouched prefill of the dates form (it saved
///   the synced dates as overrides): an override equal to the synced date is dropped.
pub const SCHEMA_V3: &str = r#"
ALTER TABLE materials ADD COLUMN text_error_kind        TEXT;
ALTER TABLE materials ADD COLUMN text_error_fingerprint TEXT;

CREATE TABLE schema_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
INSERT INTO schema_meta (key, value) VALUES ('min_reader_version', '3');

CREATE TABLE settings (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,                 -- JSON
    updated_at TEXT NOT NULL
);

ALTER TABLE courses ADD COLUMN lms_term_name         TEXT;    -- term.name, e.g. "Fall 2026"
ALTER TABLE courses ADD COLUMN lms_term_start        TEXT;    -- raw term dates, in the course time zone
ALTER TABLE courses ADD COLUMN lms_term_end          TEXT;
ALTER TABLE courses ADD COLUMN lms_course_start      TEXT;    -- raw course.start_at
ALTER TABLE courses ADD COLUMN lms_course_end        TEXT;
ALTER TABLE courses ADD COLUMN lms_time_zone         TEXT;    -- IANA name
ALTER TABLE courses ADD COLUMN lms_concluded         INTEGER; -- NULL = unknown
ALTER TABLE courses ADD COLUMN lms_workflow_state    TEXT;
ALTER TABLE courses ADD COLUMN lms_access_restricted INTEGER; -- listed but access_restricted_by_date
ALTER TABLE courses ADD COLUMN keep_current_until    TEXT;    -- "I'm still taking this" (student)
ALTER TABLE courses ADD COLUMN removal_snoozed_until TEXT;    -- "Not now" / "Keep" (student)

UPDATE courses SET user_term_start = NULL WHERE user_term_start = term_start;
UPDATE courses SET user_term_end   = NULL WHERE user_term_end   = term_end;
"#;

/// Version 4 (v0.3 M1; docs/design/v0.3-model-access.md §5.4, v0.3-course-calendar.md §3.2), all
/// additive, so `min_reader_version` stays 3 and a still-running v3 `pagelamp mcp` keeps working:
/// - AI: `model_providers` (no keys: those live in the keychain), `generations` (validated
///   output and a summary without text), the usage ledger `ai_usage` (counts and micro-USD
///   only), `reminders_shown`, `courses.material_sharing` (question (b), written only by the
///   student), `study_plans.origin` / `generation_id` / `ai_label_json` (the plan's
///   "AI-generated · backend · model · date" label, kept with the plan);
/// - course lane: `course_calendars` (proposed / accepted calendars, with evidence), the
///   removal `course_tombstones`, `courses.calendar_sources` and `institution` (sync-written),
///   `materials.linked_from_syllabus` / `is_front_page`; plus the data step
///   `Store::migrate_legacy_calendars` for PageLamp 0.1 term overrides.
pub const SCHEMA_V4: &str = r#"
CREATE TABLE model_providers (
    id              TEXT PRIMARY KEY,
    preset          TEXT NOT NULL,
    label           TEXT NOT NULL,
    wire            TEXT NOT NULL,            -- openai_responses | openai_chat | anthropic_messages | ollama_native
    base_url        TEXT NOT NULL,
    created_at      TEXT NOT NULL,
    last_probe_json TEXT                      -- the last "Test", no key
);

CREATE TABLE generations (
    id             TEXT PRIMARY KEY,          -- the caller's generation id
    feature        TEXT NOT NULL,             -- AiFeature
    course_id      TEXT REFERENCES courses(id) ON DELETE CASCADE,
    week           INTEGER,
    backend        TEXT NOT NULL,
    model          TEXT NOT NULL,
    status         TEXT NOT NULL,             -- draft | accepted | failed | cancelled
    created_at     TEXT NOT NULL,
    prompt_version INTEGER NOT NULL,
    output_json    TEXT,                      -- the validated answer
    summary_json   TEXT,                      -- context summary + manifest (no text)
    error_kind     TEXT,
    week_starts_on TEXT                       -- the week's first day when written (staleness)
);
CREATE INDEX generations_course ON generations(course_id, feature, week);

CREATE TABLE ai_usage (
    id             INTEGER PRIMARY KEY,
    at             TEXT NOT NULL,
    backend        TEXT NOT NULL,
    model          TEXT NOT NULL,
    feature        TEXT NOT NULL,
    input_uncached INTEGER NOT NULL,
    cache_read     INTEGER NOT NULL,
    cache_write    INTEGER NOT NULL,
    output         INTEGER NOT NULL,
    reasoning      INTEGER,
    micro_usd      INTEGER,                   -- NULL: price unknown, or a plan
    cost_basis     TEXT NOT NULL,             -- priced | free_on_device | unpriced | plan
    estimated      INTEGER NOT NULL DEFAULT 0,
    outcome        TEXT NOT NULL              -- ok | failed | cancelled
);
CREATE INDEX ai_usage_at ON ai_usage(at);

CREATE TABLE reminders_shown (
    id       TEXT PRIMARY KEY,
    shown_at TEXT NOT NULL
);

ALTER TABLE courses     ADD COLUMN material_sharing TEXT NOT NULL DEFAULT 'unanswered';
ALTER TABLE study_plans ADD COLUMN origin           TEXT NOT NULL DEFAULT 'ai_app';
ALTER TABLE study_plans ADD COLUMN generation_id    TEXT;
ALTER TABLE study_plans ADD COLUMN ai_label_json    TEXT;     -- origin pagelamp: AiLabel (JSON); stays after "Remove all AI data"

CREATE TABLE course_calendars (
    id              INTEGER PRIMARY KEY,
    course_id       TEXT NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
    origin          TEXT NOT NULL,            -- user | legacy | scan | ai | ai_app | restored
    state           TEXT NOT NULL,            -- proposed | accepted | dismissed | superseded
    calendar_json   TEXT NOT NULL,            -- validated CourseCalendar (dates, enums, short labels)
    evidence_json   TEXT NOT NULL,            -- per date: material_id, locator, quote (≤ 300 chars)
    checks_json     TEXT NOT NULL,            -- conflicts, drop counts per reason, agreement (no text)
    manifest_json   TEXT NOT NULL,            -- [{material_id, content_hash, chunk_ords}]
    fingerprint     TEXT NOT NULL,            -- hash of the candidate set, for dedupe
    generation_id   TEXT REFERENCES generations(id) ON DELETE SET NULL,
    backend         TEXT,
    model           TEXT,
    prompt_version  INTEGER,
    created_at      TEXT NOT NULL,
    decided_at      TEXT
);
CREATE UNIQUE INDEX course_calendars_accepted ON course_calendars(course_id) WHERE state = 'accepted';
CREATE UNIQUE INDEX course_calendars_proposed ON course_calendars(course_id, origin) WHERE state = 'proposed';
CREATE INDEX course_calendars_course ON course_calendars(course_id);

CREATE TABLE course_tombstones (
    source_id     TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    external_id   TEXT NOT NULL,
    course_id     TEXT NOT NULL,
    code          TEXT,
    name          TEXT NOT NULL,
    reason        TEXT NOT NULL,              -- ended | inactive | not_mine | other (display only)
    state         TEXT NOT NULL,              -- pending | purged | restoring
    removed_at    TEXT NOT NULL,
    purge_after   TEXT,                       -- removed_at + 7 days
    purged_at     TEXT,
    keep_files    INTEGER NOT NULL DEFAULT 0,
    files_pending INTEGER NOT NULL DEFAULT 0, -- files still to move to the Trash (or failed); retried
    delete_backup INTEGER NOT NULL DEFAULT 0, -- delete the pre-update backup at the purge
    settings_json TEXT NOT NULL,              -- the course's student settings, no quotes
    PRIMARY KEY (source_id, external_id)
);
CREATE INDEX course_tombstones_course ON course_tombstones(course_id);

ALTER TABLE courses   ADD COLUMN calendar_sources     TEXT;     -- the student's candidate edits (JSON)
ALTER TABLE courses   ADD COLUMN institution          TEXT;     -- sync-written (course.toml), never by setters
ALTER TABLE materials ADD COLUMN linked_from_syllabus INTEGER NOT NULL DEFAULT 0;
ALTER TABLE materials ADD COLUMN is_front_page        INTEGER NOT NULL DEFAULT 0;
ALTER TABLE materials ADD COLUMN named_outline        INTEGER NOT NULL DEFAULT 0;  -- course.toml `outline` (folders)

-- Additive only: a v3 reader still reads this database.
UPDATE schema_meta SET value = '3' WHERE key = 'min_reader_version';
"#;

/// Schema migrations, in order: `MIGRATIONS[i]` upgrades a database from `user_version` `i`
/// to `i + 1`. To change the schema, APPEND a migration (never edit one that has shipped) and
/// bump `SCHEMA_VERSION`; the assertion below keeps the two in step. A migration that only
/// adds things keeps `schema_meta.min_reader_version` as it is (older readers can still read
/// the database); one that changes what readers select must raise it.
const MIGRATIONS: &[&str] = &[SCHEMA_V1, SCHEMA_V2, SCHEMA_V3, SCHEMA_V4];
const _: () = assert!(MIGRATIONS.len() as i64 == SCHEMA_VERSION);

/// Settings key of the last migration's backup outcome (`MigrationBackupRecord`).
pub const LAST_MIGRATION_BACKUP: &str = "last_migration_backup";

/// How long a statement waits for another connection's lock before failing with "busy".
const BUSY_TIMEOUT: Duration = Duration::from_millis(5000);

/// `search` returns at most this many hits (a larger `limit` is clamped to it).
pub const MAX_SEARCH_LIMIT: u32 = 100;
/// Only the first this-many words of a search query are used.
const MAX_SEARCH_TERMS: usize = 32;
/// Longer "words" in a search query are cut to this many characters.
const MAX_SEARCH_TERM_CHARS: usize = 64;

// Limits checked by `save_study_plan` (plans are written by AI clients over MCP).
pub const MAX_PLAN_ITEMS: usize = 200;
pub const MAX_PLAN_TITLE_CHARS: usize = 300;
/// For `StudyPlan::notes` and `StudyPlanItem::description`.
pub const MAX_PLAN_TEXT_CHARS: usize = 4000;
/// For `StudyPlanItem::course_id` and each entry of `StudyPlanItem::material_ids`.
pub const MAX_PLAN_ID_CHARS: usize = 300;
pub const MAX_PLAN_MATERIAL_IDS: usize = 50;
/// Upper bound for a whole plan serialised as JSON (1 MiB). A realistic semester plan is
/// a few dozen KB; this only stops abuse that the per-field limits alone would allow.
pub const MAX_PLAN_JSON_BYTES: usize = 1024 * 1024;
/// `save_study_plan` keeps only this many plans (the newest); older ones are deleted.
pub const MAX_STORED_STUDY_PLANS: u32 = 50;

// Column lists shared by the SELECTs below and the matching `*_from_row` functions.
const SOURCE_COLUMNS: &str =
    "id, kind, label, config_json, last_synced_at, last_error, last_error_kind";
/// Term dates are the effective ones: the user override if set, else the synced value.
const COURSE_COLUMNS: &str = "id, source_id, external_id, code, name, \
     COALESCE(user_term_start, term_start) AS term_start, \
     COALESCE(user_term_end, term_end) AS term_end, \
     CASE WHEN user_term_start IS NOT NULL OR user_term_end IS NOT NULL THEN 'user' \
          WHEN term_start IS NOT NULL OR term_end IS NOT NULL THEN 'synced' \
          ELSE 'none' END AS term_source, \
     url, ai_policy, ai_policy_note, ai_access, material_sharing, hidden, enrollment_active, \
     updated_at";
const TERM_DATA_COLUMNS: &str = "lms_term_name, lms_term_start, lms_term_end, \
     lms_course_start, lms_course_end, lms_time_zone, lms_concluded, lms_workflow_state, \
     lms_access_restricted, keep_current_until, removal_snoozed_until, term_start, term_end, \
     user_term_start, user_term_end, institution";
const MODULE_COLUMNS: &str = "id, course_id, name, position, unlock_at, week_hint";
const MATERIAL_COLUMNS: &str = "id, course_id, module_id, kind, title, url, local_path, mime, \
     published_at, week_hint, content_hash, text_status, text_error, text_error_kind, \
     text_error_fingerprint, download_blocked, updated_at";
const CHUNK_COLUMNS: &str = "material_id, ord, locator, text";
const EVENT_COLUMNS: &str = "id, source_id, course_id, kind, title, starts_at, ends_at, due_at, url, \
                             updated_at, course_hint";

pub struct Store {
    conn: Connection,
}

impl Store {
    // ----- opening -----------------------------------------------------------------------

    /// Open read-write, creating the file if needed. Sets `journal_mode=WAL`,
    /// `foreign_keys=ON`, `busy_timeout=5000`, then migrates to `SCHEMA_VERSION`.
    /// Errors: `SchemaTooNew` if the file was written by a newer PageLamp.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        // The database holds extracted course text: private to the user even in a shared
        // folder (SQLite gives its -wal/-shm files the database's mode).
        #[cfg(unix)]
        for suffix in ["", "-wal", "-shm"] {
            let mut file = path.as_os_str().to_owned();
            file.push(suffix);
            let file = std::path::PathBuf::from(file);
            if file.exists()
                && let Err(err) = crate::paths::restrict(&file, 0o600)
            {
                tracing::warn!("could not make the database private: {err}");
            }
        }
        conn.busy_timeout(BUSY_TIMEOUT)?;
        // `PRAGMA journal_mode` answers with the resulting mode, so it is read back as a row.
        // (On a file system without WAL support it stays e.g. "delete"; that still works.)
        let _mode: String =
            conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))?;
        // NORMAL is durable enough in WAL mode and avoids an fsync per commit.
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", true)?;
        let store = Store { conn };
        store.migrate()?;
        Ok(store)
    }

    /// Open read-only (`SQLITE_OPEN_READ_ONLY`), `busy_timeout=5000`, `query_only=ON`.
    /// Errors: `NotInitialised` if the file is missing or `user_version == 0`;
    /// `SchemaTooOld` for an older, non-zero version (it needs a read-write `open` to migrate
    /// first); `SchemaTooNew` if `user_version > SCHEMA_VERSION`, unless the newer database
    /// says this version can still read it (**reader rule**: `schema_meta.min_reader_version`
    /// ≤ `SCHEMA_VERSION`; newer migrations are additive, so a running MCP process of the
    /// previous version keeps working after an update).
    ///
    /// Never creates the database file. (SQLite may create the `-wal`/`-shm` side files
    /// next to it, which is how WAL readers coordinate with the writer.)
    pub fn open_read_only(path: &Path) -> Result<Self> {
        if !path.is_file() {
            return Err(Error::NotInitialised(path.display().to_string()));
        }
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(path, flags)?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        conn.pragma_update(None, "query_only", true)?;
        let store = Store { conn };
        match store.user_version()? {
            0 => Err(Error::NotInitialised(path.display().to_string())),
            found if found < SCHEMA_VERSION => Err(Error::SchemaTooOld {
                found,
                supported: SCHEMA_VERSION,
            }),
            found => {
                store.check_readable(found)?;
                Ok(store)
            }
        }
    }

    /// Migrate an existing database written by an older version (0 < `user_version` <
    /// `SCHEMA_VERSION`) with one read-write `open`; returns the version it had. A missing
    /// file, an uninitialised (0) or current database is left alone (`None`), and so is a newer
    /// one this version may read (the reader rule of `open_read_only`); any other newer one is
    /// `SchemaTooNew`. Used by read-only processes (the MCP server) at startup.
    pub fn upgrade_existing(path: &Path) -> Result<Option<i64>> {
        if !path.is_file() {
            return Ok(None);
        }
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(path, flags)?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        let store = Store { conn };
        let version = store.user_version()?;
        if version == 0 || version == SCHEMA_VERSION {
            return Ok(None);
        }
        if version > SCHEMA_VERSION {
            store.check_readable(version)?;
            return Ok(None);
        }
        drop(store);
        Store::open(path)?;
        Ok(Some(version))
    }

    /// Fresh in-memory DB with the schema applied (tests).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", true)?;
        let store = Store { conn };
        store.migrate()?;
        Ok(store)
    }

    /// Bring the schema up to `SCHEMA_VERSION` by applying the missing `MIGRATIONS`.
    fn migrate(&self) -> Result<()> {
        // Fast path that takes no write lock: already up to date (or too new → error).
        let current = self.checked_user_version()?;
        if current == SCHEMA_VERSION {
            return Ok(());
        }
        // Outside the transaction (VACUUM can't run inside one); a copy made while another
        // process migrates is detected and dropped (`write_backup`).
        let backup = (current > 0)
            .then(|| self.back_up_before_migration(current))
            .flatten();
        self.in_transaction(|store| {
            // Read again under the write lock: another process may have migrated meanwhile.
            let current = store.checked_user_version()?;
            // `checked_user_version` guarantees 0 <= current <= MIGRATIONS.len().
            for (index, migration) in MIGRATIONS.iter().enumerate().skip(current as usize) {
                store.conn.execute_batch(migration)?;
                // Data steps that need Rust, right after their schema step.
                if index + 1 == 4 {
                    store.migrate_legacy_calendars()?;
                }
            }
            // After any upgrade: databases written before chunks were tied to `ok` (v0.1) may
            // still hold the text of files locked or moved since.
            store.drop_stale_chunks()?;
            // Part of the transaction: rolled back together with the schema on failure.
            store
                .conn
                .pragma_update(None, "user_version", SCHEMA_VERSION)?;
            Ok(())
        })?;
        // Visible later in `doctor` and reports; losing the record is not worth failing for.
        if let Some(outcome) = backup {
            let record = MigrationBackupRecord {
                from_version: current,
                to_version: SCHEMA_VERSION,
                at: Utc::now().trunc_subsecs(0),
                outcome,
            };
            if let Err(err) = self.set_setting(LAST_MIGRATION_BACKUP, &record) {
                tracing::warn!("could not record the pre-migration backup: {err}");
            }
        }
        Ok(())
    }

    /// `PRAGMA user_version`, rejecting versions this binary cannot handle.
    fn checked_user_version(&self) -> Result<i64> {
        let version = self.user_version()?;
        if version > SCHEMA_VERSION {
            return Err(Error::SchemaTooNew {
                found: version,
                supported: SCHEMA_VERSION,
            });
        }
        Ok(version)
    }

    /// `PRAGMA user_version` as stored (`Invalid` if negative).
    fn user_version(&self) -> Result<i64> {
        let version: i64 = self
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version < 0 {
            return Err(Error::Invalid(format!(
                "database has an invalid schema version ({version})"
            )));
        }
        Ok(version)
    }

    /// The reader rule: a database at `version` (newer than `SCHEMA_VERSION`) may be read if its
    /// `schema_meta.min_reader_version` is at most `SCHEMA_VERSION`; otherwise `SchemaTooNew`.
    fn check_readable(&self, version: i64) -> Result<()> {
        if version <= SCHEMA_VERSION {
            return Ok(());
        }
        let too_new = || Error::SchemaTooNew {
            found: version,
            supported: SCHEMA_VERSION,
        };
        let min_reader: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM schema_meta WHERE key = 'min_reader_version'",
                [],
                |row| row.get(0),
            )
            .optional()
            .unwrap_or(None); // no schema_meta table: a database no reader rule applies to
        match min_reader.and_then(|v| v.trim().parse::<i64>().ok()) {
            Some(min) if min <= SCHEMA_VERSION => Ok(()),
            _ => Err(too_new()),
        }
    }

    /// `schema_meta.min_reader_version`, if the database has one (schema 3 and later).
    pub fn min_reader_version(&self) -> Result<Option<i64>> {
        let value: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM schema_meta WHERE key = 'min_reader_version'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value.and_then(|v| v.trim().parse().ok()))
    }

    /// Copy the database before migrating it from `version` (see `write_backup`). Best effort:
    /// a failure is logged, and the migration (additive, in one transaction) goes ahead.
    /// `None` for an in-memory database.
    fn back_up_before_migration(&self, version: i64) -> Option<MigrationBackupOutcome> {
        let db = self
            .conn
            .path()
            .filter(|p| !p.is_empty())
            .map(std::path::PathBuf::from)?;
        Some(match write_backup(&self.conn, &db, version) {
            Ok(Some(backup)) => {
                tracing::info!(
                    "backed up the database (schema {version}) before updating it: {}",
                    backup.file_name().unwrap_or_default().to_string_lossy()
                );
                MigrationBackupOutcome::Ok
            }
            Ok(None) => MigrationBackupOutcome::Skipped,
            Err(err) => {
                tracing::warn!("could not back up the database before updating it: {err}");
                MigrationBackupOutcome::Failed {
                    code: backup_error_code(&err),
                }
            }
        })
    }

    /// The last migration's backup outcome, if this database was ever migrated (schema 3+).
    pub fn last_migration_backup(&self) -> Result<Option<MigrationBackupRecord>> {
        // Unreadable (another version's shape): as if absent; a failed read is an error.
        self.setting_or_absent(LAST_MIGRATION_BACKUP)
    }

    /// Run `f` inside `BEGIN IMMEDIATE … COMMIT` on this connection (ROLLBACK on error).
    /// All `Store` methods used inside `f` share the transaction.
    ///
    /// Must not be nested: calling `in_transaction` again inside `f` (directly, or through a
    /// function that opens its own transaction such as `ingest::index_file`) fails with an
    /// `Error::Db` that says so; nothing is nested or committed early. This is deliberate:
    /// callers such as ingest keep slow work (text extraction) outside the write lock, and a
    /// silently nested transaction would hold the lock during that work. The `Store` methods
    /// themselves never open a transaction, so they can all be used inside `f`.
    /// If `f` panics, the transaction is rolled back while the panic unwinds.
    pub fn in_transaction<T>(&self, f: impl FnOnce(&Store) -> Result<T>) -> Result<T> {
        if !self.conn.is_autocommit() {
            return Err(nested_transaction_error());
        }
        // A `Transaction` that is dropped without `commit` rolls back: that covers the early
        // `?` return below as well as unwinding from a panic in `f`.
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let value = f(self)?;
        tx.commit()?;
        Ok(value)
    }

    /// Run `f` in one read transaction: every read inside sees the same snapshot of the
    /// database (WAL), so a check and the data it guards can't disagree (the AI policy gate
    /// reads a course's state and its text this way). Takes no write lock. Must not be nested
    /// or wrap writes.
    pub fn in_read_transaction<T>(&self, f: impl FnOnce(&Store) -> Result<T>) -> Result<T> {
        if !self.conn.is_autocommit() {
            return Err(nested_transaction_error());
        }
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Deferred)?;
        let value = f(self)?;
        tx.commit()?;
        Ok(value)
    }

    /// Run `f` inside a SAVEPOINT: its statements take effect all together or not at all.
    /// Savepoints nest, so this works on its own and inside `in_transaction` alike.
    ///
    /// (Outside a transaction a SAVEPOINT starts a deferred one. Every `f` used here starts
    /// with a write, so the write lock is requested up front and `busy_timeout` applies.)
    fn atomic<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        self.conn.execute_batch("SAVEPOINT store_atomic")?;
        let savepoint = SavepointGuard {
            conn: &self.conn,
            released: false,
        };
        let value = f()?;
        savepoint.release()?;
        Ok(value)
    }

    // ----- sources -----------------------------------------------------------------------

    /// Insert or update id/kind/label/config (does not touch last_synced_at/last_error/
    /// last_error_kind).
    pub fn upsert_source(&self, source: &SourceRecord) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sources (id, kind, label, config_json) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
                 kind = excluded.kind,
                 label = excluded.label,
                 config_json = excluded.config_json",
            params![
                source.id,
                source.kind.as_str(),
                source.label,
                serde_json::to_string(&source.config)?,
            ],
        )?;
        Ok(())
    }

    pub fn get_source(&self, id: &str) -> Result<Option<SourceRecord>> {
        self.query_opt(
            &format!("SELECT {SOURCE_COLUMNS} FROM sources WHERE id = ?1"),
            [id],
            source_from_row,
        )
    }

    /// Ordered by label, then id.
    pub fn list_sources(&self) -> Result<Vec<SourceRecord>> {
        self.query_list(
            &format!("SELECT {SOURCE_COLUMNS} FROM sources ORDER BY label, id"),
            [],
            source_from_row,
        )
    }

    /// Deletes the source and (via cascade) its courses, materials, chunks, events.
    /// Errors: `NotFound` if there is no such source.
    pub fn remove_source(&self, id: &str) -> Result<()> {
        let deleted = self
            .conn
            .execute("DELETE FROM sources WHERE id = ?1", [id])?;
        expect_changed(deleted, "source", id)
    }

    /// Record a sync attempt: `error = None` → success (sets last_synced_at, clears last_error
    /// and last_error_kind); `Some((kind, msg))` → failure (sets last_error + last_error_kind,
    /// leaves last_synced_at unchanged). Errors: `NotFound` if there is no such source.
    pub fn record_sync(
        &self,
        id: &str,
        at: Timestamp,
        error: Option<(SourceErrorKind, &str)>,
    ) -> Result<()> {
        let changed = match error {
            None => self.conn.execute(
                "UPDATE sources
                 SET last_synced_at = ?2, last_error = NULL, last_error_kind = NULL
                 WHERE id = ?1",
                params![id, ts_text(at)],
            )?,
            Some((kind, message)) => self.conn.execute(
                "UPDATE sources SET last_error = ?2, last_error_kind = ?3 WHERE id = ?1",
                params![id, message, kind.as_str()],
            )?,
        };
        expect_changed(changed, "source", id)
    }

    /// Clear last_error/last_error_kind without touching last_synced_at (used after the user
    /// replaced an expired token / feed URL). Errors: `NotFound` if there is no such source.
    pub fn clear_source_error(&self, id: &str) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE sources SET last_error = NULL, last_error_kind = NULL WHERE id = ?1",
            [id],
        )?;
        expect_changed(changed, "source", id)
    }

    // ----- courses -----------------------------------------------------------------------

    /// Insert or update synced fields only; never touches ai_policy, ai_policy_note, ai_access,
    /// user_term_*, hidden. Sets updated_at = now.
    pub fn upsert_course(&self, course: &CourseUpsert) -> Result<()> {
        let lms = &course.lms;
        self.conn.execute(
            "INSERT INTO courses (id, source_id, external_id, code, name, term_start, term_end,
                                  url, syllabus_text, updated_at,
                                  lms_term_name, lms_term_start, lms_term_end, lms_course_start,
                                  lms_course_end, lms_time_zone, lms_concluded, lms_workflow_state,
                                  lms_access_restricted)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                     ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)
             ON CONFLICT(id) DO UPDATE SET
                 source_id = excluded.source_id,
                 external_id = excluded.external_id,
                 code = excluded.code,
                 name = excluded.name,
                 term_start = excluded.term_start,
                 term_end = excluded.term_end,
                 url = excluded.url,
                 syllabus_text = excluded.syllabus_text,
                 updated_at = excluded.updated_at,
                 lms_term_name = excluded.lms_term_name,
                 lms_term_start = excluded.lms_term_start,
                 lms_term_end = excluded.lms_term_end,
                 lms_course_start = excluded.lms_course_start,
                 lms_course_end = excluded.lms_course_end,
                 lms_time_zone = excluded.lms_time_zone,
                 lms_concluded = excluded.lms_concluded,
                 lms_workflow_state = excluded.lms_workflow_state,
                 lms_access_restricted = excluded.lms_access_restricted",
            params![
                course.id,
                course.source_id,
                course.external_id,
                course.code,
                course.name,
                opt_date_text(course.term_start),
                opt_date_text(course.term_end),
                course.url,
                course.syllabus_text,
                now_text(),
                lms.term_name,
                opt_date_text(lms.term_start),
                opt_date_text(lms.term_end),
                opt_date_text(lms.course_start),
                opt_date_text(lms.course_end),
                lms.time_zone,
                lms.concluded,
                lms.workflow_state,
                lms.access_restricted,
            ],
        )?;
        Ok(())
    }

    /// Mark a listed course the LMS restricts by date (`access_restricted_by_date`), which the
    /// sync does not upsert: only `lms_access_restricted` changes. `NotFound` for an unknown id.
    pub fn set_course_access_restricted(&self, course_id: &str, restricted: bool) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE courses SET lms_access_restricted = ?2 WHERE id = ?1",
            params![course_id, restricted],
        )?;
        expect_changed(changed, "course", course_id)
    }

    /// The LMS facts and the student's lifecycle answers of one course (hidden or not), or
    /// `None` for an unknown id.
    pub fn course_term_data(&self, course_id: &str) -> Result<Option<CourseTermData>> {
        self.query_opt(
            &format!("SELECT {TERM_DATA_COLUMNS} FROM courses WHERE id = ?1"),
            [course_id],
            term_data_from_row,
        )
    }

    /// `course_term_data` of every course (hidden ones too), by course id.
    pub fn all_course_term_data(
        &self,
    ) -> Result<std::collections::BTreeMap<String, CourseTermData>> {
        Ok(self
            .query_list(
                &format!("SELECT id, {TERM_DATA_COLUMNS} FROM courses"),
                [],
                |row| Ok((row.get::<_, String>("id")?, term_data_from_row(row)?)),
            )?
            .into_iter()
            .collect())
    }

    /// "I'm still taking this": keep the course current until `until` (`None` clears it).
    /// A student answer: sync never changes it. `NotFound` for an unknown id.
    pub fn set_keep_current_until(&self, course_id: &str, until: Option<NaiveDate>) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE courses SET keep_current_until = ?2 WHERE id = ?1",
            params![course_id, opt_date_text(until)],
        )?;
        expect_changed(changed, "course", course_id)
    }

    /// Snooze the removal suggestion until `until` ("Keep" = 9999-12-31; `None` clears it).
    /// A student answer: sync never changes it. `NotFound` for an unknown id.
    pub fn set_removal_snoozed_until(
        &self,
        course_id: &str,
        until: Option<NaiveDate>,
    ) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE courses SET removal_snoozed_until = ?2 WHERE id = ?1",
            params![course_id, opt_date_text(until)],
        )?;
        expect_changed(changed, "course", course_id)
    }

    /// All courses (including hidden ones when `include_hidden`), ordered by code, name.
    /// Removed courses (with a tombstone) are never listed: they show under "Removed courses".
    pub fn list_courses(&self, include_hidden: bool) -> Result<Vec<Course>> {
        self.query_list(
            &format!(
                "SELECT {COURSE_COLUMNS} FROM courses
                 WHERE (hidden = 0 OR ?1)
                   AND id NOT IN (SELECT course_id FROM course_tombstones)
                 ORDER BY code, name, id"
            ),
            [include_hidden],
            course_from_row,
        )
    }

    /// Every course row, removed ones too (whose local data may still be there): for what
    /// decides which downloaded folders are still in use.
    pub fn list_all_courses(&self) -> Result<Vec<Course>> {
        self.query_list(
            &format!("SELECT {COURSE_COLUMNS} FROM courses ORDER BY code, name, id"),
            [],
            course_from_row,
        )
    }

    /// Any course by id, hidden or not.
    pub fn get_course(&self, id: &str) -> Result<Option<Course>> {
        self.query_opt(
            &format!("SELECT {COURSE_COLUMNS} FROM courses WHERE id = ?1"),
            [id],
            course_from_row,
        )
    }

    /// Resolve a user/AI-supplied course reference. Tries, in order: exact id; exact display
    /// name ("DEMO101 — Intro…", ASCII case-insensitive); exact code
    /// (case-insensitive, ignoring spaces); unique code prefix ("demo101" → "DEMO101H1");
    /// unique case-insensitive substring of name. Hidden courses are excluded.
    /// Errors: `NotFound` (message lists available codes) or `Ambiguous`.
    pub fn resolve_course(&self, query: &str) -> Result<Course> {
        self.resolve_course_with(query, false)
    }

    /// Like `resolve_course`, optionally including hidden courses (course-settings commands
    /// must be able to address a hidden course, e.g. to un-hide it).
    ///
    /// The first rule with at least one match decides: one match → that course, several →
    /// `Ambiguous` (candidates are listed as "CODE — Name (id: …)" so the caller can retry
    /// with an exact id). The query is trimmed first; an empty query is `NotFound`.
    pub fn resolve_course_with(&self, query: &str, include_hidden: bool) -> Result<Course> {
        let courses = self.list_courses(include_hidden)?;
        let query = query.trim();
        if query.is_empty() {
            return Err(course_not_found(query, &courses));
        }

        // 1. Exact id, or exact display name ("DEMO101 — Intro…", as shown everywhere).
        if let Some(course) = courses.iter().find(|c| c.id == query) {
            return Ok(course.clone());
        }
        let by_display: Vec<&Course> = courses
            .iter()
            .filter(|c| c.display_name().eq_ignore_ascii_case(query))
            .collect();
        if let Some(result) = pick_course(query, by_display) {
            return result;
        }

        // 2. Exact code, 3. code prefix — both case-insensitive and ignoring spaces.
        let wanted_code = normalise_code(query);
        let code_of = |c: &Course| c.code.as_deref().map(normalise_code);
        let exact_code: Vec<&Course> = courses
            .iter()
            .filter(|c| code_of(c).is_some_and(|code| code == wanted_code))
            .collect();
        if let Some(result) = pick_course(query, exact_code) {
            return result;
        }
        let code_prefix: Vec<&Course> = courses
            .iter()
            .filter(|c| code_of(c).is_some_and(|code| code.starts_with(&wanted_code)))
            .collect();
        if let Some(result) = pick_course(query, code_prefix) {
            return result;
        }

        // 4. Case-insensitive substring of the name.
        let wanted_name = query.to_lowercase();
        let by_name: Vec<&Course> = courses
            .iter()
            .filter(|c| c.name.to_lowercase().contains(&wanted_name))
            .collect();
        if let Some(result) = pick_course(query, by_name) {
            return result;
        }

        Err(course_not_found(query, &courses))
    }

    /// Record which courses of `source_id` the LMS still lists as active: those in `active`
    /// get `enrollment_active = 1`, the others 0. Nothing is deleted — a course that ended is
    /// exactly what a student needs during exams (v0.2 can offer "Remove old courses").
    pub fn mark_enrollment_active(&self, source_id: &str, active: &[String]) -> Result<()> {
        let active = serde_json::to_string(active)?;
        self.conn.execute(
            "UPDATE courses SET enrollment_active =
                 CASE WHEN id IN (SELECT value FROM json_each(?2)) THEN 1 ELSE 0 END
             WHERE source_id = ?1",
            params![source_id, active],
        )?;
        Ok(())
    }
    /// Delete courses of `source_id` whose id is not in `keep` (course dropped/ended).
    /// Returns the number of deleted courses (their modules/materials/chunks cascade).
    pub fn prune_courses(&self, source_id: &str, keep: &[String]) -> Result<usize> {
        // `keep` is bound as ONE JSON array and expanded by `json_each` — no SQL is built
        // from ids, and any number of ids fits in a single parameter.
        let keep_json = serde_json::to_string(keep)?;
        self.atomic(|| {
            Ok(self.conn.execute(
                "DELETE FROM courses
                 WHERE source_id = ?1 AND id NOT IN (SELECT value FROM json_each(?2))",
                params![source_id, keep_json],
            )?)
        })
    }

    /// `note = None` clears the note. Errors: `NotFound` if there is no such course.
    pub fn set_course_policy(
        &self,
        course_id: &str,
        policy: AiPolicy,
        note: Option<&str>,
    ) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE courses SET ai_policy = ?2, ai_policy_note = ?3 WHERE id = ?1",
            params![course_id, policy.as_str(), note],
        )?;
        expect_changed(changed, "course", course_id)
    }

    /// Set/clear user term overrides (`None` = use the synced date again).
    /// Errors: `Invalid` if both are given and start > end; `NotFound` for an unknown course.
    pub fn set_course_term(
        &self,
        course_id: &str,
        start: Option<NaiveDate>,
        end: Option<NaiveDate>,
    ) -> Result<()> {
        if let (Some(start), Some(end)) = (start, end)
            && start > end
        {
            return Err(Error::Invalid(format!(
                "term start {start} is after term end {end}"
            )));
        }
        let changed = self.conn.execute(
            "UPDATE courses SET user_term_start = ?2, user_term_end = ?3 WHERE id = ?1",
            params![course_id, opt_date_text(start), opt_date_text(end)],
        )?;
        expect_changed(changed, "course", course_id)
    }

    /// Errors: `NotFound` if there is no such course.
    /// The student's per-course "AI may read this course's materials" switch. Keeps its value
    /// while a `prohibited` policy withholds the text (see `Course::ai_materials`).
    pub fn set_course_ai_access(&self, course_id: &str, allowed: bool) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE courses SET ai_access = ?2 WHERE id = ?1",
            params![course_id, allowed],
        )?;
        expect_changed(changed, "course", course_id)
    }
    pub fn set_course_hidden(&self, course_id: &str, hidden: bool) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE courses SET hidden = ?2 WHERE id = ?1",
            params![course_id, hidden],
        )?;
        expect_changed(changed, "course", course_id)
    }

    /// `None` when the course has no syllabus text (or does not exist).
    pub fn course_syllabus_text(&self, course_id: &str) -> Result<Option<String>> {
        let text: Option<Option<String>> = self.query_opt(
            "SELECT syllabus_text FROM courses WHERE id = ?1",
            [course_id],
            |row| row.get(0),
        )?;
        Ok(text.flatten())
    }

    // ----- modules -----------------------------------------------------------------------

    /// Replace all modules of a course (delete + insert). Materials referencing removed
    /// modules get module_id = NULL via FK.
    ///
    /// Implemented as "delete the course's modules that are not in `modules`, upsert the
    /// rest", so materials of modules that still exist keep their module link.
    /// Errors: `Invalid` if a module's `course_id` is not `course_id`.
    pub fn replace_modules(&self, course_id: &str, modules: &[Module]) -> Result<()> {
        if let Some(module) = modules.iter().find(|m| m.course_id != course_id) {
            return Err(Error::Invalid(format!(
                "module '{}' belongs to course '{}', not '{course_id}'",
                module.id, module.course_id
            )));
        }
        let keep: Vec<&str> = modules.iter().map(|m| m.id.as_str()).collect();
        let keep_json = serde_json::to_string(&keep)?;
        self.atomic(|| {
            self.conn.execute(
                "DELETE FROM modules
                 WHERE course_id = ?1 AND id NOT IN (SELECT value FROM json_each(?2))",
                params![course_id, keep_json],
            )?;
            let mut upsert = self.conn.prepare_cached(
                "INSERT INTO modules (id, course_id, name, position, unlock_at, week_hint)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(id) DO UPDATE SET
                     course_id = excluded.course_id,
                     name = excluded.name,
                     position = excluded.position,
                     unlock_at = excluded.unlock_at,
                     week_hint = excluded.week_hint",
            )?;
            for module in modules {
                upsert.execute(params![
                    module.id,
                    course_id,
                    module.name,
                    module.position,
                    opt_ts_text(module.unlock_at),
                    module.week_hint,
                ])?;
            }
            Ok(())
        })
    }

    /// Ordered by position (NULLs last), then name.
    pub fn list_modules(&self, course_id: &str) -> Result<Vec<Module>> {
        self.query_list(
            &format!(
                "SELECT {MODULE_COLUMNS} FROM modules WHERE course_id = ?1
                 ORDER BY position IS NULL, position, name, id"
            ),
            [course_id],
            module_from_row,
        )
    }

    // ----- materials & chunks ------------------------------------------------------------

    /// Insert (text_status = pending) or update the synced columns of a material.
    /// On update, content_hash/text_status/text_error(_kind) are preserved — EXCEPT when
    /// `local_path` changed, in which case text_status is reset to pending (and the now
    /// stale text_error, text_error_kind and fingerprint are cleared) and the old chunks go
    /// (chunks exist only for `ok` materials: `set_text_state`). Sets updated_at = now.
    pub fn upsert_material(&self, material: &MaterialUpsert) -> Result<()> {
        self.atomic(|| {
            self.upsert_material_row(material)?;
            self.conn.execute(
                "DELETE FROM chunks WHERE material_id = ?1
                   AND NOT EXISTS (SELECT 1 FROM materials WHERE id = ?1 AND text_status = 'ok')",
                [&material.id],
            )?;
            Ok(())
        })
    }

    fn upsert_material_row(&self, material: &MaterialUpsert) -> Result<()> {
        // In the DO UPDATE clause `materials.x` is the stored (old) value and `excluded.x`
        // the new one; `IS` compares NULLs as equal.
        self.conn.execute(
            "INSERT INTO materials (id, course_id, module_id, kind, title, url, local_path, mime,
                                    published_at, week_hint, text_status, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(id) DO UPDATE SET
                 course_id = excluded.course_id,
                 module_id = excluded.module_id,
                 kind = excluded.kind,
                 title = excluded.title,
                 url = excluded.url,
                 mime = excluded.mime,
                 published_at = excluded.published_at,
                 week_hint = excluded.week_hint,
                 text_status = CASE WHEN materials.local_path IS excluded.local_path
                                    THEN materials.text_status ELSE excluded.text_status END,
                 text_error = CASE WHEN materials.local_path IS excluded.local_path
                                   THEN materials.text_error ELSE NULL END,
                 text_error_kind = CASE WHEN materials.local_path IS excluded.local_path
                                        THEN materials.text_error_kind ELSE NULL END,
                 text_error_fingerprint = CASE WHEN materials.local_path IS excluded.local_path
                                          THEN materials.text_error_fingerprint ELSE NULL END,
                 local_path = excluded.local_path,
                 updated_at = excluded.updated_at",
            params![
                material.id,
                material.course_id,
                material.module_id,
                material.kind.as_str(),
                material.title,
                material.url,
                material.local_path,
                material.mime,
                opt_ts_text(material.published_at),
                material.week_hint,
                TextStatus::Pending.as_str(),
                now_text(),
            ],
        )?;
        Ok(())
    }

    pub fn get_material(&self, id: &str) -> Result<Option<Material>> {
        self.query_opt(
            &format!("SELECT {MATERIAL_COLUMNS} FROM materials WHERE id = ?1"),
            [id],
            material_from_row,
        )
    }

    /// Ordered by week_hint (NULLs last), published_at, title.
    pub fn list_materials(&self, course_id: &str) -> Result<Vec<Material>> {
        self.query_list(
            &format!(
                "SELECT {MATERIAL_COLUMNS} FROM materials WHERE course_id = ?1
                 ORDER BY week_hint IS NULL, week_hint, published_at, title, id"
            ),
            [course_id],
            material_from_row,
        )
    }

    /// Delete materials of the course whose id is not in `keep` (chunks cascade).
    /// Returns the number of deleted materials.
    pub fn prune_materials(&self, course_id: &str, keep: &[String]) -> Result<usize> {
        let keep_json = serde_json::to_string(keep)?;
        self.atomic(|| {
            Ok(self.conn.execute(
                "DELETE FROM materials
                 WHERE course_id = ?1 AND id NOT IN (SELECT value FROM json_each(?2))",
                params![course_id, keep_json],
            )?)
        })
    }

    /// Update index state of one material. `content_hash = None` leaves the stored hash as is.
    /// Clears the worker failure (`set_text_error_kind` records one after this call).
    /// Any status but `ok` also deletes the material's chunks: chunks exist only for `ok`
    /// materials, so no reader (MCP, search, a model's context) can see the text of a file
    /// that was locked, moved or failed since. Errors: `NotFound` if there is no such material.
    pub fn set_text_state(
        &self,
        material_id: &str,
        status: TextStatus,
        error: Option<&str>,
        content_hash: Option<&str>,
    ) -> Result<()> {
        self.atomic(|| {
            let changed = self.conn.execute(
                "UPDATE materials
                 SET text_status = ?2, text_error = ?3, content_hash = COALESCE(?4, content_hash),
                     text_error_kind = NULL, text_error_fingerprint = NULL
                 WHERE id = ?1",
                params![material_id, status.as_str(), error, content_hash],
            )?;
            expect_changed(changed, "material", material_id)?;
            if status != TextStatus::Ok {
                self.conn
                    .execute("DELETE FROM chunks WHERE material_id = ?1", [material_id])?;
            }
            Ok(())
        })
    }

    /// Deletes the chunks of every material that isn't `ok` (the `set_text_state` invariant),
    /// for databases written before it held. Idempotent. Returns how many chunks went.
    pub(crate) fn drop_stale_chunks(&self) -> Result<usize> {
        Ok(self.conn.execute(
            "DELETE FROM chunks WHERE material_id IN
                 (SELECT id FROM materials WHERE text_status <> 'ok')",
            [],
        )?)
    }

    /// Record why the extraction worker failed on the material's current content, and the
    /// worker protocol and app version that failed (`ingest::failure_fingerprint`).
    /// Errors: `NotFound` if there is no such material.
    pub fn set_text_error_kind(
        &self,
        material_id: &str,
        kind: TextErrorKind,
        fingerprint: &str,
    ) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE materials SET text_error_kind = ?2, text_error_fingerprint = ?3 WHERE id = ?1",
            params![material_id, kind.as_str(), fingerprint],
        )?;
        expect_changed(changed, "material", material_id)
    }

    /// How many materials the extraction worker could not read, per failure kind (kinds
    /// without files are left out), in `TextErrorKind` order.
    pub fn unreadable_file_counts(&self) -> Result<Vec<(TextErrorKind, u32)>> {
        let mut statement = self.conn.prepare(
            "SELECT text_error_kind, COUNT(*) FROM materials
             WHERE text_status = 'error' AND text_error_kind IS NOT NULL
             GROUP BY text_error_kind",
        )?;
        let mut counts = statement
            .query_map([], |row| {
                Ok((get_value(row, "text_error_kind")?, row.get::<_, u32>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<(TextErrorKind, u32)>>>()?;
        counts.sort();
        Ok(counts)
    }

    /// The outline a folder's `course.toml` names (`outline = "…"`): that material is flagged,
    /// the course's others not (`None`: none named).
    /// The school a folder course names in `course.toml` (`institution`; sync-written, never by a
    /// setter the student uses): it opts the course into the session hint and the school's
    /// calendar (calendar design §6.3, §6.4).
    pub fn set_course_institution(&self, course_id: &str, institution: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE courses SET institution = ?2 WHERE id = ?1",
            params![course_id, institution],
        )?;
        Ok(())
    }

    pub fn set_named_outline(&self, course_id: &str, material_id: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE materials SET named_outline = (id IS ?2) WHERE course_id = ?1",
            params![course_id, material_id],
        )?;
        Ok(())
    }

    /// Record why a file cannot be downloaded on request (`None` clears it).
    pub fn set_download_blocked(
        &self,
        material_id: &str,
        blocked: Option<DownloadBlock>,
    ) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE materials SET download_blocked = ?2 WHERE id = ?1",
            params![material_id, blocked.map(DownloadBlock::as_str)],
        )?;
        expect_changed(changed, "material", material_id)
    }

    /// Replace all chunks of a material (FTS kept in sync by triggers). Chunks belong only to
    /// an `ok` material (`set_text_state`): set its text state first, as the ingest does.
    /// Errors: `Invalid` if a chunk's `material_id` is not `material_id`, or if `chunks` isn't
    /// empty and the material isn't `ok`; `NotFound` if there is no such material.
    pub fn replace_chunks(&self, material_id: &str, chunks: &[Chunk]) -> Result<()> {
        if let Some(chunk) = chunks.iter().find(|c| c.material_id != material_id) {
            return Err(Error::Invalid(format!(
                "chunk {} belongs to material '{}', not '{material_id}'",
                chunk.ord, chunk.material_id
            )));
        }
        if !chunks.is_empty() {
            let status: Option<String> = self.query_opt(
                "SELECT text_status FROM materials WHERE id = ?1",
                [material_id],
                |row| row.get(0),
            )?;
            match status.as_deref() {
                Some("ok") => {}
                Some(status) => {
                    return Err(Error::Invalid(format!(
                        "material '{material_id}' is {status}, not ok: set its text state first"
                    )));
                }
                None => return Err(Error::NotFound(format!("material '{material_id}'"))),
            }
        }
        self.atomic(|| {
            self.conn
                .execute("DELETE FROM chunks WHERE material_id = ?1", [material_id])?;
            let mut insert = self.conn.prepare_cached(
                "INSERT INTO chunks (material_id, ord, locator, text) VALUES (?1, ?2, ?3, ?4)",
            )?;
            for chunk in chunks {
                insert.execute(params![material_id, chunk.ord, chunk.locator, chunk.text])?;
            }
            Ok(())
        })
    }

    /// Chunks ordered by ord, optionally a window [from_ord, from_ord+limit).
    /// (`limit = None` → every chunk from `from_ord` on.)
    pub fn get_chunks(
        &self,
        material_id: &str,
        from_ord: u32,
        limit: Option<u32>,
    ) -> Result<Vec<Chunk>> {
        self.query_list(
            &format!(
                "SELECT {CHUNK_COLUMNS} FROM chunks
                 WHERE material_id = ?1 AND ord >= ?2 AND (?3 IS NULL OR ord < ?2 + ?3)
                 ORDER BY ord"
            ),
            params![material_id, from_ord, limit],
            chunk_from_row,
        )
    }

    /// Number of chunks of every material of a course, in one query (materials without
    /// chunks map to 0).
    pub fn material_chunk_counts(
        &self,
        course_id: &str,
    ) -> Result<std::collections::HashMap<String, u32>> {
        let pairs = self.query_list(
            "SELECT m.id AS id, COUNT(ch.id) AS n
             FROM materials m LEFT JOIN chunks ch ON ch.material_id = m.id
             WHERE m.course_id = ?1
             GROUP BY m.id",
            [course_id],
            |row| Ok((row.get::<_, String>("id")?, row.get::<_, u32>("n")?)),
        )?;
        Ok(pairs.into_iter().collect())
    }
    pub fn chunk_count(&self, material_id: &str) -> Result<u32> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM chunks WHERE material_id = ?1",
            [material_id],
            |row| row.get(0),
        )?)
    }

    // ----- events --------------------------------------------------------------------------

    /// Upsert the given events of a source and delete that source's other events
    /// whose id is not in the list (the list is the source's complete current set).
    /// Each event's own `updated_at` is stored as given.
    /// Errors: `Invalid` if an event's `source_id` is not `source_id`.
    pub fn replace_events(&self, source_id: &str, events: &[Event]) -> Result<()> {
        if let Some(event) = events.iter().find(|e| e.source_id != source_id) {
            return Err(Error::Invalid(format!(
                "event '{}' belongs to source '{}', not '{source_id}'",
                event.id, event.source_id
            )));
        }
        let keep: Vec<&str> = events.iter().map(|e| e.id.as_str()).collect();
        let keep_json = serde_json::to_string(&keep)?;
        self.atomic(|| {
            let mut upsert = self.conn.prepare_cached(
                "INSERT INTO events (id, source_id, course_id, kind, title, starts_at, ends_at,
                                     due_at, url, updated_at, course_hint)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(id) DO UPDATE SET
                     source_id = excluded.source_id,
                     course_id = excluded.course_id,
                     kind = excluded.kind,
                     title = excluded.title,
                     starts_at = excluded.starts_at,
                     ends_at = excluded.ends_at,
                     due_at = excluded.due_at,
                     url = excluded.url,
                     updated_at = excluded.updated_at,
                     course_hint = excluded.course_hint",
            )?;
            for event in events {
                upsert.execute(params![
                    event.id,
                    source_id,
                    event.course_id,
                    event.kind.as_str(),
                    event.title,
                    opt_ts_text(event.starts_at),
                    opt_ts_text(event.ends_at),
                    opt_ts_text(event.due_at),
                    event.url,
                    ts_text(event.updated_at),
                    event.course_hint,
                ])?;
            }
            self.conn.execute(
                "DELETE FROM events
                 WHERE source_id = ?1 AND id NOT IN (SELECT value FROM json_each(?2))",
                params![source_id, keep_json],
            )?;
            Ok(())
        })
    }

    /// Events whose `when()` (due_at, else starts_at) falls in [from, to], optionally for one
    /// course, ordered by that instant. Events with neither date are excluded.
    /// (Instants are compared at whole-second precision, like everything stored. Bounds
    /// outside years 0000–9999 are clamped, so `DateTime::MIN_UTC`/`MAX_UTC` work as
    /// open ends.)
    pub fn list_events(
        &self,
        from: Timestamp,
        to: Timestamp,
        course_id: Option<&str>,
    ) -> Result<Vec<Event>> {
        // Stored instants all have the same fixed format, so TEXT comparison = time order.
        self.query_list(
            &format!(
                "SELECT {EVENT_COLUMNS} FROM events
                 WHERE COALESCE(due_at, starts_at) BETWEEN ?1 AND ?2
                   AND (?3 IS NULL OR course_id = ?3)
                 ORDER BY COALESCE(due_at, starts_at), title, id"
            ),
            params![ts_text(from), ts_text(to), course_id],
            event_from_row,
        )
    }

    /// Link events that have no course yet but carry a `course_hint` to the course that hint
    /// names (`model::course_for_hint`, over all courses including hidden ones). Run after a
    /// folder/Canvas sync so calendar events synced before their course existed get linked.
    /// Local only; returns how many events were linked.
    pub fn relink_events(&self) -> Result<usize> {
        let courses = self.list_courses(true)?;
        if courses.is_empty() {
            return Ok(0);
        }
        let pending: Vec<(String, String)> = self.query_list(
            "SELECT id, course_hint FROM events
             WHERE course_id IS NULL AND course_hint IS NOT NULL
             ORDER BY id",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let links: Vec<(String, &str)> = pending
            .into_iter()
            .filter_map(|(id, hint)| Some((id, course_for_hint(&hint, &courses)?.id.as_str())))
            .collect();
        if links.is_empty() {
            return Ok(0);
        }
        self.atomic(|| {
            let mut update = self
                .conn
                .prepare_cached("UPDATE events SET course_id = ?2 WHERE id = ?1")?;
            for (id, course_id) in &links {
                update.execute(params![id, course_id])?;
            }
            Ok(())
        })?;
        Ok(links.len())
    }

    // ----- search --------------------------------------------------------------------------

    /// Full-text search over chunks. `query` is free text from a user/AI: it must be
    /// sanitised into a safe FTS5 MATCH expression (quote each term, OR them; never pass
    /// raw user syntax to MATCH). Hidden courses excluded. Ordered by bm25, then limited.
    ///
    /// A query without any letters/digits returns no hits (not an error). `limit` is
    /// clamped to 1..=`MAX_SEARCH_LIMIT`.
    pub fn search(
        &self,
        query: &str,
        course_id: Option<&str>,
        limit: u32,
    ) -> Result<Vec<SearchHit>> {
        self.search_filtered(query, course_id, limit, false)
    }

    /// Like `search`, but only over courses whose material text an AI app may read
    /// (`Course::ai_materials() == Readable`: policy not `prohibited` and `ai_access` on).
    /// Used by the MCP server; the filter is part of the SQL so withheld text never leaves
    /// the database.
    pub fn search_ai_readable(
        &self,
        query: &str,
        course_id: Option<&str>,
        limit: u32,
    ) -> Result<Vec<SearchHit>> {
        self.search_filtered(query, course_id, limit, true)
    }

    fn search_filtered(
        &self,
        query: &str,
        course_id: Option<&str>,
        limit: u32,
        ai_readable_only: bool,
    ) -> Result<Vec<SearchHit>> {
        let Some(match_expression) = fts_match_expression(query) else {
            return Ok(Vec::new());
        };
        let limit = limit.clamp(1, MAX_SEARCH_LIMIT);
        // chunks_fts.rowid = chunks.id (external-content FTS table, see SCHEMA_V1).
        self.query_list(
            "SELECT m.id AS material_id, m.title AS material_title,
                    c.id AS course_id, c.code AS course_code,
                    ch.ord AS chunk_ord, ch.locator AS locator,
                    snippet(chunks_fts, 0, '«', '»', '…', 16) AS snippet,
                    m.url AS url, m.week_hint AS week_hint,
                    bm25(chunks_fts) AS score
             FROM chunks_fts
             JOIN chunks ch ON ch.id = chunks_fts.rowid
             JOIN materials m ON m.id = ch.material_id
             JOIN courses c ON c.id = m.course_id
             WHERE chunks_fts MATCH ?1
               AND c.hidden = 0
               AND (?2 IS NULL OR c.id = ?2)
               AND (?4 = 0 OR (c.ai_access = 1 AND c.ai_policy <> 'prohibited'))
             ORDER BY score, ch.id
             LIMIT ?3",
            params![match_expression, course_id, limit, ai_readable_only],
            search_hit_from_row,
        )
    }

    // ----- study plans ---------------------------------------------------------------------

    /// Validate and store a plan (created_at = now). Errors: `Invalid` when the horizon is
    /// reversed, there are more than `MAX_PLAN_ITEMS` items, an item title is empty, a
    /// text exceeds its `MAX_PLAN_*` length, or the plan's JSON exceeds
    /// `MAX_PLAN_JSON_BYTES`.
    ///
    /// Plans are written by AI clients over MCP, so storage is bounded: only the newest
    /// `MAX_STORED_STUDY_PLANS` plans are kept (older ones are deleted in the same step).
    pub fn save_study_plan(&self, plan: &StudyPlan) -> Result<StoredStudyPlan> {
        validate_study_plan(plan)?;
        let plan_json = serde_json::to_string(plan)?;
        // The per-field limits count characters, but JSON can make text up to 6× longer
        // (a control character becomes "\u0001"), so the total size is checked as well.
        if plan_json.len() > MAX_PLAN_JSON_BYTES {
            return Err(Error::Invalid(format!(
                "the study plan is too large ({} bytes as JSON, at most {MAX_PLAN_JSON_BYTES})",
                plan_json.len()
            )));
        }
        // Whole seconds, so the returned value equals what `latest_study_plan` reads back.
        let created_at = Utc::now().trunc_subsecs(0);
        let id = self.atomic(|| {
            self.conn.execute(
                "INSERT INTO study_plans (created_at, plan_json) VALUES (?1, ?2)",
                params![ts_text(created_at), plan_json],
            )?;
            let id = self.conn.last_insert_rowid();
            self.conn.execute(
                "DELETE FROM study_plans
                 WHERE id NOT IN (SELECT id FROM study_plans ORDER BY id DESC LIMIT ?1)",
                [MAX_STORED_STUDY_PLANS],
            )?;
            Ok(id)
        })?;
        Ok(StoredStudyPlan {
            id,
            created_at,
            plan: plan.clone(),
        })
    }

    /// The most recently saved plan (highest id), without the items of removed courses: the
    /// stored JSON keeps them (calendar design §8.3), every reader (the app, the MCP server,
    /// the digest, the weekly note) leaves them out. A write by item index must read the
    /// stored plan itself.
    pub fn latest_study_plan(&self) -> Result<Option<StoredStudyPlan>> {
        let row = self.query_opt(
            "SELECT id, created_at, plan_json FROM study_plans ORDER BY id DESC LIMIT 1",
            [],
            |row| {
                let id: i64 = row.get("id")?;
                let created_at: Timestamp = get_value(row, "created_at")?;
                let plan_json: String = row.get("plan_json")?;
                Ok((id, created_at, plan_json))
            },
        )?;
        let Some((id, created_at, plan_json)) = row else {
            return Ok(None);
        };
        let mut plan: StudyPlan = serde_json::from_str(&plan_json)?;
        let removed: Vec<String> =
            self.query_list("SELECT course_id FROM course_tombstones", [], |row| {
                row.get(0)
            })?;
        plan.items.retain(|item| {
            item.course_id
                .as_ref()
                .is_none_or(|id| !removed.contains(id))
        });
        Ok(Some(StoredStudyPlan {
            id,
            created_at,
            plan,
        }))
    }

    // ----- settings (schema 3) --------------------------------------------------------------

    /// The JSON value stored under `key`, if any. `Invalid` when it can't be read as `T` (a
    /// value written by another version); callers usually fall back to their default.
    pub fn setting<T: serde::de::DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let text: Option<String> =
            self.query_opt("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })?;
        text.map(|text| {
            serde_json::from_str(&text)
                .map_err(|err| Error::Invalid(format!("setting '{key}' could not be read: {err}")))
        })
        .transpose()
    }

    /// Like `setting`, but a stored value that can't be parsed (another version's shape) counts
    /// as absent, with a warning, while a failed read (a busy or damaged database) is still an
    /// error. For settings whose default is right when the value is unreadable, but where a
    /// failed read must never look like the default (consent, what the student turned off).
    pub fn setting_or_absent<T: serde::de::DeserializeOwned>(
        &self,
        key: &str,
    ) -> Result<Option<T>> {
        let text: Option<String> =
            self.query_opt("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })?;
        Ok(text.and_then(|text| match serde_json::from_str(&text) {
            Ok(value) => Some(value),
            Err(err) => {
                // The key and the kind of error only: never the stored value.
                tracing::warn!(key, kind = ?err.classify(), "unreadable setting; using the default");
                None
            }
        }))
    }

    /// Store `value` as JSON under `key`, replacing an earlier value.
    pub fn set_setting<T: serde::Serialize>(&self, key: &str, value: &T) -> Result<()> {
        let text = serde_json::to_string(value)?;
        self.conn.execute(
            "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value,
                                            updated_at = excluded.updated_at",
            params![key, text, now_text()],
        )?;
        Ok(())
    }

    /// Forget the value under `key` (absent is fine).
    pub fn remove_setting(&self, key: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM settings WHERE key = ?1", [key])?;
        Ok(())
    }

    // ----- statistics ----------------------------------------------------------------------

    /// `PRAGMA user_version` of this database (see `SCHEMA_VERSION`).
    pub fn schema_version(&self) -> Result<i64> {
        Ok(self
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))?)
    }

    /// Row counts (see `StoreCounts` for what each number means).
    pub fn counts(&self) -> Result<StoreCounts> {
        let sql = "SELECT
            (SELECT COUNT(*) FROM courses WHERE hidden = 0
               AND id NOT IN (SELECT course_id FROM course_tombstones)) AS courses,
            (SELECT COUNT(*) FROM courses WHERE hidden <> 0
               AND id NOT IN (SELECT course_id FROM course_tombstones)) AS hidden_courses,
            (SELECT COUNT(*) FROM course_tombstones) AS removed_courses,
            (SELECT COUNT(*) FROM modules) AS modules,
            (SELECT COUNT(*) FROM materials) AS materials,
            (SELECT COUNT(*) FROM materials m
              WHERE m.text_status = ?1
                AND EXISTS (SELECT 1 FROM chunks c WHERE c.material_id = m.id))
              AS indexed_materials,
            (SELECT COUNT(*) FROM chunks) AS chunks,
            (SELECT COUNT(*) FROM events) AS events,
            (SELECT COUNT(*) FROM study_plans) AS study_plans";
        Ok(self.conn.query_row(sql, [TextStatus::Ok.as_str()], |row| {
            Ok(StoreCounts {
                courses: row.get("courses")?,
                hidden_courses: row.get("hidden_courses")?,
                modules: row.get("modules")?,
                materials: row.get("materials")?,
                indexed_materials: row.get("indexed_materials")?,
                chunks: row.get("chunks")?,
                events: row.get("events")?,
                study_plans: row.get("study_plans")?,
                removed_courses: row.get("removed_courses")?,
            })
        })?)
    }

    /// Escape hatch for tests/migrations.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    // ----- query helpers -------------------------------------------------------------------

    /// All rows of `sql`, each converted by `map`.
    fn query_list<T, P, F>(&self, sql: &str, params: P, map: F) -> Result<Vec<T>>
    where
        P: Params,
        F: FnMut(&Row<'_>) -> rusqlite::Result<T>,
    {
        let mut statement = self.conn.prepare_cached(sql)?;
        let rows = statement.query_map(params, map)?;
        Ok(rows.collect::<rusqlite::Result<Vec<T>>>()?)
    }

    /// The first row of `sql` converted by `map`, or `None` when there is no row.
    fn query_opt<T, P, F>(&self, sql: &str, params: P, map: F) -> Result<Option<T>>
    where
        P: Params,
        F: FnOnce(&Row<'_>) -> rusqlite::Result<T>,
    {
        let mut statement = self.conn.prepare_cached(sql)?;
        Ok(statement.query_row(params, map).optional()?)
    }
}

/// Rolls a `Store::atomic` savepoint back unless it was released (committed). Doing this in
/// `Drop` also covers early `?` returns and panics.
struct SavepointGuard<'a> {
    conn: &'a Connection,
    released: bool,
}

impl SavepointGuard<'_> {
    fn release(mut self) -> Result<()> {
        self.conn.execute_batch("RELEASE store_atomic")?;
        self.released = true;
        Ok(())
    }
}

impl Drop for SavepointGuard<'_> {
    fn drop(&mut self) {
        if !self.released {
            // Undo everything since the SAVEPOINT, then remove the savepoint itself. Errors
            // cannot be reported from `drop`; the original error is already on its way up.
            let _ = self
                .conn
                .execute_batch("ROLLBACK TO store_atomic; RELEASE store_atomic");
        }
    }
}

/// The error for a nested `in_transaction`: a database "misuse" error whose message names the
/// rule, instead of SQLite's bare "cannot start a transaction within a transaction".
fn nested_transaction_error() -> Error {
    Error::Db(rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_MISUSE),
        Some(
            "Store::in_transaction must not be nested: a transaction is already open on this \
             connection (functions that open their own transaction, such as \
             ingest::index_file, must be called outside in_transaction)"
                .to_string(),
        ),
    ))
}

/// `NotFound` when an UPDATE/DELETE by id changed no row.
fn expect_changed(changed: usize, what: &str, id: &str) -> Result<()> {
    if changed == 0 {
        Err(Error::NotFound(format!("{what} '{id}'")))
    } else {
        Ok(())
    }
}

// ----- pre-migration backups ----------------------------------------------------------------

/// `<db>.v<version>.bak` next to the database, e.g. `pagelamp.db.v2.bak`.
pub fn backup_path(db: &Path, version: i64) -> PathBuf {
    let name = db.file_name().unwrap_or_default().to_string_lossy();
    db.with_file_name(format!("{name}.v{version}.bak"))
}

/// The copy of the database a migration made before changing it (the newest one; only that one
/// is kept). It holds course text like the database itself: mode 0600, never part of a report,
/// deleted with the last source (docs/design/v0.3-model-access.md §5.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DatabaseBackup {
    pub path: PathBuf,
    /// The schema version the database had when it was copied.
    pub schema_version: i64,
    /// When the copy was made (the file's modification time).
    pub made_at: Option<SystemTime>,
}

/// The pre-migration backup next to `db`, if there is one.
pub fn database_backup(db: &Path) -> Option<DatabaseBackup> {
    backup_files(db)
        .into_iter()
        .filter_map(|(path, version)| {
            let made_at = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
            path.is_file().then_some(DatabaseBackup {
                path,
                schema_version: version,
                made_at,
            })
        })
        .max_by_key(|backup| (backup.made_at, backup.schema_version))
}

/// Delete every pre-migration backup next to `db`, and copies left half-written. Returns how
/// many files were removed.
pub fn delete_database_backups(db: &Path) -> std::io::Result<usize> {
    let mut removed = 0;
    for (path, _) in backup_files(db).into_iter().chain(backup_temp_files(db)) {
        match std::fs::remove_file(&path) {
            Ok(()) => removed += 1,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
    }
    Ok(removed)
}

/// `(path, version)` of every `<db>.v<N>.bak` next to `db`.
fn backup_files(db: &Path) -> Vec<(PathBuf, i64)> {
    backup_siblings(db)
        .into_iter()
        .filter_map(|(path, rest)| Some((path, rest.strip_suffix(".bak")?.parse().ok()?)))
        .collect()
}

/// `<db>.v<N>.bak.<pid>.tmp` files: copies a process didn't finish.
fn backup_temp_files(db: &Path) -> Vec<(PathBuf, i64)> {
    backup_siblings(db)
        .into_iter()
        .filter(|(_, rest)| rest.contains(".bak.") && rest.ends_with(".tmp"))
        .map(|(path, _)| (path, 0))
        .collect()
}

/// Files next to `db` named `<db name>.v…`, with the text after `.v`.
fn backup_siblings(db: &Path) -> Vec<(PathBuf, String)> {
    let prefix = format!("{}.v", db.file_name().unwrap_or_default().to_string_lossy());
    let dir = match db.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let rest = name.strip_prefix(&prefix)?.to_string();
            Some((entry.path(), rest))
        })
        .collect()
}

/// A short code for why a backup failed (no path, no message).
fn backup_error_code(err: &Error) -> String {
    match err {
        Error::Io(io) => {
            // `StorageFull` → `storage_full`
            let mut code = String::new();
            for (i, c) in format!("{:?}", io.kind()).chars().enumerate() {
                if c.is_uppercase() && i > 0 {
                    code.push('_');
                }
                code.push(c.to_ascii_lowercase());
            }
            code
        }
        Error::Db(_) => "sqlite".into(),
        _ => "other".into(),
    }
}

/// Copy the database (`conn`, at `db`, schema `version`) to `backup_path(db, version)` with
/// `VACUUM INTO`, which is safe while other connections use the database (WAL). The copy is
/// written to a private temporary file first and renamed; older backups are then deleted.
/// `Ok(None)` when another process migrated the database meanwhile (the copy is dropped).
fn write_backup(conn: &Connection, db: &Path, version: i64) -> Result<Option<PathBuf>> {
    let name = db.file_name().unwrap_or_default().to_string_lossy();
    let temp = db.with_file_name(format!("{name}.v{version}.bak.{}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&temp);
    // Created empty and private first: VACUUM INTO accepts an empty file and keeps its mode.
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    drop(options.open(&temp)?);
    let copied = conn
        .execute("VACUUM INTO ?1", [temp.to_string_lossy()])
        .map_err(Error::from)
        .and_then(|_| {
            let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
            let copy = Connection::open_with_flags(&temp, flags)?;
            Ok(copy.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))?)
        });
    match copied {
        Ok(copied) if copied == version => {}
        Ok(_) => {
            let _ = std::fs::remove_file(&temp);
            return Ok(None);
        }
        Err(err) => {
            let _ = std::fs::remove_file(&temp);
            return Err(err);
        }
    }
    let backup = backup_path(db, version);
    if let Err(err) = std::fs::rename(&temp, &backup) {
        let _ = std::fs::remove_file(&temp);
        return Err(err.into());
    }
    for (older, _) in backup_files(db) {
        if older != backup {
            let _ = std::fs::remove_file(older);
        }
    }
    Ok(Some(backup))
}

// ----- course resolution helpers ------------------------------------------------------------

/// Lower-case and drop all whitespace, so "demo 101h1" and "DEMO101H1" compare equal.
fn normalise_code(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Outcome of one `resolve_course_with` rule: `None` = no match (try the next rule),
/// otherwise the single match or `Ambiguous`.
fn pick_course(query: &str, matches: Vec<&Course>) -> Option<Result<Course>> {
    match matches.as_slice() {
        [] => None,
        [only] => Some(Ok((*only).clone())),
        several => Some(Err(Error::Ambiguous {
            query: query.to_string(),
            candidates: several
                .iter()
                .map(|c| format!("{} (id: {})", c.display_name(), c.id))
                .collect(),
        })),
    }
}

fn course_not_found(query: &str, courses: &[Course]) -> Error {
    let available: Vec<&str> = courses
        .iter()
        .map(|c| c.code.as_deref().unwrap_or(&c.name))
        .collect();
    let available = if available.is_empty() {
        "none (no courses synced yet)".to_string()
    } else {
        available.join(", ")
    };
    Error::NotFound(format!(
        "no course matches '{query}'; available courses: {available}"
    ))
}

// ----- search helpers -----------------------------------------------------------------------

/// Turn free text into a safe FTS5 MATCH expression, or `None` when it contains no words.
///
/// Raw user text is never passed to MATCH: FTS5 syntax such as `NEAR(`, `"`, `*`, `-`,
/// `col:`, `^` or AND/OR/NOT would raise errors or change the meaning. Instead every run of
/// Unicode letters/digits becomes one double-quoted term (matched literally), and the terms
/// are OR-ed so any word may match; bm25 ranks chunks that match more/rarer words first.
fn fts_match_expression(query: &str) -> Option<String> {
    let terms: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .take(MAX_SEARCH_TERMS)
        .map(|word| {
            let word: String = word.chars().take(MAX_SEARCH_TERM_CHARS).collect();
            // A word cannot contain '"' (not alphanumeric), but quote-escape anyway: " → "".
            format!("\"{}\"", word.replace('"', "\"\""))
        })
        .collect();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" OR "))
    }
}

// ----- study plan validation ----------------------------------------------------------------

fn validate_study_plan(plan: &StudyPlan) -> Result<()> {
    if plan.horizon_start > plan.horizon_end {
        return Err(Error::Invalid(format!(
            "horizon_start {} is after horizon_end {}",
            plan.horizon_start, plan.horizon_end
        )));
    }
    if plan.items.len() > MAX_PLAN_ITEMS {
        return Err(Error::Invalid(format!(
            "a study plan may have at most {MAX_PLAN_ITEMS} items (got {})",
            plan.items.len()
        )));
    }
    check_length("notes", plan.notes.as_deref(), MAX_PLAN_TEXT_CHARS)?;
    for (index, item) in plan.items.iter().enumerate() {
        let n = index + 1; // 1-based in messages
        if item.title.trim().is_empty() {
            return Err(Error::Invalid(format!("item {n}: title must not be empty")));
        }
        check_length(
            &format!("item {n}: title"),
            Some(&item.title),
            MAX_PLAN_TITLE_CHARS,
        )?;
        check_length(
            &format!("item {n}: description"),
            item.description.as_deref(),
            MAX_PLAN_TEXT_CHARS,
        )?;
        check_length(
            &format!("item {n}: course_id"),
            item.course_id.as_deref(),
            MAX_PLAN_ID_CHARS,
        )?;
        if item.material_ids.len() > MAX_PLAN_MATERIAL_IDS {
            return Err(Error::Invalid(format!(
                "item {n}: at most {MAX_PLAN_MATERIAL_IDS} material_ids allowed"
            )));
        }
        for material_id in &item.material_ids {
            check_length(
                &format!("item {n}: material id"),
                Some(material_id),
                MAX_PLAN_ID_CHARS,
            )?;
        }
    }
    Ok(())
}

/// `Invalid` when `value` has more than `max` characters.
fn check_length(field: &str, value: Option<&str>, max: usize) -> Result<()> {
    match value {
        Some(text) if text.chars().count() > max => Err(Error::Invalid(format!(
            "{field} is longer than {max} characters"
        ))),
        _ => Ok(()),
    }
}

// ----- row conversion -----------------------------------------------------------------------
// Each function reads the columns of the matching `*_COLUMNS` list by name.

fn source_from_row(row: &Row<'_>) -> rusqlite::Result<SourceRecord> {
    Ok(SourceRecord {
        id: row.get("id")?,
        kind: get_value(row, "kind")?,
        label: row.get("label")?,
        // rusqlite parses the JSON text into a `serde_json::Value`.
        config: row.get("config_json")?,
        last_synced_at: get_opt_value(row, "last_synced_at")?,
        last_error: row.get("last_error")?,
        last_error_kind: get_opt_value(row, "last_error_kind")?,
    })
}

fn course_from_row(row: &Row<'_>) -> rusqlite::Result<Course> {
    Ok(Course {
        id: row.get("id")?,
        source_id: row.get("source_id")?,
        external_id: row.get("external_id")?,
        code: row.get("code")?,
        name: row.get("name")?,
        term_start: get_opt_value(row, "term_start")?,
        term_end: get_opt_value(row, "term_end")?,
        term_source: match row.get_ref("term_source")?.as_str()? {
            "user" => TermSource::User,
            "synced" => TermSource::Synced,
            _ => TermSource::None,
        },
        url: row.get("url")?,
        ai_policy: get_value(row, "ai_policy")?,
        ai_policy_note: row.get("ai_policy_note")?,
        ai_access: row.get("ai_access")?,
        material_sharing: get_value(row, "material_sharing")?,
        enrollment_active: row.get("enrollment_active")?,
        hidden: row.get("hidden")?,
        updated_at: get_value(row, "updated_at")?,
    })
}

fn term_data_from_row(row: &Row<'_>) -> rusqlite::Result<CourseTermData> {
    Ok(CourseTermData {
        lms: LmsCourseInfo {
            term_name: row.get("lms_term_name")?,
            term_start: get_opt_value(row, "lms_term_start")?,
            term_end: get_opt_value(row, "lms_term_end")?,
            course_start: get_opt_value(row, "lms_course_start")?,
            course_end: get_opt_value(row, "lms_course_end")?,
            time_zone: row.get("lms_time_zone")?,
            concluded: row.get("lms_concluded")?,
            workflow_state: row.get("lms_workflow_state")?,
            access_restricted: row.get("lms_access_restricted")?,
        },
        keep_current_until: get_opt_value(row, "keep_current_until")?,
        removal_snoozed_until: get_opt_value(row, "removal_snoozed_until")?,
        synced_term_start: get_opt_value(row, "term_start")?,
        synced_term_end: get_opt_value(row, "term_end")?,
        user_term_start: get_opt_value(row, "user_term_start")?,
        user_term_end: get_opt_value(row, "user_term_end")?,
        institution: row.get("institution")?,
    })
}

fn module_from_row(row: &Row<'_>) -> rusqlite::Result<Module> {
    Ok(Module {
        id: row.get("id")?,
        course_id: row.get("course_id")?,
        name: row.get("name")?,
        position: row.get("position")?,
        unlock_at: get_opt_value(row, "unlock_at")?,
        week_hint: row.get("week_hint")?,
    })
}

fn material_from_row(row: &Row<'_>) -> rusqlite::Result<Material> {
    Ok(Material {
        id: row.get("id")?,
        course_id: row.get("course_id")?,
        module_id: row.get("module_id")?,
        kind: get_value(row, "kind")?,
        title: row.get("title")?,
        url: row.get("url")?,
        local_path: row.get("local_path")?,
        mime: row.get("mime")?,
        published_at: get_opt_value(row, "published_at")?,
        week_hint: row.get("week_hint")?,
        content_hash: row.get("content_hash")?,
        text_status: get_value(row, "text_status")?,
        text_error: row.get("text_error")?,
        text_error_kind: get_opt_value(row, "text_error_kind")?,
        text_error_fingerprint: row.get("text_error_fingerprint")?,
        download_blocked: get_opt_value(row, "download_blocked")?,
        updated_at: get_value(row, "updated_at")?,
    })
}

fn chunk_from_row(row: &Row<'_>) -> rusqlite::Result<Chunk> {
    Ok(Chunk {
        material_id: row.get("material_id")?,
        ord: row.get("ord")?,
        locator: row.get("locator")?,
        text: row.get("text")?,
    })
}

fn event_from_row(row: &Row<'_>) -> rusqlite::Result<Event> {
    Ok(Event {
        id: row.get("id")?,
        source_id: row.get("source_id")?,
        course_id: row.get("course_id")?,
        kind: get_value(row, "kind")?,
        title: row.get("title")?,
        starts_at: get_opt_value(row, "starts_at")?,
        ends_at: get_opt_value(row, "ends_at")?,
        due_at: get_opt_value(row, "due_at")?,
        url: row.get("url")?,
        updated_at: get_value(row, "updated_at")?,
        course_hint: row.get("course_hint")?,
    })
}

fn search_hit_from_row(row: &Row<'_>) -> rusqlite::Result<SearchHit> {
    Ok(SearchHit {
        material_id: row.get("material_id")?,
        material_title: row.get("material_title")?,
        course_id: row.get("course_id")?,
        course_code: row.get("course_code")?,
        chunk_ord: row.get("chunk_ord")?,
        locator: row.get("locator")?,
        snippet: row.get("snippet")?,
        url: row.get("url")?,
        week_hint: row.get("week_hint")?,
        score: row.get("score")?,
    })
}

// ----- text encodings -----------------------------------------------------------------------
// Writing: `ts_text` / `date_text` / `as_str()`. Reading: `get_value` / `get_opt_value`.

const DATE_FORMAT: &str = "%Y-%m-%d";

/// Earliest instant that fits the fixed format: 0000-01-01T00:00:00Z.
const EARLIEST_STORED_INSTANT: Timestamp = NaiveDate::from_ymd_opt(0, 1, 1)
    .unwrap()
    .and_hms_opt(0, 0, 0)
    .unwrap()
    .and_utc();
/// Latest instant that fits the fixed format: 9999-12-31T23:59:59Z.
const LATEST_STORED_INSTANT: Timestamp = NaiveDate::from_ymd_opt(9999, 12, 31)
    .unwrap()
    .and_hms_opt(23, 59, 59)
    .unwrap()
    .and_utc();

/// The ONE way an instant is written to the DB: RFC 3339, UTC, whole seconds (sub-seconds
/// are truncated), `Z` suffix — "2026-09-25T12:00:00Z".
///
/// Instants outside years 0000–9999 are clamped to that range first. chrono would write them
/// with a sign ("+10000-01-01T…"), which sorts before every digit (breaking the text
/// comparisons in SQL) and is not accepted by the reader. Such values only occur as
/// "forever" sentinels (e.g. 9999-12-31 in a UTC-8 feed, or `DateTime::MAX_UTC` as an open
/// query bound), so clamping keeps their meaning.
fn ts_text(instant: Timestamp) -> String {
    instant
        .clamp(EARLIEST_STORED_INSTANT, LATEST_STORED_INSTANT)
        .to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn opt_ts_text(instant: Option<Timestamp>) -> Option<String> {
    instant.map(ts_text)
}

fn now_text() -> String {
    ts_text(Utc::now())
}

/// Calendar dates are written as "YYYY-MM-DD".
fn date_text(date: NaiveDate) -> String {
    date.format(DATE_FORMAT).to_string()
}

fn opt_date_text(date: Option<NaiveDate>) -> Option<String> {
    date.map(date_text)
}

/// A type stored as TEXT in our own format (instants, dates, enums). `parse_text` returns
/// `None` for anything unexpected, which `get_value` turns into an error (never a panic).
trait TextValue: Sized {
    fn parse_text(text: &str) -> Option<Self>;
}

impl TextValue for Timestamp {
    fn parse_text(text: &str) -> Option<Self> {
        DateTime::parse_from_rfc3339(text)
            .ok()
            .map(|instant| instant.with_timezone(&Utc))
    }
}

impl TextValue for NaiveDate {
    fn parse_text(text: &str) -> Option<Self> {
        NaiveDate::parse_from_str(text, DATE_FORMAT).ok()
    }
}

impl TextValue for AiPolicy {
    fn parse_text(text: &str) -> Option<Self> {
        text.parse().ok()
    }
}

impl TextValue for SourceErrorKind {
    fn parse_text(text: &str) -> Option<Self> {
        text.parse().ok()
    }
}

// The enums below have no `FromStr` in `model`; they are parsed by comparing with each
// variant's `as_str()`, so the stored spelling is defined in exactly one place.
// When adding a variant to one of these enums, add it to its list here too.

impl TextValue for SourceKind {
    fn parse_text(text: &str) -> Option<Self> {
        let all = [SourceKind::Canvas, SourceKind::Folder, SourceKind::Ical];
        variant_named(text, &all, SourceKind::as_str)
    }
}

impl TextValue for MaterialKind {
    fn parse_text(text: &str) -> Option<Self> {
        let all = [
            MaterialKind::File,
            MaterialKind::Page,
            MaterialKind::Announcement,
            MaterialKind::Syllabus,
            MaterialKind::ExternalLink,
        ];
        variant_named(text, &all, MaterialKind::as_str)
    }
}

impl TextValue for TextStatus {
    fn parse_text(text: &str) -> Option<Self> {
        let all = [
            TextStatus::Pending,
            TextStatus::Ok,
            TextStatus::Unsupported,
            TextStatus::NotDownloaded,
            TextStatus::Error,
        ];
        variant_named(text, &all, TextStatus::as_str)
    }
}

impl TextValue for TextErrorKind {
    fn parse_text(text: &str) -> Option<Self> {
        variant_named(text, &TextErrorKind::ALL, TextErrorKind::as_str)
    }
}

impl TextValue for crate::ai::MaterialSharing {
    fn parse_text(text: &str) -> Option<Self> {
        variant_named(
            text,
            &crate::ai::MaterialSharing::ALL,
            crate::ai::MaterialSharing::as_str,
        )
    }
}

impl TextValue for crate::ai::AiFeature {
    fn parse_text(text: &str) -> Option<Self> {
        variant_named(
            text,
            &crate::ai::AiFeature::ALL,
            crate::ai::AiFeature::as_str,
        )
    }
}

impl TextValue for DownloadBlock {
    fn parse_text(text: &str) -> Option<Self> {
        let all = [DownloadBlock::Locked, DownloadBlock::TooLarge];
        variant_named(text, &all, DownloadBlock::as_str)
    }
}

impl TextValue for EventKind {
    fn parse_text(text: &str) -> Option<Self> {
        let all = [
            EventKind::AssignmentDue,
            EventKind::QuizDue,
            EventKind::Exam,
            EventKind::ClassEvent,
            EventKind::PlannerItem,
            EventKind::Other,
        ];
        variant_named(text, &all, EventKind::as_str)
    }
}

/// The variant in `all` whose `name` is `text`.
fn variant_named<T: Copy>(text: &str, all: &[T], name: fn(T) -> &'static str) -> Option<T> {
    all.iter().copied().find(|variant| name(*variant) == text)
}

/// Adapter that lets rusqlite read a `TextValue` column; a bad value becomes a
/// `FromSqlConversionFailure` naming the column index.
struct Stored<T>(T);

impl<T: TextValue> FromSql for Stored<T> {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let text = value.as_str()?;
        T::parse_text(text).map(Stored).ok_or_else(|| {
            let type_name = std::any::type_name::<T>();
            FromSqlError::Other(format!("unexpected stored {type_name} value '{text}'").into())
        })
    }
}

/// Read a NOT NULL text-encoded column (instant, date or enum).
fn get_value<T: TextValue>(row: &Row<'_>, column: &str) -> rusqlite::Result<T> {
    Ok(row.get::<_, Stored<T>>(column)?.0)
}

/// Read a nullable text-encoded column (instant, date or enum).
fn get_opt_value<T: TextValue>(row: &Row<'_>, column: &str) -> rusqlite::Result<Option<T>> {
    Ok(row
        .get::<_, Option<Stored<T>>>(column)?
        .map(|stored| stored.0))
}

#[cfg(test)]
mod tests {
    //! Unit tests for the private helpers. The public API is tested in
    //! `tests/store_api.rs` (in-memory) and `tests/store_files.rs` (file-backed, concurrency).

    use super::*;

    #[test]
    fn migrations_match_schema_version() {
        assert_eq!(MIGRATIONS.len() as i64, SCHEMA_VERSION);
        assert_eq!(MIGRATIONS[0], SCHEMA_V1);
    }

    #[test]
    fn instants_use_one_fixed_format() {
        let instant = DateTime::parse_from_rfc3339("2026-09-25T14:30:05.987+02:00")
            .unwrap()
            .with_timezone(&Utc);
        // UTC, whole seconds (truncated), `Z` suffix.
        assert_eq!(ts_text(instant), "2026-09-25T12:30:05Z");
        assert_eq!(
            Timestamp::parse_text("2026-09-25T12:30:05Z"),
            Some(instant.trunc_subsecs(0))
        );
        assert_eq!(Timestamp::parse_text("2026-09-25 12:30:05"), None);
        assert_eq!(Timestamp::parse_text(""), None);
    }

    #[test]
    fn fixed_format_sorts_like_time() {
        let early = DateTime::parse_from_rfc3339("2026-01-02T03:04:05Z")
            .unwrap()
            .with_timezone(&Utc);
        let late = DateTime::parse_from_rfc3339("2026-11-12T13:14:15Z")
            .unwrap()
            .with_timezone(&Utc);
        assert!(ts_text(early) < ts_text(late));
        assert_eq!(ts_text(early).len(), ts_text(late).len());
    }

    #[test]
    fn instants_outside_the_four_digit_years_are_clamped() {
        assert_eq!(ts_text(DateTime::<Utc>::MAX_UTC), "9999-12-31T23:59:59Z");
        assert_eq!(ts_text(DateTime::<Utc>::MIN_UTC), "0000-01-01T00:00:00Z");
        // The boundaries themselves (and years < 1000) are written unchanged, zero-padded.
        assert_eq!(ts_text(LATEST_STORED_INSTANT), "9999-12-31T23:59:59Z");
        assert_eq!(ts_text(EARLIEST_STORED_INSTANT), "0000-01-01T00:00:00Z");
        let early = DateTime::parse_from_rfc3339("0999-03-04T05:06:07Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(ts_text(early), "0999-03-04T05:06:07Z");
        // Everything written can be read back.
        for text in ["9999-12-31T23:59:59Z", "0000-01-01T00:00:00Z"] {
            let parsed = Timestamp::parse_text(text).unwrap();
            assert_eq!(ts_text(parsed), text);
        }
    }

    #[test]
    fn dates_round_trip() {
        let date = NaiveDate::from_ymd_opt(2026, 9, 8).unwrap();
        assert_eq!(date_text(date), "2026-09-08");
        assert_eq!(NaiveDate::parse_text("2026-09-08"), Some(date));
        assert_eq!(NaiveDate::parse_text("08/09/2026"), None);
    }

    #[test]
    fn every_enum_variant_round_trips_through_its_text() {
        fn round_trip<T: TextValue + Copy + PartialEq + std::fmt::Debug>(
            all: &[T],
            name: fn(T) -> &'static str,
        ) {
            for &variant in all {
                assert_eq!(T::parse_text(name(variant)), Some(variant));
            }
            assert_eq!(T::parse_text("no_such_variant"), None);
        }
        round_trip(
            &[SourceKind::Canvas, SourceKind::Folder, SourceKind::Ical],
            SourceKind::as_str,
        );
        round_trip(
            &[
                MaterialKind::File,
                MaterialKind::Page,
                MaterialKind::Announcement,
                MaterialKind::Syllabus,
                MaterialKind::ExternalLink,
            ],
            MaterialKind::as_str,
        );
        round_trip(
            &[
                TextStatus::Pending,
                TextStatus::Ok,
                TextStatus::Unsupported,
                TextStatus::NotDownloaded,
                TextStatus::Error,
            ],
            TextStatus::as_str,
        );
        round_trip(
            &[
                EventKind::AssignmentDue,
                EventKind::QuizDue,
                EventKind::Exam,
                EventKind::ClassEvent,
                EventKind::PlannerItem,
                EventKind::Other,
            ],
            EventKind::as_str,
        );
        round_trip(
            &[
                AiPolicy::Unknown,
                AiPolicy::Prohibited,
                AiPolicy::LearningAid,
                AiPolicy::AllowedWithCitation,
                AiPolicy::Unrestricted,
            ],
            AiPolicy::as_str,
        );
        round_trip(
            &[
                SourceErrorKind::AuthExpiredOrRevoked,
                SourceErrorKind::Network,
                SourceErrorKind::NotFound,
                SourceErrorKind::RateLimited,
                SourceErrorKind::Other,
            ],
            SourceErrorKind::as_str,
        );
        round_trip(
            &[DownloadBlock::Locked, DownloadBlock::TooLarge],
            DownloadBlock::as_str,
        );
        round_trip(&TextErrorKind::ALL, TextErrorKind::as_str);
    }

    #[test]
    fn stored_enum_text_matches_json_names() {
        // The DB text and the MCP/JSON spelling are the same (`serde(rename_all = "snake_case")`).
        let json = serde_json::to_value(MaterialKind::ExternalLink).unwrap();
        assert_eq!(json, MaterialKind::ExternalLink.as_str());
        let json = serde_json::to_value(TextStatus::NotDownloaded).unwrap();
        assert_eq!(json, TextStatus::NotDownloaded.as_str());
        let json = serde_json::to_value(DownloadBlock::TooLarge).unwrap();
        assert_eq!(json, DownloadBlock::TooLarge.as_str());
        for kind in TextErrorKind::ALL {
            assert_eq!(serde_json::to_value(kind).unwrap(), kind.as_str());
        }
    }

    #[test]
    fn fts_expression_quotes_and_ors_words() {
        assert_eq!(
            fts_match_expression("gradient descent").as_deref(),
            Some(r#""gradient" OR "descent""#)
        );
        assert_eq!(
            fts_match_expression(r#"NEAR("a" b)*"#).as_deref(),
            Some(r#""NEAR" OR "a" OR "b""#)
        );
        assert_eq!(
            fts_match_expression("col:x -y ^z").as_deref(),
            Some(r#""col" OR "x" OR "y" OR "z""#)
        );
        // Unicode letters/digits are kept together.
        assert_eq!(
            fts_match_expression("Übung 3 — 第三周").as_deref(),
            Some(r#""Übung" OR "3" OR "第三周""#)
        );
    }

    #[test]
    fn fts_expression_is_none_without_words() {
        for query in ["", "   ", "\"", "*", "-", "^", "()", "\"\" * - ^ : ( )"] {
            assert_eq!(fts_match_expression(query), None, "query {query:?}");
        }
    }

    #[test]
    fn fts_expression_caps_terms_and_term_length() {
        let many = "word ".repeat(1000);
        let expression = fts_match_expression(&many).unwrap();
        assert_eq!(expression.matches(" OR ").count(), MAX_SEARCH_TERMS - 1);

        let long_word = "x".repeat(10_000);
        let expression = fts_match_expression(&long_word).unwrap();
        assert_eq!(expression.len(), MAX_SEARCH_TERM_CHARS + 2);
    }

    #[test]
    fn code_normalisation_ignores_case_and_spaces() {
        assert_eq!(normalise_code(" Demo 101h1 "), "demo101h1");
        assert_eq!(normalise_code("DEMO101H1"), "demo101h1");
    }
}
