//! The `pagelamp` binary end to end, with a temporary PAGELAMP_HOME and synthetic course
//! folders. Commands that would store secrets in the real OS keychain are only exercised up to
//! their input validation.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

fn pagelamp(home: &Path, args: &[&str]) -> Output {
    pagelamp_with_stdin(home, args, "")
}

/// `pagelamp` with every location it could fall back to pointed into the temp dir `home`:
/// the data dir, the home directory and the AI apps' config locations (Codex, Claude
/// Desktop on Windows/Linux). Developer log settings and AppImage variables are cleared.
fn base_command(home: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pagelamp"));
    command.env("PAGELAMP_HOME", home);
    for var in [
        "HOME",
        "USERPROFILE",
        "CODEX_HOME",
        "APPDATA",
        "LOCALAPPDATA",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
    ] {
        command.env(var, home);
    }
    for var in ["RUST_LOG", "PAGELAMP_LOG", "APPIMAGE", "APPDIR"] {
        command.env_remove(var);
    }
    command
}

fn pagelamp_with_stdin(home: &Path, args: &[&str], stdin: &str) -> Output {
    let mut child = base_command(home)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn ok(output: &Output) -> String {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn json_out(output: &Output) -> Value {
    serde_json::from_str(&ok(output)).unwrap()
}

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn demo_courses(root: &Path) {
    write(
        root,
        "DEMO101 Intro to Demo Studies/Week 1/notes.md",
        "# Basics\nphotosynthesis converts light",
    );
    write(
        root,
        "DEMO101 Intro to Demo Studies/Week 2/cycle.txt",
        "the calvin cycle fixes carbon",
    );
    write(
        root,
        "DEMO202 Advanced Demo Studies/kinetics.md",
        "enzyme kinetics",
    );
}

#[test]
fn folder_add_sync_and_read_commands() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let courses = temp.path().join("Courses");
    demo_courses(&courses);

    let empty = pagelamp(&home, &["sync"]);
    assert!(ok(&empty).contains("No sources yet"));

    let added = pagelamp(
        &home,
        &[
            "folder",
            "add",
            courses.to_str().unwrap(),
            "--term-start",
            "2026-09-07",
        ],
    );
    let stdout = ok(&added);
    assert!(stdout.contains("Added Courses (folder:"), "{stdout}");
    let stderr = String::from_utf8_lossy(&added.stderr);
    assert!(
        stderr.contains("stores nothing remotely"),
        "first add shows the disclosure: {stderr}"
    );
    // …but only once.
    let again = pagelamp(&home, &["folder", "add", courses.to_str().unwrap()]);
    ok(&again);
    assert!(!String::from_utf8_lossy(&again.stderr).contains("stores nothing remotely"));

    let synced = pagelamp(&home, &["sync"]);
    assert!(ok(&synced).contains("ok — 2 courses, 3 materials"));

    let list = json_out(&pagelamp(&home, &["--json", "courses"]));
    let codes: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["course"]["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["DEMO101", "DEMO202"]);
    assert!(ok(&pagelamp(&home, &["courses"])).contains("DEMO101 Intro to Demo Studies"));

    let hits = json_out(&pagelamp(&home, &["--json", "search", "calvin"]));
    assert_eq!(hits.as_array().unwrap().len(), 1);
    assert!(
        ok(&pagelamp(
            &home,
            &["search", "calvin", "--course", "DEMO101"]
        ))
        .contains("cycle.txt")
    );

    let status = json_out(&pagelamp(&home, &["--json", "status"]));
    assert_eq!(status["counts"]["courses"], 2);
    assert!(status["last_synced_at"].is_string());
    assert!(ok(&pagelamp(&home, &["status"])).contains("Courses: 2 (0 hidden)"));
    let sources = json_out(&pagelamp(&home, &["--json", "sources"]));
    assert_eq!(sources.as_array().unwrap().len(), 1);
}

#[test]
fn course_settings_commands() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let courses = temp.path().join("Courses");
    demo_courses(&courses);
    ok(&pagelamp(
        &home,
        &["folder", "add", courses.to_str().unwrap()],
    ));
    ok(&pagelamp(&home, &["sync"]));

    ok(&pagelamp(&home, &["course", "ai-access", "DEMO101", "off"]));
    ok(&pagelamp(
        &home,
        &[
            "course",
            "policy",
            "demo202",
            "prohibited",
            "--note",
            "syllabus §2",
        ],
    ));
    // The JSON spelling of a policy is accepted as well as the kebab-case one.
    ok(&pagelamp(
        &home,
        &["course", "policy", "DEMO101", "learning_aid"],
    ));
    ok(&pagelamp(
        &home,
        &["course", "policy", "DEMO101", "allowed-with-citation"],
    ));
    ok(&pagelamp(
        &home,
        &["course", "term", "DEMO101", "--start", "2026-09-08"],
    ));
    let list = json_out(&pagelamp(&home, &["--json", "courses"]));
    assert_eq!(list[0]["course"]["ai_policy"], "allowed_with_citation");
    assert_eq!(list[0]["ai_materials"], "turned_off");
    assert_eq!(list[0]["course"]["term_source"], "user");
    assert_eq!(list[1]["ai_materials"], "withheld_by_policy");
    assert_eq!(list[1]["course"]["ai_policy_note"], "syllabus §2");

    ok(&pagelamp(&home, &["course", "term", "DEMO101", "--clear"]));
    let hidden = json_out(&pagelamp(&home, &["--json", "course", "hide", "DEMO202"]));
    assert_eq!(hidden["saved"], true);
    let list = json_out(&pagelamp(&home, &["--json", "courses"]));
    // No course.toml and no --term-start: nothing synced to fall back to.
    assert_eq!(list[0]["course"]["term_source"], "none");
    assert_eq!(list[1]["course"]["hidden"], true);
    ok(&pagelamp(&home, &["course", "show", "DEMO202"]));

    let unknown = pagelamp(&home, &["course", "hide", "NOPE999"]);
    assert!(!unknown.status.success());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("error:"));
    let no_dates = pagelamp(&home, &["course", "term", "DEMO101"]);
    assert!(!no_dates.status.success());
}

#[test]
fn mcp_config_and_schema() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let code = ok(&pagelamp(&home, &["mcp-config", "claude-code"]));
    assert!(
        code.starts_with("claude mcp add --scope user --env "),
        "{code}"
    );
    assert!(code.contains("--transport stdio pagelamp -- "), "{code}");
    assert!(code.contains(home.to_str().unwrap()));

    let desktop = ok(&pagelamp(&home, &["mcp-config", "claude-desktop"]));
    let desktop: Value = serde_json::from_str(&desktop).unwrap();
    assert_eq!(desktop["mcpServers"]["pagelamp"]["args"], json!(["mcp"]));
    assert_eq!(
        desktop["mcpServers"]["pagelamp"]["env"]["PAGELAMP_HOME"],
        json!(home.to_str().unwrap())
    );
    let all = json_out(&pagelamp(&home, &["--json", "mcp-config"]));
    assert_eq!(all.as_array().unwrap().len(), 4);

    let schema = json_out(&pagelamp(&home, &["schema"]));
    assert!(schema["$defs"]["CourseSummary"].is_object());
}

#[test]
fn ical_add_rejects_bad_urls_before_touching_anything() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let output = pagelamp_with_stdin(&home, &["ical", "add"], "not a url secret-token\n");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("not a valid calendar feed URL"), "{stderr}");
    assert!(
        !stderr.contains("secret-token"),
        "the secret is never echoed: {stderr}"
    );
    assert!(
        json_out(&pagelamp(&home, &["--json", "sources"]))
            .as_array()
            .unwrap()
            .is_empty()
    );
}

/// A running `pagelamp … mcp` with line-based JSON-RPC over its stdin/stdout.
struct McpSession {
    child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
    stdout: BufReader<std::process::ChildStdout>,
    next_id: u64,
}

impl McpSession {
    fn start(home: &Path, args: &[&str]) -> McpSession {
        let mut child = base_command(home)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        McpSession {
            child,
            stdin,
            stdout,
            next_id: 1,
        }
    }

    fn send(&mut self, message: Value) {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    }

    fn receive(&mut self) -> Value {
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        serde_json::from_str::<Value>(&line).expect("every stdout line is JSON-RPC")
    }

    /// Send a request and wait for its answer.
    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        self.receive()
    }

    fn initialize(&mut self, client_name: &str) -> Value {
        let init = self.request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": client_name, "version": "9.9"}}),
        );
        self.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        init
    }

    /// Close stdin (the server exits) and return its exit status and stderr.
    fn finish(mut self) -> (std::process::ExitStatus, String) {
        drop(self.stdin.take());
        let status = self.child.wait().unwrap();
        let mut stderr = String::new();
        std::io::Read::read_to_string(&mut self.child.stderr.take().unwrap(), &mut stderr).unwrap();
        (status, stderr)
    }
}

fn tool_text(result: &Value) -> &str {
    result["result"]["content"][0]["text"].as_str().unwrap()
}

#[test]
fn mcp_speaks_json_rpc_on_stdout_only() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let courses = temp.path().join("Courses");
    demo_courses(&courses);
    ok(&pagelamp(
        &home,
        &["folder", "add", courses.to_str().unwrap()],
    ));
    ok(&pagelamp(&home, &["sync"]));

    let mut mcp = McpSession::start(&home, &["mcp"]);
    let init = mcp.initialize("cli-test");
    assert_eq!(init["result"]["serverInfo"]["name"], "pagelamp");
    let result = mcp.request(
        "tools/call",
        json!({"name": "search_materials", "arguments": {"query": "calvin"}}),
    );
    let text = tool_text(&result);
    assert!(
        text.contains("<course_material") && text.contains("«calvin»"),
        "{text}"
    );
    let (status, stderr) = mcp.finish();
    assert!(status.success());
    assert!(stderr.is_empty(), "no log noise: {stderr}");
}

#[test]
fn canvas_add_fails_cleanly_and_stores_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let bad_url = pagelamp_with_stdin(
        &home,
        &["canvas", "add", "--base-url", "ftp://lms.example.edu"],
        "demo-not-a-real-token\n",
    );
    assert!(!bad_url.status.success());
    // A local port nobody listens on: the token check fails fast, without real network use.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let base_url = format!("http://127.0.0.1:{port}");
    let output = pagelamp_with_stdin(
        &home,
        &["canvas", "add", "--base-url", &base_url],
        "demo-not-a-real-token\n",
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{stderr}");
    assert!(!stderr.contains("panicked"), "{stderr}");
    assert!(
        stderr.contains("personal use only") && stderr.contains("Approved Integrations"),
        "{stderr}"
    );
    assert!(!stderr.contains("demo-not-a-real-token"));
    assert!(
        json_out(&pagelamp(&home, &["--json", "sources"]))
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn doctor_report_and_log_files() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let courses = temp.path().join("Courses");
    demo_courses(&courses);
    ok(&pagelamp(
        &home,
        &["folder", "add", courses.to_str().unwrap()],
    ));
    let sync = ok(&pagelamp(&home, &["sync"]));
    assert!(
        sync.contains("s\n") || sync.contains(" s"),
        "elapsed time shown: {sync}"
    );

    let doctor = ok(&pagelamp(&home, &["doctor"]));
    assert!(doctor.contains("Database: ok (schema"), "{doctor}");
    assert!(doctor.contains("Source folder: ok"), "{doctor}");
    assert!(doctor.contains("Courses: 2"), "{doctor}");
    let doctor_json = json_out(&pagelamp(&home, &["--json", "doctor"]));
    assert_eq!(doctor_json["courses"], 2);
    // The CLI is its own extraction worker, and the sync above read the files through it.
    assert!(doctor.contains("Extraction worker: ok ("), "{doctor}");
    assert_eq!(doctor_json["extract_worker"]["status"], "ok");
    assert!(doctor_json["extract_worker"]["spawn_ms"].is_u64());
    assert_eq!(doctor_json["unreadable_files"], serde_json::json!([]));

    // The app log file exists, has the sync line, and no course names at info level.
    let logs = home.join("logs");
    let log_file = std::fs::read_dir(&logs)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.file_name().unwrap().to_string_lossy().starts_with("app-"))
        .expect("app log file");
    let log = std::fs::read_to_string(log_file).unwrap();
    assert!(log.contains("folder sync ok: 2 courses"), "{log}");
    assert!(log.contains(" pid="), "{log}");
    assert!(
        !log.contains("DEMO101") && !log.contains("photosynthesis"),
        "{log}"
    );

    // The report pseudonymises course names and goes to a file on request.
    let out = temp.path().join("report.md");
    let written = pagelamp(&home, &["report", "--out", out.to_str().unwrap()]);
    ok(&written);
    let report = std::fs::read_to_string(&out).unwrap();
    assert!(
        report.starts_with("# PageLamp diagnostic report"),
        "{report}"
    );
    assert!(report.contains("- folder: ok"), "{report}");
    assert!(report.contains("- Extraction worker: ok ("), "{report}");
    assert!(
        !report.contains("DEMO101") && !report.contains("Intro to Demo Studies"),
        "{report}"
    );
    assert!(ok(&pagelamp(&home, &["report"])).contains("## Recent log"));
}

/// The whole `logs/<prefix>*.log` text.
fn log_text(home: &Path, prefix: &str) -> String {
    std::fs::read_dir(home.join("logs"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with(prefix))
        .map(|p| std::fs::read_to_string(p).unwrap())
        .collect()
}

#[test]
fn mcp_sessions_are_logged_without_arguments_or_output() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let courses = temp.path().join("Courses");
    demo_courses(&courses);
    ok(&pagelamp(
        &home,
        &["folder", "add", courses.to_str().unwrap()],
    ));
    ok(&pagelamp(&home, &["sync"]));

    let mut mcp = McpSession::start(&home, &["-v", "mcp"]);
    // A client name with a line break right before a token: neither may reach the log.
    mcp.initialize("demo-client\n1234~AbCdEfGhIjKlMnOp");
    let found = mcp.request(
        "tools/call",
        json!({"name": "search_materials", "arguments": {"query": "calvin zebra-demo-query"}}),
    );
    let text = tool_text(&found);
    let material_id = text
        .split("id=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .unwrap()
        .to_string();
    let read = mcp.request(
        "tools/call",
        json!({"name": "read_material", "arguments": {"material_id": material_id}}),
    );
    assert!(tool_text(&read).contains("fixes carbon"));
    // An unknown course in a prompt: rmcp logs error responses, with their arguments.
    let prompt = mcp.request(
        "prompts/get",
        json!({"name": "weekly_review", "arguments": {"course": "ZEBRA-COURSE-QUERY"}}),
    );
    assert!(prompt.get("error").is_some(), "{prompt}");
    let (status, _) = mcp.finish();
    assert!(status.success());

    let log = log_text(&home, "mcp-");
    let version = env!("CARGO_PKG_VERSION");
    assert!(
        log.contains(&format!("MCP server {version} started (pid ")),
        "{log}"
    );
    assert!(
        log.contains(r#"client "demo-client <redacted-token>" "9.9", protocol "2025-06-18""#),
        "{log}"
    );
    assert!(log.contains(r#"tool "search_materials": ok"#), "{log}");
    assert!(log.contains(r#"tool "read_material": ok"#), "{log}");
    assert!(log.contains("MCP server stopped ("), "{log}");
    // rmcp logs error responses (warn) and whole results (debug): none of it is kept.
    assert!(!log.contains("rmcp"), "{log}");
    for absent in [
        "AbCdEfGh",
        "zebra-demo-query",
        "calvin",
        "fixes carbon",
        "ZEBRA-COURSE-QUERY",
        "DEMO101",
    ] {
        assert!(!log.contains(absent), "{absent} in log:\n{log}");
    }
}

#[cfg(unix)]
#[test]
fn mcp_logs_being_terminated() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let mut mcp = McpSession::start(&home, &["mcp"]);
    mcp.initialize("demo-client");
    let killed = Command::new("kill")
        .args(["-TERM", &mcp.child.id().to_string()])
        .status()
        .unwrap();
    assert!(killed.success());
    // With stdin still open (a client that signals before closing the pipe): the process
    // must end by itself.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let status = loop {
        if let Some(status) = mcp.child.try_wait().unwrap() {
            break status;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "still running 10 s after SIGTERM"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    assert!(status.success(), "{status:?}");
    let log = log_text(&home, "mcp-");
    assert!(log.contains("MCP server stopped (terminated)"), "{log}");
    drop(mcp);
}

#[cfg(unix)]
#[test]
fn a_closed_pipe_ends_the_command_quietly_without_a_crash_record() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    // Like `pagelamp sources | head -c 0`: the reader is gone before anything is written
    // (closed right after spawning, long before the process has started up).
    let mut child = base_command(&home)
        .arg("sources")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let output = child.wait_with_output().unwrap();
    assert_ne!(output.status.code(), Some(101), "no panic");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("panicked"), "{stderr}");
    assert!(!home.join("logs").join("last-crash.json").exists());
}

#[test]
fn course_term_needs_dates_or_clear_and_report_speaks_json() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let output = pagelamp(&home, &["course", "term", "DEMO101"]);
    assert_eq!(output.status.code(), Some(2), "a usage error");
    let report = json_out(&pagelamp(&home, &["--json", "report"]));
    assert!(
        report["report"]
            .as_str()
            .unwrap()
            .starts_with("# PageLamp diagnostic report")
    );
}

#[test]
fn help_explains_arguments_and_removal_prints_json() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let help = ok(&pagelamp(&home, &["sync", "--help"]));
    assert!(
        help.contains("Folder and calendar sources always sync fully"),
        "{help}"
    );
    let help = ok(&pagelamp(&home, &["course", "term", "--help"]));
    assert!(help.contains("Last day of the term (YYYY-MM-DD)"), "{help}");

    let courses = temp.path().join("Courses");
    demo_courses(&courses);
    let added = json_out(&pagelamp(
        &home,
        &["--json", "folder", "add", courses.to_str().unwrap()],
    ));
    let id = added["id"].as_str().unwrap();
    let removed = json_out(&pagelamp(&home, &["--json", "sources", "remove", id]));
    assert_eq!(removed["removed"], id);
}

#[test]
fn canvas_urls_with_a_path_are_rejected_before_any_network_use() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    for url in [
        "https://lms.example.edu/courses/1",
        "https://lms.example.edu/?login=1",
    ] {
        let output = pagelamp_with_stdin(
            &home,
            &["canvas", "add", "--base-url", url],
            "demo-not-a-real-token\n",
        );
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("Enter just the address"), "{stderr}");
    }
}

/// A course that is over, inactive or not started shows its lifecycle, never a week
/// (`courses`, `course timeline`): a site whose last file, two years ago, was "Week 12" isn't
/// in week 12 now; a course Canvas marks concluded isn't in a week although its dates say so;
/// a site named for a later term hasn't started.
#[test]
fn a_finished_course_shows_its_lifecycle_not_a_week() {
    use chrono::{Datelike, Local, TimeDelta, Utc};
    use pagelamp_core::model::{
        CourseUpsert, LmsCourseInfo, MaterialKind, MaterialUpsert, SourceKind, SourceRecord,
    };
    use pagelamp_core::store::Store;

    const SOURCE: &str = "canvas:lms.example.edu";
    // Session codes ("… 20271") count only on this host.
    const UOFT: &str = "canvas:q.utoronto.ca";
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let store = Store::open(&pagelamp_core::paths::db_path_in(&home)).unwrap();
    for (id, host) in [(SOURCE, "lms.example.edu"), (UOFT, "q.utoronto.ca")] {
        store
            .upsert_source(&SourceRecord {
                id: id.into(),
                kind: SourceKind::Canvas,
                label: host.into(),
                config: json!({ "base_url": format!("https://{host}") }),
                last_synced_at: None,
                last_error: None,
                last_error_kind: None,
            })
            .unwrap();
    }
    let today = Local::now().date_naive();
    // Next year's winter session: its window (January to April) is always ahead.
    let winter = format!("DEM210H5 S LEC0101 {}1", today.year() + 1);
    // (source, external id, code, Canvas's facts, a week-numbered file and its age in days)
    let courses = [
        // No dates at all, nothing for two years.
        (
            SOURCE,
            "909",
            "OLD909",
            LmsCourseInfo::default(),
            "Week 12 notes",
            700,
        ),
        // Its dates say teaching (week 3), but Canvas marks the course concluded.
        (
            SOURCE,
            "777",
            "DONE777",
            LmsCourseInfo {
                course_start: Some(today - TimeDelta::days(20)),
                course_end: Some(today + TimeDelta::days(60)),
                concluded: Some(true),
                ..LmsCourseInfo::default()
            },
            "Week 3 notes",
            3,
        ),
        // Named for a later session, with a file left from an earlier year.
        (
            UOFT,
            "210",
            winter.as_str(),
            LmsCourseInfo::default(),
            "Week 12 review",
            200,
        ),
    ];
    for (source, external, code, lms, title, days_ago) in courses {
        let id = format!("{source}/course/{external}");
        store
            .upsert_course(&CourseUpsert {
                id: id.clone(),
                source_id: source.into(),
                external_id: external.into(),
                code: Some(code.into()),
                name: format!("{code} Demo"),
                term_start: None,
                term_end: None,
                url: None,
                syllabus_text: None,
                lms,
            })
            .unwrap();
        store
            .upsert_material(&MaterialUpsert {
                id: format!("{source}/file/{external}"),
                course_id: id,
                module_id: None,
                kind: MaterialKind::File,
                title: title.into(),
                url: None,
                local_path: None,
                mime: None,
                published_at: Some(Utc::now() - TimeDelta::days(days_ago)),
                week_hint: pagelamp_core::timeline::parse_week_hint(title),
            })
            .unwrap();
    }
    drop(store);

    let all = ok(&pagelamp(&home, &["courses", "--all"]));
    let line = |code: &str| {
        all.lines()
            .find(|line| line.contains(code))
            .unwrap_or_else(|| panic!("{code}: {all}"))
    };
    for (code, label) in [
        ("OLD909", "inactive"),
        ("DONE777", "ended"),
        ("DEM210H5", "starts "),
    ] {
        assert!(line(code).contains(label), "{}", line(code));
        assert!(!line(code).contains("week"), "{}", line(code));
    }
    let listed = json_out(&pagelamp(&home, &["--json", "courses", "--all"]));
    let states: Vec<(&str, &str, &Value)> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["course"]["external_id"].as_str().unwrap(),
                c["lifecycle"]["state"].as_str().unwrap(),
                &c["timeline"]["current_week"],
            )
        })
        .collect();
    assert_eq!(
        states,
        [
            ("210", "upcoming", &Value::Null),
            ("777", "ended", &Value::Null),
            ("909", "inactive", &Value::Null),
        ]
    );
    let timeline = json_out(&pagelamp(
        &home,
        &["--json", "course", "timeline", "OLD909"],
    ));
    assert_eq!(timeline["timeline"]["current_week"], Value::Null);
    assert_eq!(timeline["timeline"]["default_week"], Value::Null);
    assert_eq!(timeline["lifecycle"]["suggest_removal"], true);
    // The text names the lifecycle once, and says the course is suggested for removal.
    let text = ok(&pagelamp(&home, &["course", "timeline", "OLD909"]));
    assert!(text.contains("Now:       inactive (phase"), "{text}");
    assert!(text.contains("Lifecycle: inactive ("), "{text}");
    assert!(text.contains("— suggested for removal"), "{text}");
    // Concluded while its dates still say teaching: "ended", not "ended · teaching".
    let done = ok(&pagelamp(&home, &["course", "timeline", "DONE777"]));
    assert!(done.contains("Now:       ended (phase"), "{done}");
}

/// Course weeks and lifecycle groups (calendar design §7.13): `courses` shows Current and
/// Upcoming with a count of the past courses, `--past` / `--all` the rest, `course timeline`
/// the dates used and why, `course keep` "I'm still taking this".
#[test]
fn courses_are_grouped_by_lifecycle_with_timeline_and_keep() {
    use chrono::{Datelike, Local, TimeDelta};

    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let courses = temp.path().join("Courses");
    // DEMO101 started on the Monday two weeks ago (week 3); DEMO202 ended in 2024 (no files,
    // so nothing contradicts its dates).
    let today = Local::now().date_naive();
    let start = today - TimeDelta::days(i64::from(today.weekday().num_days_from_monday()) + 14);
    let end = start + TimeDelta::days(90);
    write(
        &courses,
        "DEMO101 Intro to Demo Studies/course.toml",
        &format!("term_start = {start}\nterm_end = {end}\n"),
    );
    for week in 1..=3 {
        write(
            &courses,
            &format!("DEMO101 Intro to Demo Studies/Week {week}/notes.md"),
            "# Notes\nsynthetic text",
        );
    }
    write(
        &courses,
        "DEMO202 Old Demo Studies/course.toml",
        "term_start = 2024-09-09\nterm_end = 2024-12-13\n",
    );
    ok(&pagelamp(
        &home,
        &["folder", "add", courses.to_str().unwrap()],
    ));
    ok(&pagelamp(&home, &["sync"]));

    let listed = ok(&pagelamp(&home, &["courses"]));
    assert!(listed.contains("Current (1)"), "{listed}");
    assert!(listed.contains("week 3 ("), "{listed}");
    assert!(!listed.contains("DEMO202"), "{listed}");
    assert!(listed.contains("Past courses: 1"), "{listed}");

    let past = ok(&pagelamp(&home, &["courses", "--past"]));
    assert!(
        past.contains("Past (1)") && past.contains("DEMO202"),
        "{past}"
    );
    assert!(past.contains("ended"), "{past}");
    let why = ok(&pagelamp(&home, &["-v", "courses", "--past"]));
    assert!(
        why.contains("the folder's dates ended on 2024-12-13"),
        "{why}"
    );

    let all = json_out(&pagelamp(&home, &["--json", "courses", "--all"]));
    let groups: Vec<&str> = all
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["lifecycle"]["group"].as_str().unwrap())
        .collect();
    assert_eq!(groups, ["current", "past"]);

    let timeline = ok(&pagelamp(&home, &["course", "timeline", "DEMO101"]));
    assert!(timeline.contains("from the folder's dates"), "{timeline}");
    assert!(timeline.contains("teaching"), "{timeline}");
    let timeline = json_out(&pagelamp(
        &home,
        &["--json", "course", "timeline", "DEMO101"],
    ));
    assert_eq!(timeline["timeline"]["phase"], "teaching");
    assert_eq!(timeline["timeline"]["current_week"], 3);
    assert_eq!(timeline["lifecycle"]["state"], "current");

    let kept = ok(&pagelamp(
        &home,
        &["course", "keep", "DEMO202", "--until", "2099-01-01"],
    ));
    assert!(
        kept.contains("counts as current until 2099-01-01"),
        "{kept}"
    );
    assert!(ok(&pagelamp(&home, &["courses"])).contains("DEMO202"));
    let cleared = json_out(&pagelamp(
        &home,
        &["--json", "course", "keep", "DEMO202", "--clear"],
    ));
    assert_eq!(cleared["kept_current_until"], Value::Null);
    assert_eq!(cleared["lifecycle"], "ended");
    let conflict = pagelamp(
        &home,
        &[
            "course",
            "keep",
            "DEMO202",
            "--clear",
            "--until",
            "2099-01-01",
        ],
    );
    assert!(!conflict.status.success());
}
