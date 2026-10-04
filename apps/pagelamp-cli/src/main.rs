//! `pagelamp` — sync your courses and serve them to your own AI app over MCP.
//!
//! Every command is a thin wrapper over `pagelamp_app::App` (the same facade the desktop app
//! uses); `mcp` runs the stdio MCP server. Output: human-readable text on stdout, `--json` for
//! scripts; progress, notices and logs go to stderr — stdout is reserved for the MCP protocol
//! in `pagelamp mcp`.
//!
//! Secrets (Canvas token, calendar-feed URL) are read from the terminal without echo, or from
//! stdin when piped (`echo "$URL" | pagelamp ical add`), never from command-line arguments
//! (they would end up in shell history and process lists).

mod course;
mod text;

use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use chrono::NaiveDate;
use clap::{Parser, Subcommand, ValueEnum};
use pagelamp_app::diagnostics;
use pagelamp_app::{
    App, AppError, AutoSync, McpClient, McpClientConfig, SourceSyncResult, SyncEvent, SyncPrefs,
    SyncRequest, SyncSummary,
};
use pagelamp_core::brand;
use pagelamp_core::model::{AiMaterialsState, AiPolicy, SourceKind, SourceRecord};
use serde::Serialize;

#[derive(Parser)]
#[command(name = brand::CLI_NAME, version, about = brand::TAGLINE)]
struct Cli {
    /// Print machine-readable JSON instead of text (where supported).
    #[arg(long, global = true)]
    json: bool,
    /// Show detailed diagnostics (e.g. every Canvas request) on stderr and in the log file;
    /// same as PAGELAMP_LOG=debug. Never shows tokens, feed URLs or course text.
    #[arg(short, long, global = true)]
    verbose: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Add a Canvas LMS account (personal access token; personal use only).
    #[command(subcommand)]
    Canvas(CanvasCommand),
    /// Add a local folder of course materials (<folder>/<COURSE>/...).
    #[command(subcommand)]
    Folder(FolderCommand),
    /// Add a calendar feed (e.g. Canvas Calendar → "Calendar Feed") for deadlines.
    #[command(subcommand)]
    Ical(IcalCommand),
    /// List, remove or update data sources.
    Sources {
        #[command(subcommand)]
        command: Option<SourcesCommand>,
    },
    /// Sync every source (or one) into the local database.
    Sync {
        /// Only this source id (see `sources`).
        #[arg(long)]
        source: Option<String>,
        /// Only these Canvas courses (id or code); repeatable. Folder and calendar sources
        /// always sync fully.
        #[arg(long = "course")]
        courses: Vec<String>,
        /// Download and index Canvas files (counts as viewing them in Canvas).
        #[arg(long)]
        download_files: bool,
        /// Skip Canvas files larger than this many MB.
        #[arg(long, default_value_t = 50)]
        max_file_mb: u32,
        /// Set how often the PageLamp app syncs by itself while it is open, and sync nothing
        /// now. (This command itself only ever syncs when you run it.)
        #[arg(long, value_enum, value_name = "HOW_OFTEN", conflicts_with_all = ["source", "courses", "download_files"])]
        auto: Option<AutoSyncArg>,
    },
    /// Data folder, sources, counts, last sync and the automatic sync setting.
    Status,
    /// Your courses by group (current, upcoming, past): week or phase, next deadline, AI
    /// policy and access. With -v, why. Past courses are left out (also from --json) unless
    /// you pass --past or --all.
    Courses {
        /// Only the past courses (ended, or inactive for months).
        #[arg(long, conflicts_with = "all")]
        past: bool,
        /// Every course, past ones included (use this with --json for the full list).
        #[arg(long)]
        all: bool,
    },
    /// Change a course's settings.
    #[command(subcommand)]
    Course(CourseCommand),
    /// Search your course materials.
    Search {
        /// Words to look for (any of them; best matches first).
        query: String,
        /// Only this course (code, name or id).
        #[arg(long)]
        course: Option<String>,
        /// At most this many results.
        #[arg(long, default_value_t = 10)]
        limit: u32,
    },
    /// Run the MCP server on stdin/stdout (started by your AI app, not by you).
    Mcp,
    /// Print the snippet that connects your AI app.
    McpConfig {
        /// Which app (default: all).
        client: Option<ClientArg>,
    },
    /// Print JSON Schemas of the app facade types (for the desktop frontend).
    Schema,
    /// Check the setup: version, data folder, database, keychain, sources, AI apps.
    Doctor,
    /// Print a diagnostic report to paste into a GitHub issue (read it first).
    Report {
        /// Write the report to this file instead of stdout.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Extract one file in this process, for the app (one JSON request on stdin, one JSON
    /// response on stdout; see `pagelamp_extract::worker`). Not for people.
    #[command(name = "extract-worker", hide = true)]
    ExtractWorker {
        #[arg(long)]
        protocol: u32,
    },
}

/// Counts live heap bytes, so the extraction worker can cap its memory (a no-op cap
/// elsewhere; see `pagelamp_extract::worker`).
#[global_allocator]
static ALLOCATOR: pagelamp_extract::worker::CountingAllocator =
    pagelamp_extract::worker::CountingAllocator;

#[derive(Subcommand)]
enum CanvasCommand {
    /// Add a Canvas account. The token is read without echo (or from stdin when piped).
    Add {
        /// Your school's Canvas address, e.g. https://canvas.school.edu
        #[arg(long)]
        base_url: String,
    },
}

#[derive(Subcommand)]
enum FolderCommand {
    /// Add a folder whose sub-folders are courses.
    Add {
        /// The folder that contains one sub-folder per course.
        path: PathBuf,
        /// First day of the term (YYYY-MM-DD), used when a course has no course.toml.
        #[arg(long, value_parser = parse_date)]
        term_start: Option<NaiveDate>,
        /// Name shown for this source (default: the folder's name).
        #[arg(long)]
        label: Option<String>,
    },
}

#[derive(Subcommand)]
enum IcalCommand {
    /// Add a calendar feed. The URL is read without echo (or from stdin when piped).
    Add {
        /// Name shown for this source (default: "Calendar feed").
        #[arg(long)]
        label: Option<String>,
    },
}

#[derive(Subcommand)]
enum SourcesCommand {
    /// List sources (default).
    List,
    /// Remove a source and everything synced from it.
    Remove {
        /// The source's id, as shown by `sources`.
        source_id: String,
    },
    /// Replace an expired Canvas token or a changed feed URL (read like `add`).
    UpdateSecret {
        /// The source's id, as shown by `sources`.
        source_id: String,
    },
}

#[derive(Subcommand)]
enum CourseCommand {
    /// Record the course's generative-AI policy.
    Policy {
        /// The course's code, name or id.
        course: String,
        /// What the syllabus allows (learning_aid / allowed_with_citation also work).
        policy: PolicyArg,
        /// Where the policy comes from, e.g. "syllabus §4".
        #[arg(long)]
        note: Option<String>,
    },
    /// Where a course is: week, phase, the dates used and not used, and why.
    Timeline {
        /// The course's code, name or id.
        course: String,
    },
    /// "I'm still taking this": count the course as current (by default until its term ends).
    Keep {
        /// The course's code, name or id.
        course: String,
        /// Until this day (YYYY-MM-DD).
        #[arg(long, value_parser = parse_date)]
        until: Option<NaiveDate>,
        /// Undo: the course follows its own dates again.
        #[arg(long, conflicts_with = "until")]
        clear: bool,
    },
    /// Override the term dates: first and last day of classes (or --clear to use the synced
    /// ones).
    #[command(group = clap::ArgGroup::new("dates").required(true).multiple(true).args(["start", "end", "clear"]))]
    Term {
        /// The course's code, name or id.
        course: String,
        /// First day of the term (YYYY-MM-DD).
        #[arg(long, value_parser = parse_date)]
        start: Option<NaiveDate>,
        /// Last day of the term (YYYY-MM-DD).
        #[arg(long, value_parser = parse_date)]
        end: Option<NaiveDate>,
        /// Forget your dates and use the synced ones again.
        #[arg(long, conflicts_with_all = ["start", "end"])]
        clear: bool,
    },
    /// Hide a course everywhere (including from your AI app).
    Hide {
        /// The course's code, name or id.
        course: String,
    },
    /// Show a hidden course again.
    Show {
        /// The course's code, name or id.
        course: String,
    },
    /// Let your AI app read this course's materials (on) or not (off).
    AiAccess {
        /// The course's code, name or id.
        course: String,
        /// on: your AI app may read the material text; off: titles and dates only.
        access: OnOff,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum PolicyArg {
    Unknown,
    Prohibited,
    // The stored/JSON spelling (`learning_aid`) works too.
    #[value(alias = "learning_aid")]
    LearningAid,
    #[value(alias = "allowed_with_citation")]
    AllowedWithCitation,
    Unrestricted,
}

impl From<PolicyArg> for AiPolicy {
    fn from(arg: PolicyArg) -> Self {
        match arg {
            PolicyArg::Unknown => AiPolicy::Unknown,
            PolicyArg::Prohibited => AiPolicy::Prohibited,
            PolicyArg::LearningAid => AiPolicy::LearningAid,
            PolicyArg::AllowedWithCitation => AiPolicy::AllowedWithCitation,
            PolicyArg::Unrestricted => AiPolicy::Unrestricted,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum OnOff {
    On,
    Off,
}

#[derive(Clone, Copy, ValueEnum)]
enum ClientArg {
    ClaudeDesktop,
    ClaudeCode,
    Codex,
    Generic,
}

impl ClientArg {
    fn client(self) -> McpClient {
        match self {
            ClientArg::ClaudeDesktop => McpClient::ClaudeDesktop,
            ClientArg::ClaudeCode => McpClient::ClaudeCode,
            ClientArg::Codex => McpClient::Codex,
            ClientArg::Generic => McpClient::Generic,
        }
    }
}

fn parse_date(text: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .map_err(|_| format!("expected YYYY-MM-DD, got '{text}'"))
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    // The worker is a bare child process: no logging setup, no runtime, nothing but the one
    // extraction (its environment is empty, so it must not look for a data folder either).
    if let Command::ExtractWorker { protocol } = cli.command {
        let code = pagelamp_extract::worker::serve_stdio(protocol);
        return ExitCode::from(u8::try_from(code).unwrap_or(1));
    }
    let kind = if matches!(cli.command, Command::Mcp) {
        diagnostics::ProcessKind::Mcp
    } else {
        // Rust ignores SIGPIPE, so `pagelamp courses | head` would make println! panic once
        // head exits. Like other command-line tools, just stop instead. (The MCP server keeps
        // ignoring it: a closed stdout there is the normal end of a session.)
        #[cfg(unix)]
        // SAFETY: restoring the default action of one signal before any other thread starts.
        unsafe {
            libc::signal(libc::SIGPIPE, libc::SIG_DFL);
        }
        diagnostics::ProcessKind::App
    };
    diagnostics::init(kind, cli.verbose);
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("error: could not start: {err}");
            return ExitCode::FAILURE;
        }
    };
    let result = runtime.block_on(run(cli));
    // Don't wait for leftover blocking threads: the MCP server's stdin reader only ends at
    // EOF, so after a stop signal (stdin still open) dropping the runtime would hang.
    runtime.shutdown_background();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            // Redacted like the log: stderr of `pagelamp mcp` ends up in the AI app's logs.
            eprintln!(
                "error: {}",
                pagelamp_core::diagnostics::redact(&err.to_string())
            );
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    let json = cli.json;
    match cli.command {
        Command::Mcp => {
            // Never creates anything; an older existing database is migrated once, then the
            // server reads it read-only. Its errors are fixed texts (stderr reaches the AI
            // app's own logs).
            let db = pagelamp_core::paths::db_path()?;
            pagelamp_mcp::serve_stdio(db).await
        }
        Command::Schema => print_json(&pagelamp_app::json_schema()),
        // Handled at the top of `main`, before any setup.
        Command::ExtractWorker { .. } => anyhow::bail!("the extraction worker runs on its own"),
        Command::Doctor => {
            // Works even when the database can't be opened.
            let (doctor, last_update) = match open_app() {
                Ok(app) => (app.doctor()?, app.last_migration_backup()),
                Err(_) => (diagnostics::doctor()?, None),
            };
            if json {
                return print_json(&doctor);
            }
            print_doctor(&doctor);
            if let Some(update) = last_update {
                println!(
                    "Last database update: {}",
                    diagnostics::describe_update(&update)
                );
            }
            Ok(())
        }
        Command::Report { out } => {
            let report = match open_app() {
                Ok(app) => app.diagnostic_report()?,
                Err(_) => diagnostics::diagnostic_report()?,
            };
            match out {
                Some(path) => {
                    std::fs::write(&path, &report)?;
                    if json {
                        return print_json(&serde_json::json!({ "written": path }));
                    }
                    eprintln!(
                        "Wrote {}. Read it before sharing; it contains no tokens, feed links or course names.",
                        path.display()
                    );
                }
                None if json => return print_json(&serde_json::json!({ "report": report })),
                None => print!("{report}"),
            }
            Ok(())
        }
        Command::McpConfig { client } => {
            let app = open_app()?;
            let binary = std::env::current_exe()?;
            let binary = std::fs::canonicalize(&binary).unwrap_or(binary);
            let configs: Vec<McpClientConfig> = app
                .mcp_client_configs(&binary)
                .into_iter()
                .filter(|c| client.is_none_or(|arg| arg.client() == c.client))
                .collect();
            if json {
                return print_json(&configs);
            }
            eprintln!("{}\n", text::ai_disclosure());
            for config in &configs {
                print_config(config, configs.len() > 1);
            }
            eprintln!("{}", text::try_prompt());
            Ok(())
        }
        Command::Canvas(CanvasCommand::Add { base_url }) => {
            let app = open_app()?;
            eprintln!("{}\n", text::canvas_personal_use());
            first_add_disclosure(&app)?;
            eprintln!("{}", text::CANVAS_TOKEN_HOWTO);
            let token = read_secret("Canvas access token: ")?;
            let source = app.add_canvas_source(&base_url, &token).await?;
            if json {
                return print_json(&source);
            }
            let name = source.config["account_name"]
                .as_str()
                .unwrap_or("your Canvas account");
            println!("Connected as {name}");
            eprintln!("Next: run `{} sync`.", brand::CLI_NAME);
            Ok(())
        }
        Command::Folder(FolderCommand::Add {
            path,
            term_start,
            label,
        }) => {
            let app = open_app()?;
            first_add_disclosure(&app)?;
            let source = app.add_folder_source(&path, term_start, label.as_deref())?;
            print_added(&source, json)
        }
        Command::Ical(IcalCommand::Add { label }) => {
            let app = open_app()?;
            first_add_disclosure(&app)?;
            let url = read_secret("Calendar feed URL: ")?;
            let source = app.add_ical_source(&url, label.as_deref()).await?;
            print_added(&source, json)
        }
        Command::Sources { command } => {
            let app = open_app()?;
            match command.unwrap_or(SourcesCommand::List) {
                SourcesCommand::List => {
                    let sources = app.list_sources()?;
                    if json {
                        return print_json(&sources);
                    }
                    if sources.is_empty() {
                        println!(
                            "No sources yet. Add one with `{} folder add <path>`.",
                            brand::CLI_NAME
                        );
                    }
                    for source in &sources {
                        println!("{}", source_line(source));
                    }
                    Ok(())
                }
                SourcesCommand::Remove { source_id } => {
                    app.remove_source(&source_id)?;
                    if json {
                        return print_json(&serde_json::json!({ "removed": source_id }));
                    }
                    println!("Removed {source_id}.");
                    Ok(())
                }
                SourcesCommand::UpdateSecret { source_id } => {
                    let secret = read_secret("New token or feed URL: ")?;
                    let source = app.update_source_secret(&source_id, &secret).await?;
                    if json {
                        return print_json(&source);
                    }
                    println!("Updated {}.", source.id);
                    Ok(())
                }
            }
        }
        Command::Sync {
            source,
            courses,
            download_files,
            max_file_mb,
            auto,
        } => {
            let app = open_app()?;
            if let Some(auto) = auto {
                let setting = AutoSync::from(auto);
                app.set_sync_prefs(SyncPrefs { auto_sync: setting })?;
                if json {
                    return print_json(&setting);
                }
                println!("Automatic sync: {}.", text::auto_sync(setting));
                return Ok(());
            }
            if download_files {
                eprintln!("{}", text::CANVAS_DOWNLOAD_NOTICE);
            }
            let req = SyncRequest {
                download_files,
                max_file_mb,
                only_courses: courses,
                // A sync from the command line is always one the student started.
                automatic: None,
            };
            match source {
                Some(id) => {
                    let result = app.sync_source(&id, req, print_event).await?;
                    finish_sync(&[result], json, cli.verbose)
                }
                None => {
                    let summary: SyncSummary = app.sync_all(req, print_event).await?;
                    if summary.results.is_empty() && !json {
                        println!(
                            "No sources yet. Add one with `{} folder add <path>`.",
                            brand::CLI_NAME
                        );
                        return Ok(());
                    }
                    finish_sync(&summary.results, json, cli.verbose)
                }
            }
        }
        Command::Status => {
            let app = open_app()?;
            let status = app.status()?;
            if json {
                return print_json(&status);
            }
            println!("{} {}", brand::PRODUCT_NAME, status.version);
            println!("Data folder: {}", status.data_dir);
            let c = &status.counts;
            println!(
                "Courses: {} ({} hidden) · materials: {} ({} with text) · events: {}",
                c.courses, c.hidden_courses, c.materials, c.indexed_materials, c.events
            );
            match status.last_synced_at {
                Some(at) => println!(
                    "Last sync: {}",
                    at.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M")
                ),
                None => println!("Last sync: never"),
            }
            if status.sync_in_progress {
                println!("A sync is running right now.");
            }
            println!("Automatic sync: {}.", text::auto_sync(status.auto_sync));
            for source in &status.sources {
                println!("  {}", source_line(source));
            }
            Ok(())
        }
        Command::Courses { past, all } => {
            let app = open_app()?;
            let groups = match (past, all) {
                (true, _) => course::Groups::Past,
                (_, true) => course::Groups::All,
                _ => course::Groups::Active,
            };
            course::list(&app, groups, cli.verbose, json)
        }
        Command::Course(command) => {
            let app = open_app()?;
            match command {
                CourseCommand::Timeline { course } => {
                    return course::timeline(&app, &course, json);
                }
                CourseCommand::Keep {
                    course,
                    until,
                    clear,
                } => return course::keep(&app, &course, until, clear, json),
                CourseCommand::Policy {
                    course,
                    policy,
                    note,
                } => app.set_course_policy(&course, policy.into(), note.as_deref())?,
                // clap requires --start/--end or --clear (the "dates" group); --clear is
                // the same as setting neither date.
                CourseCommand::Term {
                    course,
                    start,
                    end,
                    clear: _,
                } => app.set_course_term(&course, start, end)?,
                CourseCommand::Hide { course } => app.set_course_hidden(&course, true)?,
                CourseCommand::Show { course } => app.set_course_hidden(&course, false)?,
                CourseCommand::AiAccess { course, access } => {
                    app.set_course_ai_access(&course, matches!(access, OnOff::On))?
                }
            }
            if json {
                return print_json(&serde_json::json!({ "saved": true }));
            }
            println!("Saved.");
            Ok(())
        }
        Command::Search {
            query,
            course,
            limit,
        } => {
            let app = open_app()?;
            let hits = app.search(&query, course.as_deref(), limit)?;
            if json {
                return print_json(&hits);
            }
            if hits.is_empty() {
                println!("No matches.");
            }
            for hit in &hits {
                println!(
                    "{} · {}{} · {}",
                    hit.course_code.as_deref().unwrap_or(&hit.course_id),
                    hit.material_title,
                    hit.locator
                        .as_deref()
                        .map(|l| format!(", {l}"))
                        .unwrap_or_default(),
                    hit.snippet.replace('\n', " ")
                );
            }
            Ok(())
        }
    }
}

// ----- helpers --------------------------------------------------------------------------------

/// Read a secret: from the terminal without echo, or one line from stdin when piped.
fn read_secret(prompt: &str) -> anyhow::Result<String> {
    let secret = if std::io::stdin().is_terminal() {
        rpassword::prompt_password(prompt)?
    } else {
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)?;
        line
    };
    Ok(secret.trim().to_string())
}

/// The AI-use disclosure, printed once: when the first source is added.
fn first_add_disclosure(app: &App) -> Result<(), AppError> {
    if app.list_sources()?.is_empty() {
        eprintln!("{}\n", text::ai_disclosure());
    }
    Ok(())
}

fn print_added(source: &SourceRecord, json: bool) -> anyhow::Result<()> {
    if json {
        return print_json(source);
    }
    println!("Added {} ({}).", source.label, source.id);
    println!("Next: run `{} sync`.", brand::CLI_NAME);
    Ok(())
}

fn print_event(event: SyncEvent) {
    match event {
        SyncEvent::SourceStarted { label, .. } => eprintln!("Syncing {label}…"),
        SyncEvent::Progress {
            message,
            current,
            total,
            ..
        } => match (current, total) {
            (Some(current), Some(total)) => eprintln!("  {message} ({current}/{total})"),
            _ => eprintln!("  {message}"),
        },
        SyncEvent::Warning { message, .. } => eprintln!("  warning: {message}"),
        SyncEvent::SourceFinished { .. } => {}
    }
}

/// `PAGELAMP_LOG=debug` (or trace) already shows what `-v` would.
fn debug_logging() -> bool {
    std::env::var(pagelamp_core::paths::LOG_ENV).is_ok_and(|level| {
        matches!(
            level.trim().to_ascii_lowercase().as_str(),
            "debug" | "trace"
        )
    })
}

fn finish_sync(results: &[SourceSyncResult], json: bool, verbose: bool) -> anyhow::Result<()> {
    if json {
        print_json(&results)?;
    } else {
        for r in results {
            let seconds = (r.finished_at - r.started_at).num_milliseconds() as f64 / 1000.0;
            if r.ok {
                println!(
                    "{}: ok — {} courses, {} materials ({} newly indexed), {} events",
                    r.label, r.courses, r.materials, r.files_indexed, r.events
                );
            } else {
                println!(
                    "{}: FAILED — {}",
                    r.label,
                    r.error.as_deref().unwrap_or("unknown error")
                );
            }
            if !r.course_summaries.is_empty() {
                let width = r
                    .course_summaries
                    .iter()
                    .map(|c| c.course.chars().count())
                    .max()
                    .unwrap_or(0);
                for c in &r.course_summaries {
                    println!(
                        "  {:<width$}  {:>3} modules  {:>4} pages  {:>4} files  {:>3} events{}",
                        c.course,
                        c.modules,
                        c.pages,
                        c.files,
                        c.events,
                        match c.warnings {
                            0 => String::new(),
                            1 => "  1 warning".to_string(),
                            n => format!("  {n} warnings"),
                        }
                    );
                }
            }
            match r.requests {
                Some(requests) => println!("  {requests} requests · {seconds:.1} s"),
                None => println!("  {seconds:.1} s"),
            }
            // Only Canvas logs request details; not needed when they are already shown.
            if r.kind == SourceKind::Canvas && !verbose && !debug_logging() {
                if !r.ok {
                    println!("  run with -v for request details");
                } else if !r.warnings.is_empty() {
                    println!(
                        "  {} warning(s) above; run with -v for request details",
                        r.warnings.len()
                    );
                }
            }
        }
    }
    if results.iter().any(|r| !r.ok) {
        anyhow::bail!("some sources failed to sync");
    }
    Ok(())
}

/// The app, with this executable as its extraction worker (`pagelamp extract-worker`).
fn open_app() -> pagelamp_app::Result<App> {
    let app = App::open()?;
    app.set_extract_worker(std::env::current_exe().ok());
    Ok(app)
}

fn print_doctor(doctor: &diagnostics::DoctorReport) {
    let yes = |b: bool| if b { "yes" } else { "no" };
    println!(
        "{} {} on {} ({})",
        brand::PRODUCT_NAME,
        doctor.version,
        doctor.os,
        doctor.arch
    );
    println!("Data folder: {}", doctor.data_dir);
    println!("Logs folder: {}", doctor.logs_dir);
    match (&doctor.schema_version, &doctor.database_error) {
        (Some(v), _) => println!("Database: ok (schema {v})"),
        (None, Some(err)) => println!("Database: not readable — {err}"),
        (None, None) => println!("Database: unknown"),
    }
    match &doctor.keychain_error {
        None => println!("Keychain: available"),
        Some(err) => println!("Keychain: NOT available — {err}"),
    }
    println!(
        "Courses: {} ({} hidden) · materials: {} · events: {}",
        doctor.courses, doctor.hidden_courses, doctor.materials, doctor.events
    );
    if doctor.sources.is_empty() {
        println!(
            "Sources: none (add one with `{} folder add <path>`)",
            brand::CLI_NAME
        );
    }
    for source in &doctor.sources {
        println!(
            "Source {}: {}{}{}",
            source.kind.as_str(),
            if source.ok { "ok" } else { "not ok" },
            source
                .last_synced_at
                .map(|t| format!(
                    ", last synced {}",
                    t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M")
                ))
                .unwrap_or_default(),
            source
                .last_error_kind
                .map(|k| format!(", last error: {}", k.as_str()))
                .unwrap_or_default()
        );
    }
    println!(
        "AI apps with {} configured: Claude Desktop {} · Claude Code {} · Codex {}",
        brand::PRODUCT_NAME,
        yes(doctor.mcp_clients.claude_desktop),
        yes(doctor.mcp_clients.claude_code),
        yes(doctor.mcp_clients.codex)
    );
    println!(
        "Extraction worker: {}",
        diagnostics::describe_worker_check(&doctor.extract_worker)
    );
    if !doctor.unreadable_files.is_empty() {
        println!(
            "Unreadable files: {}",
            diagnostics::describe_unreadable(&doctor.unreadable_files)
        );
    }
    if let Some(crash) = &doctor.last_crash {
        println!(
            "Last crash: {} ({}); run `{} report` to include it in an issue",
            crash
                .time
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M"),
            crash.message,
            brand::CLI_NAME
        );
    }
}

fn print_config(config: &McpClientConfig, with_title: bool) {
    if with_title {
        println!("== {} ==", config.title);
    }
    if let Some(path) = &config.config_path_hint {
        eprintln!("Add to {path}:");
    }
    println!("{}", config.content);
    for note in &config.notes {
        eprintln!("  • {note}");
    }
    if with_title {
        println!();
    }
}

/// `sync --auto <HOW_OFTEN>`.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum AutoSyncArg {
    Off,
    Daily,
    TwiceDaily,
}

impl From<AutoSyncArg> for AutoSync {
    fn from(arg: AutoSyncArg) -> Self {
        match arg {
            AutoSyncArg::Off => AutoSync::Off,
            AutoSyncArg::Daily => AutoSync::Daily,
            AutoSyncArg::TwiceDaily => AutoSync::TwiceDaily,
        }
    }
}

fn source_line(source: &SourceRecord) -> String {
    let synced = source
        .last_synced_at
        .map(|at| {
            at.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "never synced".into());
    let error = source
        .last_error
        .as_deref()
        .map(|e| format!(" — last sync failed: {e}"))
        .unwrap_or_default();
    format!(
        "{} [{}] {} · {synced}{error}",
        source.id,
        source.kind.as_str(),
        source.label
    )
}

fn confidence(c: pagelamp_core::model::Confidence) -> &'static str {
    match c {
        pagelamp_core::model::Confidence::High => "high",
        pagelamp_core::model::Confidence::Medium => "medium",
        pagelamp_core::model::Confidence::Low => "low",
    }
}

fn ai_materials(state: AiMaterialsState) -> &'static str {
    match state {
        AiMaterialsState::Readable => "readable",
        AiMaterialsState::TurnedOff => "turned_off",
        AiMaterialsState::WithheldByPolicy => "withheld_by_policy",
    }
}

fn print_json(value: &impl Serialize) -> anyhow::Result<()> {
    let mut out = std::io::stdout().lock();
    serde_json::to_writer_pretty(&mut out, value)?;
    writeln!(out)?;
    Ok(())
}
