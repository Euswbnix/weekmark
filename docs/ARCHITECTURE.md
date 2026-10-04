# PageLamp v0.1 — Architecture & team contract

Status: agreed baseline (2026-09-25). Changes to anything in §3–§5 go through the maintainers.

## 1. Product scope (v0.1 = "MCP-first")

PageLamp syncs a student's **own** course data to a local SQLite knowledge base and serves it over
**MCP** to the AI app the student already pays for (Claude Desktop, Claude Code, Codex, …). The
student's subscription supplies the model; PageLamp runs no model and no server.

In scope: study plans, weekly content explanations, "where is each course this week", deadlines for
planning, catching up. **Out of scope:** fetching assignment instructions to solve them, submitting
anything, any write to the LMS, a chat UI of our own, our own backend server.

Target users: students outside mainland China (Canvas first; folder + calendar-feed sources make it
work for Brightspace/Moodle schools too). UI English-first, zh-CN second.

## 2. Components

```
                        ┌──────────── student's AI app (their subscription) ────────────┐
                        │ Claude Desktop      Claude Code        Codex        Cursor …  │
                        └──────┬──────────────────┬─────────────────┬───────────────────┘
                     spawns    │ stdio MCP        │                 │   (one process per client)
                               ▼                  ▼                 ▼
                        pagelamp mcp       pagelamp mcp     pagelamp mcp
                               │   read-only, per-request connection (WAL)
                               ▼
   pagelamp sync ───►  <data_dir>/pagelamp.db  (+ <data_dir>/files/ cache)
   (CLI or desktop app,        ▲   single writer, advisory sync.lock
    the only network user)     │
   Canvas API (GET only) ──────┤
   Local course folder ────────┤
   iCal feed (deadlines) ──────┘
```

Rust workspace:

| Crate | Purpose | Area |
|---|---|---|
| `crates/pagelamp-core` | model, store (SQLite + FTS5), ingest, timeline, views, paths, secrets | backend |
| `crates/pagelamp-extract` | PDF/PPTX/DOCX/ipynb/HTML/text extraction + chunking | backend |
| `crates/pagelamp-canvas` | read-only Canvas sync (personal token) | backend |
| `crates/pagelamp-local` | folder source + iCal source | backend |
| `crates/pagelamp-mcp` | rmcp 3.4 stdio server: tools + prompts | backend |
| `crates/pagelamp-app` | **the facade** used by CLI and desktop app | backend |
| `apps/pagelamp-cli` | `pagelamp` binary | backend |
| `apps/desktop` (+ `src-tauri`) | Tauri 2 + React desktop shell | frontend |
| root `Cargo.toml`, `docs/` | workspace + contracts | shared |
| root `package.json`, `pnpm-workspace.yaml` (if any) | JS tooling | frontend |

## 3. Hard rules (product/policy requirements)

1. **Canvas access is GET-only**, from `pagelamp-canvas` only, triggered by sync only. A sync starts
   only from the student's action in PageLamp or the CLI, or from the app's own timer under the
   student's setting (automatic sync); no MCP tool call may cause, request or schedule a sync, and
   nothing an MCP client can write is read by the timer's rule. A run the timer starts with nobody
   at the app makes no `/courses/:id/…` request: it checks the token and reads the course list,
   planner items and each course's announcements. Everything else is read when the student starts
   a sync or comes back to the app. An automatic run never downloads files. The MCP server
   never touches the network (Canvas API Policy §3(i) restricts access to Canvas APIs through MCP
   servers that Instructure hasn't approved; our understanding is that this rules out Canvas access
   from the MCP server, so all Canvas access happens in sync).
2. **Personal token = personal use.** Instructure: asking other users to manually generate a token
   for your app violates the API Policy. UI and CLI must say so when adding a Canvas source; the
   folder + calendar-feed path is the shareable one. Tokens expire (the maximum is set per school/role and shown by Canvas, e.g. 90 days at some schools; Instructure caps student-only users at 30) — surface
   expiry/401 clearly.
3. **Secrets** (Canvas token, calendar-feed URL) live only in the OS keychain (`core::secrets`);
   never in the DB, logs, MCP output, frontend state beyond the input field, or test fixtures.
4. **No assignment solving.** Assignments are recorded as title + due date + link only.
5. **Untrusted content.** Course text returned over MCP is wrapped in `<course_material …>` tags and
   the server instructions say it is data, not instructions.
6. **AI disclosure** (Canvas API Policy §2E): UI states that course text is sent to whatever AI app
   the student connects, and that PageLamp itself stores nothing remotely.
7. **Test data is synthetic.** No real course materials, names, or tokens anywhere in the repo.
8. **AI access to course materials is the student's choice, on by default** (decided 2026-09-25).
   - One-time disclosure at onboarding (and printed by `pagelamp mcp-config`): when the student
     asks their AI app about a course, the app reads that course's materials from PageLamp and
     sends them to the AI provider under the student's own account; the student is responsible
     for following each course's AI policy; sharing can be turned off per course.
   - Per-course switch `ai_access` ("Let my AI app read this course's materials"), default **on**.
   - If the student marks a course `ai_policy = prohibited`, its material **text** is withheld from
     MCP regardless of the switch (the switch keeps its stored value and applies again if the policy
     changes). Structure, titles, deadlines and the study plan remain available for planning.
   - Effective state is computed, never stored: `AiMaterialsState = readable | turned_off |
     withheld_by_policy`.
   - "Material text" = `read_material` content, `search_materials` snippets, announcement bodies,
     syllabus text, and any text a prompt would inline. Titles, kinds, dates, week numbers, URLs and
     counts are structure and stay available.

## 4. Concurrency (why there is no connection pool)

- stdio MCP is 1:1 — each AI client spawns its own `pagelamp mcp` process; nothing is shared.
- The DB is SQLite in WAL mode: many readers, one writer, readers never block each other.
- MCP opens a fresh read-only connection per request inside `spawn_blocking` (sub-ms). No global
  mutex, never hold a connection across `.await`.
- Heavy work (download, extract, chunk, index) happens at sync time only; MCP tools are indexed reads.
- `save_study_plan` is the only MCP write: short read-write transaction + `busy_timeout=5000`.
- `pagelamp mcp` must stay lightweight at startup (no model loading, no network).

## 5. App facade API (`pagelamp-app`) — the backend ⇄ frontend contract

Backend implements; frontend's Tauri commands are thin 1:1 wrappers. Names below are agreed; field
details may be refined in the backend; the TypeScript types are then regenerated from the schema (below).
All return types are `Serialize + JsonSchema`; `pagelamp schema` prints them as JSON Schema →
frontend generates TS with `json-schema-to-typescript` (no hand-written duplicate types).

```rust
pub struct App { /* data_dir */ }
impl App {
    pub fn open() -> Result<App>;                          // default data dir (PAGELAMP_HOME respected)
    pub fn open_at(data_dir: PathBuf) -> Result<App>;

    // status & sources
    pub fn status(&self) -> Result<AppStatus>;            // data_dir, db_path, sources, counts, last sync
    pub fn list_sources(&self) -> Result<Vec<SourceRecord>>;
    pub async fn add_canvas_source(&self, base_url: &str, token: &str) -> Result<SourceRecord>; // validates token
    pub fn add_folder_source(&self, path: &Path, term_start: Option<NaiveDate>, label: Option<&str>) -> Result<SourceRecord>;
    pub async fn add_ical_source(&self, feed_url: &str, label: Option<&str>) -> Result<SourceRecord>; // validates by fetching
    pub fn remove_source(&self, source_id: &str) -> Result<()>;  // rows + its secret (none for folders) + Canvas downloads; Busy during a sync
    /// Replace an expired/revoked Canvas token or a changed feed URL WITHOUT removing the
    /// source (remove cascades to courses + user overrides). Validates exactly like add_*,
    /// then overwrites the keychain entry and clears last_error/last_error_kind.
    pub async fn update_source_secret(&self, source_id: &str, secret: &str) -> Result<SourceRecord>;

    // sync (progress streamed to the UI; desktop forwards via tauri::ipc::Channel)
    pub async fn sync_all(&self, req: SyncRequest, on_event: impl Fn(SyncEvent) + Send + Sync) -> Result<SyncSummary>;
    pub async fn sync_source(&self, source_id: &str, req: SyncRequest, on_event: impl Fn(SyncEvent) + Send + Sync) -> Result<SourceSyncResult>;
    /// Explicit per-course "download & index files" for an LMS course (downloads can count as
    /// "viewed" in Canvas module requirements, so never part of a normal sync).
    pub async fn download_course_files(&self, course: &str, on_event: impl Fn(SyncEvent) + Send + Sync) -> Result<SourceSyncResult>;

    // read views (same functions back the MCP tools)
    pub fn list_courses(&self) -> Result<Vec<CourseSummary>>;           // course + timeline + counts + next deadline
    pub fn course_overview(&self, course: &str) -> Result<CourseOverview>;
    pub fn week_materials(&self, course: &str, week: Option<u32>) -> Result<WeekMaterials>;
    pub fn list_deadlines(&self, course: Option<&str>, days_ahead: u32, days_back: u32) -> Result<Vec<Deadline>>;
    pub fn search(&self, query: &str, course: Option<&str>, limit: u32) -> Result<Vec<SearchHit>>;
    pub fn latest_study_plan(&self) -> Result<Option<StoredStudyPlan>>;

    // course settings
    pub fn set_course_policy(&self, course: &str, policy: AiPolicy, note: Option<&str>) -> Result<()>;
    pub fn set_course_term(&self, course: &str, start: Option<NaiveDate>, end: Option<NaiveDate>) -> Result<()>;
    pub fn set_course_hidden(&self, course: &str, hidden: bool) -> Result<()>;
    pub fn set_course_ai_access(&self, course: &str, allowed: bool) -> Result<()>;  // §3 rule 8

    // "connect your AI app"
    pub fn mcp_client_configs(&self, pagelamp_binary: &Path) -> Vec<McpClientConfig>;
    pub fn mcp_launch(&self, pagelamp_binary: &Path) -> McpLaunch;   // command + args + env

    // diagnostics (logs, crash notice, doctor, redacted report)
    pub fn logs_dir(&self) -> Result<PathBuf>;
    pub fn last_crash(&self) -> Result<Option<CrashReport>>;
    pub fn clear_last_crash(&self) -> Result<()>;
    pub fn doctor(&self) -> Result<DoctorReport>;
    pub fn diagnostic_report(&self) -> Result<String>;    // Markdown, redacted, course names → "Course N"
}

pub enum SyncEvent {            // serde tag = "type"
    SourceStarted { source_id, label },
    Progress { source_id, message, current: Option<u32>, total: Option<u32> },
    Warning { source_id, message },
    SourceFinished { source_id, ok: bool, error: Option<String>, error_kind: Option<SourceErrorKind> },
}

/// Structured failure classes — the UI branches on these, never on message strings.
/// Persisted on the source row as `last_error_kind` next to `last_error`
/// (`SourceRecord.last_error_kind`, schema v1 column `sources.last_error_kind TEXT`).
pub enum SourceErrorKind {      // serde snake_case
    AuthExpiredOrRevoked,       // Canvas 401 / invalid_token; feed URL 401/403
    Network,                    // DNS, TLS, timeout, connection refused
    NotFound,                   // folder missing, feed 404, Canvas host 404
    RateLimited,                // Canvas throttling exhausted retries
    Other,
}

/// Every facade method returns `Result<T, AppError>`; Tauri commands serialise it as-is.
pub struct AppError {           // Serialize + JsonSchema
    kind: AppErrorKind,         // auth | network | invalid | not_found | ambiguous | busy | internal
    message: String,            // user-presentable, never contains secrets
}
// `busy` = another process holds sync.lock (CLI vs desktop).
pub struct McpClientConfig {    // one per client: claude_desktop | claude_code | codex | generic
    client, title, install_kind /* json_snippet | shell_command | toml_snippet */,
    config_path_hint: Option<String>, content: String, notes: Vec<String>,
    note_codes: Vec<McpNoteCode>, launch: McpLaunch,
}
pub struct McpLaunch { command: String /* absolute path */, args: Vec<String>, env: BTreeMap<String, String> }
```

Read-view types (`CourseSummary`, `CourseOverview`, `WeekMaterials`, `MaterialView`) live in
`pagelamp_core::views` so the MCP server can use them over a read-only store.

Additions agreed 2026-09-25 (after the first draft of this section):
- `Course.term_source: TermSource` = `user | synced | none`.
- `Course.ai_access: bool` (stored, default true; schema v1 column `courses.ai_access INTEGER NOT
  NULL DEFAULT 1`) and `CourseSummary.ai_materials` / `CourseOverview.ai_materials:
  AiMaterialsState` (computed: `readable | turned_off | withheld_by_policy`; `prohibited` policy
  wins over the switch). "Materials readable by your AI app" counts are 0 unless `readable`.
- `WeekMaterials.note_kind: Option<WeekNoteKind>` and `McpClientConfig.note_codes: Vec<McpNoteCode>`
  — stable codes next to the English text so the UI can localise.

Additions agreed 2026-09-26 (release audit):
- `McpNoteCode::RunFromTemporaryLocation` + `McpLaunch.temporary_location: Option<TemporaryLocation>`
  (`disk_image` = `/Volumes/…` on a read-only volume, `translocated` = Gatekeeper `AppTranslocation`
  copy, `appimage` = inside `$APPDIR` or a `.mount_*` dir): the `pagelamp` binary runs from a path that
  won't last. The UI shows a warning above the snippets ("move PageLamp to Applications first").
- `McpNoteCode::QuitBeforeEditing` (Claude Desktop only, replaces `restart_client_after_change` there):
  quit Claude Desktop, then edit, save, reopen — it saves over its config when it quits.
- `CourseOverview.downloadable_files` (Canvas files not yet downloaded and not blocked) and
  `MaterialView.download_blocked: Option<locked | too_large>`.
- `App::open_at_with_secrets(data_dir, Arc<dyn SecretBackend>)` for embedders and tests
  (`MemorySecrets` never touches the OS keychain).
- `pagelamp mcp` never creates a database but migrates an existing older one once at startup
  (additive migrations only).
- Versions: `[workspace.package] version` carries the pre-release (`0.1.0-beta.1`) and is what users
  see (`--version`, About, reports, MCP `serverInfo`); `tauri.conf.json` keeps the numeric part
  (MSI rejects pre-releases). `release.yml` refuses a tag that doesn't match.
- `sync_all` syncs course-creating sources (folder, Canvas) before calendar feeds, and re-links
  unmatched calendar events after each course-creating sync.

## 6. Desktop app (frontend) baseline

- `apps/desktop`: pnpm, Vite, React 19, TypeScript strict, Tailwind v4 + shadcn/ui, lucide icons,
  React Router, TanStack Query (command calls/caching) + Zustand (UI state), react-i18next (en default,
  zh-CN), Biome (lint+format), Vitest + Testing Library.
- `src/api/`: one typed interface with two implementations — `tauri` (invoke/Channel) and `mock`
  (synthetic "DEMO101 — Intro to Demo Studies" fixture), selected by `VITE_API=mock`, so the UI is
  built and tested in a plain browser before the Rust facade lands.
- `src-tauri`: Tauri 2; commands = thin wrappers over `pagelamp_app::App`; no business logic;
  no `shell:*` permissions granted to the frontend. It is a root workspace member, but `cargo`
  can only build it after `pnpm run build && pnpm run build:sidecar`, so workspace-wide checks use
  `--exclude pagelamp-desktop`.
- Screens v0.1: Welcome/onboarding (pick source: Folder+Calendar feed [recommended, shareable] /
  Canvas token [personal use notice]) · Sources & Sync (status, progress, errors, token-expiry
  hints) · Courses (code, name, current week + confidence, next deadline, AI-policy badge) ·
  Course detail (timeline evidence, this week's materials, deadlines, AI-policy editor, term
  override, hide) · Connect your AI app (cards from `mcp_client_configs`, copy buttons) ·
  Settings/About (data dir, privacy + AI disclosure). **No chat UI.**

## 7. Roadmap (decided 2026-09-25, revised 2026-09-29)

License: **Apache-2.0** (root `LICENSE`; `license.workspace = true` in every crate, `"license":
"Apache-2.0"` in package.json). Dependencies must be Apache-2.0/MIT/BSD/ISC/Zlib/Unicode-compatible —
no GPL/AGPL/SSPL/BUSL/FSL crates or npm packages (a `cargo deny` license check will enforce this in CI).

| Version | Scope |
|---|---|
| **v0.1 MCP-first** | core + sources + MCP server + CLI; desktop shell for onboarding/sources/courses/connect. `v0.1.0` released 2026-09-28 (a normal release, signed on macOS and Windows; it does not update itself) |
| v0.2 | **no separate release** (decided 2026-09-27): its items moved into v0.3 (updater, extraction worker, reminders, `.mcpb`, bilingual docs, "Past courses" and cleanup of inactive courses) or to v0.3.x (Canvas `public_url` downloads); signed releases already shipped in v0.1.0. A browser-connector approach was evaluated and is not planned |
| **v0.3.0 "full" PageLamp** | **Start (M0):** signed releases (in place since v0.1.0: macOS Developer ID + notarization, Windows Azure Artifact Signing; M0 adds the universal DMG, NSIS per-user only, Linux SHA256SUMS + attestations), Tauri updater (stable/beta channels, own manifest), `pagelamp extract-worker` (resource-limited extraction), schema v3 with a reader-compatibility marker. **Then:** embedded model access through `crates/pagelamp-llm` behind the `pagelamp-app` facade (Tauri, Swift and CLI): the student's own API key and local models (Ollama/LM Studio). The ChatGPT plan through Codex is not offered (decided 2026-10-03, `docs/design/v0.3-model-access.md` §2.3); a ChatGPT plan works with PageLamp through the student's own Codex or ChatGPT desktop over MCP. Features: study plan (deterministic dates), weekly explanation with citations, weekly progress reminders. **Course calendar and lifecycle:** deterministic weeks and phases from Canvas dates and the professor's week-numbered materials (enrollment-window terms are not used to count weeks); the AI reads the syllabus and schedule into cited date proposals the student confirms, plus a deterministic syllabus scan; finished courses grouped under "Past", and two-stage course removal (undo for 7 days, files to the Trash) that sync honours. The course AI policy gates every model call by type (`ai_gate` → `RenderedPrompt`); course materials go to a cloud model unless the student answered "not allowed" for that course (a one-time reminder otherwise); no chat UI; no tools; never proxy/resell usage or handle subscription tokens; coding-plan keys refused. See `docs/design/v0.3-model-access.md` and `docs/design/v0.3-course-calendar.md` |
| v0.3.x | Claude plan through the student's **unmodified Claude Code** (`claude -p`), switched on only after Anthropic's written confirmation and Commercial Terms acceptance; `.mcpb` generated at runtime (if not already in v0.3.0, per D30); Canvas `public_url` downloads if verified; follow-up questions on an explanation (their own gated type); opt-in automatic syllabus re-read (with the student's own API key or a local model; through a Claude plan only after Anthropic explicitly agrees); Canvas enrollments and calendar-event signals; class timetables as a week-1 anchor; midterm dates in the course calendar; calendar week topics over MCP |
| after v0.3 | branded distributions (first: a student association) via brand config — display, provider-preset overlay, updater endpoints/channel — no fork |
| later | read-only tutor loop (in-process MCP tools, ≤ 8), optional ACP host for the student's own coding agent (only where the vendor's terms allow it), Gemini native / Open Responses dialect, the Codex app-server backend once Stable (only where the vendor's terms allow it). macOS: the native SwiftUI app (`apps/macos`, on `main` since 9b12029 as a developer preview outside the release artifacts; over the same Rust core via UniFFI) ships to students once it passes its own M2 (Tauri parity) gate; same Developer ID + notarization; its own updater (Sparkle 2), not Tauri's. **Never a Swift re-implementation of core/sync/policy logic**: keep `pagelamp-app` FFI-friendly (plain serialisable types, no Tauri types in its API). Direct distribution, not the Mac App Store (sandbox vs course folders and AI-app-launched MCP) |

The v0.3 milestones, dates and decisions are in `docs/design/v0.3-plan.md`.

## 8. Collaboration rules

Contributions go through pull requests; CI must be green on Linux, macOS and Windows.
[CONTRIBUTING.md](../CONTRIBUTING.md) has the setup, the ground rules and a pre-PR checklist.

- Definition of done, at least for what you changed:
  - desktop (`apps/desktop`): `pnpm run lint` (Biome), `pnpm run typecheck` (tsc), `pnpm run test`
    (Vitest) and `pnpm run build` (Vite) green; `pnpm tauri dev` launches;
  - Rust: `cargo fmt --all --check`,
    `cargo clippy --workspace --exclude pagelamp-desktop --all-targets -- -D warnings` and
    `cargo test --workspace --exclude pagelamp-desktop` green; for the desktop crate, after
    `pnpm run build && pnpm run build:sidecar`, also
    `cargo clippy -p pagelamp-desktop --all-targets -- -D warnings` and `cargo test -p pagelamp-desktop`;
  - generated types: after a facade type changes, run `pnpm run gen:types` and commit
    `apps/desktop/src/api/generated.ts` (CI fails on drift);
  - macOS strings: `node apps/macos/scripts/gen-strings.mjs --check` passes (after changing UI text,
    run it without `--check` and commit the regenerated files);
  - no secrets: the added lines contain no tokens, keys or calendar-feed URLs.
- Commit messages: imperative subject ≤ 72 chars with an area prefix (`core:`, `desktop:`, `docs:`,
  …; the list is in CONTRIBUTING.md), body explains why.
