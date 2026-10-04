//! Canvas sync against wiremock servers (synthetic JSON only). A second "storage" server
//! plays S3/inst-fs: it only answers requests WITHOUT an Authorization header, so a leaked
//! token makes the download fail.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use chrono::{TimeDelta, Utc};
use pagelamp_core::model::*;
use pagelamp_core::source::{SyncProgress, no_progress};
use pagelamp_core::store::Store;
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

use crate::api::Api;
use crate::transport::{CanvasError, RetryPolicy, TokenTransport};
use crate::{SyncOptions, source_id, sync_with};

const TOKEN: &str = "demo-not-a-real-token";

/// Matches requests that carry no Authorization header.
struct NoAuthorization;
impl Match for NoAuthorization {
    fn matches(&self, request: &Request) -> bool {
        !request.headers.contains_key("authorization")
    }
}

struct Fixture {
    canvas: MockServer,
    storage: MockServer,
    _dir: tempfile::TempDir,
    db: PathBuf,
    files: PathBuf,
    source: String,
}

impl Fixture {
    async fn new() -> Fixture {
        let canvas = MockServer::start().await;
        let storage = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("pagelamp.db");
        let source = source_id(&canvas.uri());
        let store = Store::open(&db).unwrap();
        store
            .upsert_source(&SourceRecord {
                id: source.clone(),
                kind: SourceKind::Canvas,
                label: "Demo Canvas".into(),
                config: json!({ "base_url": canvas.uri() }),
                last_synced_at: None,
                last_error: None,
                last_error_kind: None,
            })
            .unwrap();
        Fixture {
            files: dir.path().join("files"),
            canvas,
            storage,
            _dir: dir,
            db,
            source,
        }
    }

    fn api(&self) -> Api<TokenTransport> {
        self.api_for(false)
    }

    /// The API as an automatic sync (`true`) or a sync the student started uses it.
    fn api_for(&self, automatic: bool) -> Api<TokenTransport> {
        let base = url::Url::parse(&self.canvas.uri()).unwrap();
        let retry = RetryPolicy {
            base_delay: Duration::from_millis(1),
            max_tries: 3,
            max_retry_after: Duration::from_millis(20),
        };
        Api::new(
            TokenTransport::new(base.clone(), TOKEN, retry, automatic).unwrap(),
            base,
        )
    }

    fn options(&self, download_files: bool) -> SyncOptions {
        SyncOptions {
            download_files,
            max_file_bytes: 1024,
            files_dir: self.files.clone(),
            only_courses: Vec::new(),
            extractor: Default::default(),
            automatic: false,
            user_level_only: false,
        }
    }

    async fn sync(
        &self,
        options: &SyncOptions,
    ) -> Result<crate::SyncReport, pagelamp_core::source::SourceError> {
        sync_with(&self.api(), &self.db, &self.source, options, &no_progress).await
    }

    fn store(&self) -> Store {
        Store::open(&self.db).unwrap()
    }

    /// GET `api_path` (under /api/v1) with the token → JSON.
    async fn get(&self, api_path: &str, body: Value) {
        Mock::given(method("GET"))
            .and(path(format!("/api/v1{api_path}")))
            .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&self.canvas)
            .await;
    }

    fn file(&self, id: u64, name: &str, size: u64) -> Value {
        json!({
            "id": id, "display_name": name, "filename": name, "content-type": "text/plain",
            "size": size, "url": format!("{}/files/{id}/download?download_frd=1", self.canvas.uri()),
            "updated_at": "2026-09-20T10:00:00Z", "created_at": "2026-09-01T10:00:00Z"
        })
    }

    /// Two active courses + one date-restricted stub, with modules, files, pages,
    /// an assignment (with a description that must never be stored), an announcement and a
    /// planner note.
    async fn standard(&self) {
        self.get("/users/self", json!({"id": 1, "name": "Demo Student"}))
            .await;
        self.get(
            "/courses",
            json!([
                {"id": 101, "name": "Intro to Demo Studies", "course_code": "DEMO101",
                 "time_zone": "America/Toronto", "workflow_state": "available",
                 "concluded": false,
                 "term": {"name": "Fall 2026", "start_at": "2026-09-07T04:00:00Z",
                          "end_at": "2026-12-18T05:00:00Z"},
                 "syllabus_body": "<p>Weekly quizzes and a final project.</p>"},
                {"id": 202, "name": "Advanced Demo Studies", "course_code": "DEMO202"},
                {"id": 303, "access_restricted_by_date": true}
            ]),
        )
        .await;
        self.get(
            "/courses/101/tabs",
            json!([{"id": "home"}, {"id": "modules"}, {"id": "files"}, {"id": "pages"}]),
        )
        .await;
        self.get(
            "/courses/202/tabs",
            json!([{"id": "home"}, {"id": "modules"}]),
        )
        .await;
        self.get(
            "/courses/101/modules",
            json!([
                {"id": 1, "name": "Week 1: Basics", "position": 1, "items": [
                    {"id": 11, "type": "File", "content_id": 501, "title": "Week 1 slides"},
                    {"id": 12, "type": "Page", "page_url": "week-1-overview", "title": "Overview"},
                    {"id": 13, "type": "ExternalUrl", "external_url": "https://video.example.edu/w1", "title": "Lecture video"},
                    {"id": 14, "type": "Assignment", "content_id": 9, "title": "Problem Set 1"}
                ]},
                {"id": 2, "name": "Week 2", "position": 2, "items_count": 1}
            ]),
        )
        .await;
        self.get(
            "/courses/101/modules/2/items",
            json!([{"id": 21, "type": "File", "content_id": 502, "title": "Week 2 notes"}]),
        )
        .await;
        // Files: two pages of results.
        let next = format!(
            "<{}/api/v1/courses/101/files?page=2&per_page=100>; rel=\"next\"",
            self.canvas.uri()
        );
        Mock::given(method("GET"))
            .and(path("/api/v1/courses/101/files"))
            .and(query_param("page", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                self.file(502, "notes.txt", 20),
                self.file(503, "huge.txt", 5000)
            ])))
            .with_priority(1)
            .mount(&self.canvas)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/courses/101/files"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!([self.file(501, "slides.txt", 20)]))
                    .insert_header("Link", next.as_str()),
            )
            .with_priority(2)
            .mount(&self.canvas)
            .await;
        self.get("/courses/101/pages", json!([{"page_id": 601, "url": "week-1-overview", "title": "Overview", "updated_at": "2026-09-08T10:00:00Z"}])).await;
        self.get(
            "/courses/101/pages/week-1-overview",
            json!({"page_id": 601, "url": "week-1-overview", "title": "Overview", "updated_at": "2026-09-08T10:00:00Z",
                   "html_url": format!("{}/courses/101/pages/week-1-overview", self.canvas.uri()),
                   "body": "<h1>Overview</h1><p>Photosynthesis introduction.</p>"}),
        )
        .await;
        let due = (Utc::now() + TimeDelta::days(5)).to_rfc3339();
        self.get(
            "/courses/101/assignments",
            json!([{"id": 9, "name": "Problem Set 1", "due_at": due, "html_url": "/courses/101/assignments/9",
                    "description": "<p>SECRET-INSTRUCTIONS do question 4</p>", "submission_types": ["online_upload"]}]),
        )
        .await;
        for course in ["202"] {
            self.get(&format!("/courses/{course}/modules"), json!([]))
                .await;
            self.get(&format!("/courses/{course}/assignments"), json!([]))
                .await;
        }
        Mock::given(method("GET"))
            .and(path("/api/v1/announcements"))
            .and(query_param("context_codes[]", "course_101"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"id": 701, "title": "Room change", "message": "<p>The lab moves to room 2.</p>", "posted_at": "2026-09-22T12:00:00Z"}
            ])))
            .mount(&self.canvas)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/announcements"))
            .and(query_param("context_codes[]", "course_202"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&self.canvas)
            .await;
        self.get(
            "/planner/items",
            json!([{"plannable_id": 801, "plannable_type": "planner_note", "plannable_date": (Utc::now() + TimeDelta::days(2)).to_rfc3339(),
                    "plannable": {"title": "Buy a lab coat"}}]),
        )
        .await;
    }

    /// File downloads: Canvas redirects to the storage server, which serves only
    /// unauthenticated requests.
    async fn downloads(&self) {
        for (id, text) in [(501, "chlorophyll slides"), (502, "stomata notes")] {
            Mock::given(method("GET"))
                .and(path(format!("/files/{id}/download")))
                .respond_with(ResponseTemplate::new(302).insert_header(
                    "Location",
                    format!("{}/blob/{id}?sig=demo", self.storage.uri()).as_str(),
                ))
                .mount(&self.canvas)
                .await;
            Mock::given(method("GET"))
                .and(path(format!("/blob/{id}")))
                .and(NoAuthorization)
                .respond_with(ResponseTemplate::new(200).set_body_string(text))
                .expect(1)
                .mount(&self.storage)
                .await;
        }
    }
}

fn material<'a>(materials: &'a [Material], id: &str) -> &'a Material {
    materials
        .iter()
        .find(|m| m.id.ends_with(id))
        .unwrap_or_else(|| panic!("no material {id}"))
}

fn all_events(store: &Store) -> Vec<Event> {
    store
        .list_events(
            chrono::DateTime::<Utc>::MIN_UTC,
            chrono::DateTime::<Utc>::MAX_UTC,
            None,
        )
        .unwrap()
}

#[tokio::test]
async fn full_sync_without_downloads() {
    let f = Fixture::new().await;
    f.standard().await;
    // No file may be downloaded unless asked (downloads count as views in Canvas).
    Mock::given(method("GET"))
        .and(path("/files/501/download"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&f.canvas)
        .await;

    let report = f.sync(&f.options(false)).await.unwrap();
    assert_eq!(report.courses, 2);
    assert_eq!(report.files_downloaded, 0);

    let store = f.store();
    let courses = store.list_courses(true).unwrap();
    let codes: Vec<_> = courses.iter().filter_map(|c| c.code.clone()).collect();
    assert_eq!(codes, ["DEMO101", "DEMO202"]);
    let demo = &courses[0];
    assert_eq!(demo.term_start.unwrap().to_string(), "2026-09-07");
    assert!(demo.url.as_deref().unwrap().ends_with("/courses/101"));
    // The LMS facts are stored raw, in the course's time zone (calendar design §5 S2/S3).
    let lms = store.course_term_data(&demo.id).unwrap().unwrap().lms;
    assert_eq!(lms.term_name.as_deref(), Some("Fall 2026"));
    assert_eq!(lms.term_start.unwrap().to_string(), "2026-09-07");
    assert_eq!(lms.term_end.unwrap().to_string(), "2026-12-18");
    assert_eq!(lms.time_zone.as_deref(), Some("America/Toronto"));
    assert_eq!(lms.workflow_state.as_deref(), Some("available"));
    assert_eq!(lms.concluded, Some(false));
    assert_eq!(lms.access_restricted, Some(false));
    let other = store.course_term_data(&courses[1].id).unwrap().unwrap().lms;
    assert_eq!((other.concluded, other.time_zone), (None, None));

    let modules = store.list_modules(&demo.id).unwrap();
    let names: Vec<(String, Option<u32>)> = modules
        .iter()
        .map(|m| (m.name.clone(), m.week_hint))
        .collect();
    assert_eq!(
        names,
        [
            ("Week 1: Basics".to_string(), Some(1)),
            ("Week 2".to_string(), Some(2))
        ]
    );

    let materials = store.list_materials(&demo.id).unwrap();
    let slides = material(&materials, "/file/501");
    assert_eq!(slides.title, "Week 1 slides");
    assert_eq!(slides.week_hint, Some(1));
    assert_eq!(slides.text_status, TextStatus::NotDownloaded);
    assert_eq!(
        material(&materials, "/file/502").week_hint,
        Some(2),
        "module items fallback endpoint"
    );
    assert_eq!(
        material(&materials, "/file/503").module_id,
        None,
        "second files page followed"
    );
    let link = material(&materials, "/link/13");
    assert_eq!(
        (link.kind, link.text_status),
        (MaterialKind::ExternalLink, TextStatus::Unsupported)
    );
    assert_eq!(
        material(&materials, "/page/601").text_status,
        TextStatus::Ok
    );
    assert_eq!(
        material(&materials, "/announcement/701").kind,
        MaterialKind::Announcement
    );
    assert_eq!(
        material(&materials, "/syllabus/101").kind,
        MaterialKind::Syllabus
    );
    assert_eq!(store.search("photosynthesis", None, 5).unwrap().len(), 1);
    assert_eq!(store.search("room", None, 5).unwrap().len(), 1);
    assert!(
        store
            .course_syllabus_text(&demo.id)
            .unwrap()
            .unwrap()
            .contains("Weekly quizzes")
    );

    let events = all_events(&store);
    let titles: Vec<_> = events.iter().map(|e| e.title.as_str()).collect();
    assert!(
        titles.contains(&"Problem Set 1") && titles.contains(&"Buy a lab coat"),
        "{titles:?}"
    );
    let ps1 = events.iter().find(|e| e.title == "Problem Set 1").unwrap();
    assert!(
        ps1.url
            .as_deref()
            .unwrap()
            .ends_with("/courses/101/assignments/9")
    );

    // Rule 4: the assignment description exists nowhere in the database.
    let conn = store.conn();
    let leaked: i64 = conn
        .query_row(
            "SELECT (SELECT COUNT(*) FROM chunks WHERE text LIKE '%SECRET-INSTRUCTIONS%')
                  + (SELECT COUNT(*) FROM events WHERE title LIKE '%SECRET%')
                  + (SELECT COUNT(*) FROM materials WHERE title LIKE '%SECRET%')
                  + (SELECT COUNT(*) FROM courses WHERE syllabus_text LIKE '%SECRET%')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(leaked, 0);
}

#[tokio::test]
async fn downloads_follow_redirects_without_leaking_the_token() {
    let f = Fixture::new().await;
    f.standard().await;
    f.downloads().await;
    let warnings = Mutex::new(Vec::new());
    let options = f.options(true);
    let report = sync_with(&f.api(), &f.db, &f.source, &options, &|p| {
        if let SyncProgress::Warning(w) = p {
            warnings.lock().unwrap().push(w);
        }
    })
    .await
    .unwrap();
    assert_eq!(report.files_downloaded, 2);
    let warnings = warnings.into_inner().unwrap();
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("huge.txt") && w.contains("larger than")),
        "{warnings:?}"
    );

    let store = f.store();
    assert_eq!(store.search("chlorophyll", None, 5).unwrap().len(), 1);
    let materials = store
        .list_materials(&format!("{}/course/101", f.source))
        .unwrap();
    let slides = material(&materials, "/file/501");
    assert_eq!(slides.text_status, TextStatus::Ok);
    let local = PathBuf::from(slides.local_path.as_deref().unwrap());
    assert!(local.starts_with(&f.files) && local.is_file());
    assert_eq!(
        material(&materials, "/file/503").text_status,
        TextStatus::NotDownloaded
    );

    // A second sync neither re-downloads (storage mocks expect exactly one hit each) nor
    // forgets the text when downloads are off.
    f.sync(&f.options(true)).await.unwrap();
    f.sync(&f.options(false)).await.unwrap();
    let materials = f
        .store()
        .list_materials(&format!("{}/course/101", f.source))
        .unwrap();
    assert_eq!(
        material(&materials, "/file/501").text_status,
        TextStatus::Ok
    );
}

#[tokio::test]
async fn invalid_token_aborts_and_never_leaks() {
    let f = Fixture::new().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/users/self"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({"errors": [{"message": "Invalid access token."}]})),
        )
        .mount(&f.canvas)
        .await;
    let err = f.sync(&f.options(false)).await.unwrap_err();
    assert_eq!(err.kind, SourceErrorKind::AuthExpiredOrRevoked);
    assert!(!err.message.contains(TOKEN));
    let requests = f.canvas.received_requests().await.unwrap();
    assert_eq!(
        requests.len(),
        1,
        "nothing else is requested after a rejected token"
    );
}

#[tokio::test]
async fn a_canvas_that_moved_is_not_found_not_an_internal_error() {
    let f = Fixture::new().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/users/self"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("Location", "https://sso.example.edu/login"),
        )
        .mount(&f.canvas)
        .await;
    let err = f.sync(&f.options(false)).await.unwrap_err();
    assert_eq!(err.kind, SourceErrorKind::NotFound, "{}", err.message);
    assert_eq!(f.canvas.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn throttling_backs_off_then_succeeds_or_gives_up() {
    let f = Fixture::new().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/users/self"))
        .respond_with(
            ResponseTemplate::new(403).set_body_string("403 Forbidden (Rate Limit Exceeded)"),
        )
        .up_to_n_times(2)
        .with_priority(1)
        .mount(&f.canvas)
        .await;
    f.get("/users/self", json!({"id": 1, "name": "Demo Student"}))
        .await;
    let user: crate::json::User = f
        .api()
        .get_one(crate::endpoint::Endpoint::UsersSelf)
        .await
        .unwrap();
    assert_eq!(user.name.as_deref(), Some("Demo Student"));

    let g = Fixture::new().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/users/self"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&g.canvas)
        .await;
    let err = g.sync(&g.options(false)).await.unwrap_err();
    assert_eq!(err.kind, SourceErrorKind::RateLimited);
    assert_eq!(
        g.canvas.received_requests().await.unwrap().len(),
        3,
        "max_tries"
    );
}

/// A throttled answer's `Retry-After` is waited for, up to the policy's limit, and a sync
/// PageLamp started by itself says so in its User-Agent.
#[tokio::test]
async fn retry_after_is_honoured_up_to_a_limit_and_an_automatic_sync_names_itself() {
    let agents = |requests: Vec<Request>| -> Vec<String> {
        requests
            .iter()
            .map(|r| r.headers["user-agent"].to_str().unwrap().to_string())
            .collect()
    };

    // Canvas asks for an hour: more than a sync waits inside itself (the wait couldn't be
    // stopped). The call ends as throttled at once, after that one request.
    let f = Fixture::new().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/users/self"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "3600"))
        .mount(&f.canvas)
        .await;
    let started = std::time::Instant::now();
    let throttled = f
        .api_for(true)
        .get_one::<crate::json::User>(crate::endpoint::Endpoint::UsersSelf)
        .await
        .unwrap_err();
    assert!(matches!(throttled, CanvasError::RateLimited));
    assert!(started.elapsed() < Duration::from_secs(5));
    let automatic = agents(f.canvas.received_requests().await.unwrap());
    assert_eq!(automatic.len(), 1, "no second try");
    assert!(
        automatic[0].ends_with("(read-only; automatic sync)"),
        "{automatic:?}"
    );

    // One second is inside the limit: it is waited (not the 1 ms backoff), then the call
    // succeeds. A sync the student started keeps the plain User-Agent.
    let g = Fixture::new().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/users/self"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "1"))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&g.canvas)
        .await;
    g.get("/users/self", json!({"id": 1, "name": "Demo Student"}))
        .await;
    let base = url::Url::parse(&g.canvas.uri()).unwrap();
    let retry = RetryPolicy {
        base_delay: Duration::from_millis(1),
        max_tries: 3,
        max_retry_after: Duration::from_secs(5),
    };
    let api = Api::new(
        TokenTransport::new(base.clone(), TOKEN, retry, false).unwrap(),
        base,
    );
    let started = std::time::Instant::now();
    let user: crate::json::User = api
        .get_one(crate::endpoint::Endpoint::UsersSelf)
        .await
        .unwrap();
    assert_eq!(user.name.as_deref(), Some("Demo Student"));
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_millis(900) && waited < Duration::from_secs(10),
        "{waited:?}"
    );
    let manual = agents(g.canvas.received_requests().await.unwrap());
    assert_eq!(manual.len(), 2);
    assert!(
        manual.iter().all(|agent| agent.ends_with("(read-only)")),
        "{manual:?}"
    );
}

#[tokio::test]
async fn forbidden_areas_warn_and_prevent_pruning() {
    let f = Fixture::new().await;
    f.standard().await;
    f.sync(&f.options(false)).await.unwrap();
    let course = format!("{}/course/101", f.source);
    let before = f.store().list_materials(&course).unwrap().len();

    // Now the files list is forbidden and a module page vanished from the modules list.
    f.canvas.reset().await;
    f.standard().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/courses/101/files"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({"status": "unauthorized"})))
        .with_priority(1)
        .mount(&f.canvas)
        .await;
    let report = f.sync(&f.options(false)).await.unwrap();
    assert!(
        report.warnings.iter().any(|w| w.contains("files list")),
        "{:?}",
        report.warnings
    );
    assert_eq!(
        f.store().list_materials(&course).unwrap().len(),
        before,
        "nothing pruned"
    );
}

#[tokio::test]
async fn foreign_next_links_are_never_followed() {
    let f = Fixture::new().await;
    f.get("/users/self", json!({"id": 1, "name": "Demo Student"}))
        .await;
    let evil = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(0)
        .mount(&evil)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/courses"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([]))
                .insert_header(
                    "Link",
                    format!("<{}/api/v1/courses?page=2>; rel=\"next\"", evil.uri()).as_str(),
                ),
        )
        .mount(&f.canvas)
        .await;
    f.get("/planner/items", json!([])).await;
    // An earlier course must survive: the truncated listing prunes nothing.
    f.store()
        .upsert_course(&CourseUpsert {
            id: format!("{}/course/101", f.source),
            source_id: f.source.clone(),
            external_id: "101".into(),
            code: Some("DEMO101".into()),
            name: "Intro to Demo Studies".into(),
            term_start: None,
            term_end: None,
            url: None,
            syllabus_text: None,
            lms: Default::default(),
        })
        .unwrap();
    f.sync(&f.options(false)).await.unwrap();
    assert_eq!(f.store().list_courses(true).unwrap().len(), 1);
}

#[tokio::test]
async fn ended_courses_are_kept_and_only_courses_limits_the_sync() {
    let f = Fixture::new().await;
    f.standard().await;
    f.sync(&f.options(false)).await.unwrap();
    assert_eq!(f.store().list_courses(true).unwrap().len(), 2);

    // Syncing only DEMO202 keeps DEMO101's data and events.
    let mut only = f.options(false);
    only.only_courses = vec!["demo202".into()];
    f.sync(&only).await.unwrap();
    let store = f.store();
    assert_eq!(store.list_courses(true).unwrap().len(), 2);
    assert!(
        all_events(&store)
            .iter()
            .any(|e| e.title == "Problem Set 1")
    );

    // DEMO202 leaves Canvas's active list (term over) → kept with its data, marked inactive.
    let c202 = format!("{}/course/202", f.source);
    f.store()
        .set_course_policy(&c202, AiPolicy::LearningAid, None)
        .unwrap();
    f.canvas.reset().await;
    f.standard().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/courses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 101, "name": "Intro to Demo Studies", "course_code": "DEMO101"}
        ])))
        .with_priority(1)
        .mount(&f.canvas)
        .await;
    f.sync(&f.options(false)).await.unwrap();
    let store = f.store();
    let old = store
        .get_course(&c202)
        .unwrap()
        .expect("never deleted by a Canvas sync");
    assert!(!old.enrollment_active);
    assert_eq!(old.ai_policy, AiPolicy::LearningAid);
    assert!(
        store
            .get_course(&format!("{}/course/101", f.source))
            .unwrap()
            .unwrap()
            .enrollment_active
    );
}
#[tokio::test]
async fn every_request_is_an_allow_listed_get() {
    let f = Fixture::new().await;
    f.standard().await;
    f.downloads().await;
    f.sync(&f.options(true)).await.unwrap();
    let allowed = regex::Regex::new(
        r"^/api/v1/(users/self|courses|courses/\d+/(tabs|modules|modules/\d+/items|files|files/\d+|pages|pages/[^/]+|assignments)|announcements|planner/items)$|^/files/\d+/download$",
    )
    .unwrap();
    for request in f.canvas.received_requests().await.unwrap() {
        assert_eq!(request.method.as_str(), "GET");
        assert!(
            allowed.is_match(request.url.path()),
            "not allow-listed: {}",
            request.url
        );
        assert_eq!(
            request
                .headers
                .get("authorization")
                .and_then(|v| v.to_str().ok()),
            Some(format!("Bearer {TOKEN}").as_str())
        );
    }
    for request in f.storage.received_requests().await.unwrap() {
        assert!(!request.headers.contains_key("authorization"));
    }
}

/// A sync nobody is at the app for: Canvas may record a request for a course's modules, files,
/// pages or assignments as the student's activity in that course, so none is made.
#[tokio::test]
async fn a_user_level_sync_asks_for_no_course_and_removes_only_what_it_read() {
    let f = Fixture::new().await;
    f.standard().await;
    // Beside Problem Set 1: a graded quiz, and a report the planner won't list.
    let day = |days: i64| (Utc::now() + TimeDelta::days(days)).to_rfc3339();
    Mock::given(method("GET"))
        .and(path("/api/v1/courses/101/assignments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 9, "name": "Problem Set 1", "due_at": day(5),
             "html_url": "/courses/101/assignments/9", "submission_types": ["online_upload"]},
            {"id": 11, "name": "Quiz A", "due_at": day(7),
             "html_url": "/courses/101/assignments/11", "submission_types": ["online_quiz"]},
            {"id": 13, "name": "Lab report", "due_at": day(20),
             "html_url": "/courses/101/assignments/13", "submission_types": ["online_upload"]}
        ])))
        .with_priority(1)
        .mount(&f.canvas)
        .await;
    let full = f.sync(&f.options(false)).await.unwrap();
    let demo = course101(&f);
    assert_eq!(full.read_courses.len(), 2);
    assert!(full.read_courses.contains(&demo));
    let ids =
        |materials: Vec<Material>| -> Vec<String> { materials.into_iter().map(|m| m.id).collect() };
    let materials_before = ids(f.store().list_materials(&demo).unwrap());
    let modules_before = f.store().list_modules(&demo).unwrap().len();
    let before = all_events(&f.store());
    let was = |title: &str| before.iter().find(|e| e.title == title).unwrap().clone();
    assert_eq!(was("Quiz A").kind, EventKind::QuizDue);

    // Canvas moves on: a third course, a new announcement, a changed syllabus, Problem Set 1
    // and Quiz A moved, a new assignment and a graded quiz. Only the user-level endpoints
    // answer.
    f.canvas.reset().await;
    f.get("/users/self", json!({"id": 1, "name": "Demo Student"}))
        .await;
    f.get(
        "/courses",
        json!([
            {"id": 101, "name": "Intro to Demo Studies", "course_code": "DEMO101",
             "syllabus_body": "<p>Weekly quizzes, a midterm and a final project.</p>"},
            {"id": 202, "name": "Advanced Demo Studies", "course_code": "DEMO202"},
            {"id": 404, "name": "Demo Seminar", "course_code": "DEMO404"}
        ]),
    )
    .await;
    for (course, body) in [
        (
            "101",
            json!([
                {"id": 701, "title": "Room change", "message": "<p>The lab moves to room 2.</p>", "posted_at": "2026-09-22T12:00:00Z"},
                {"id": 702, "title": "Midterm date", "message": "<p>The midterm is on a Thursday.</p>", "posted_at": "2026-09-29T12:00:00Z"}
            ]),
        ),
        ("202", json!([])),
        (
            "404",
            json!([{"id": 711, "title": "Welcome", "message": "<p>Welcome to the seminar.</p>", "posted_at": "2026-09-30T12:00:00Z"}]),
        ),
    ] {
        Mock::given(method("GET"))
            .and(path("/api/v1/announcements"))
            .and(query_param(
                "context_codes[]",
                format!("course_{course}").as_str(),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&f.canvas)
            .await;
    }
    let moved = Utc::now() + TimeDelta::days(9);
    f.get(
        "/planner/items",
        json!([
            {"plannable_id": 801, "plannable_type": "planner_note", "plannable_date": day(2),
             "plannable": {"title": "Buy a lab coat"}},
            // Handed in: the planner links to the student's submission and says more than
            // PageLamp may keep.
            {"plannable_id": 9, "plannable_type": "assignment", "course_id": 101,
             "plannable_date": moved.to_rfc3339(),
             "plannable": {"title": "Problem Set 1", "points_possible": 10,
                           "details": "<p>SECRET-DETAILS do question 4</p>",
                           "description": "<p>SECRET-DESCRIPTION</p>"},
             "submissions": {"submitted": true, "graded": true, "feedback": {"comment": "SECRET-FEEDBACK"}},
             "html_url": "/courses/101/assignments/9/submissions/1"},
            // A quiz the planner happens to name as an assignment: it stays a quiz.
            {"plannable_id": 11, "plannable_type": "assignment", "course_id": 101,
             "plannable_date": day(8), "plannable": {"title": "Quiz A"},
             "html_url": "/courses/101/assignments/11/submissions/1"},
            // New, and handed in already.
            {"plannable_id": 10, "plannable_type": "assignment", "course_id": 101,
             "plannable_date": day(12), "plannable": {"title": "Problem Set 2"},
             "html_url": "/courses/101/assignments/10/submissions/1"},
            {"plannable_id": 44, "plannable_type": "quiz", "course_id": 404,
             "plannable_date": day(6),
             "plannable": {"title": "Quiz 1", "assignment_id": 12, "points_possible": 5,
                           "details": "SECRET-QUIZ-TEXT"},
             "html_url": "/courses/404/quizzes/44"},
            // A graded discussion no full sync has seen: its date may be the day it was posted.
            {"plannable_id": 51, "plannable_type": "discussion_topic", "course_id": 101,
             "plannable_date": day(-1),
             "plannable": {"title": "Week 3 discussion", "assignment_id": 14}}
        ]),
    )
    .await;

    let light = SyncOptions {
        automatic: true,
        user_level_only: true,
        ..f.options(false)
    };
    let report = sync_with(&f.api_for(true), &f.db, &f.source, &light, &no_progress)
        .await
        .unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    assert!(report.user_level_read);

    // No request has a course in its path.
    let requests = f.canvas.received_requests().await.unwrap();
    let mut paths: Vec<&str> = requests.iter().map(|r| r.url.path()).collect();
    paths.sort_unstable();
    paths.dedup();
    assert_eq!(
        paths,
        [
            "/api/v1/announcements",
            "/api/v1/courses",
            "/api/v1/planner/items",
            "/api/v1/users/self"
        ]
    );

    // The course it met for the first time is reported, and nothing counts as read in full.
    let seminar = format!("{}/course/404", f.source);
    assert_eq!(report.new_courses, std::slice::from_ref(&seminar));
    assert!(report.read_courses.is_empty());
    let store = f.store();
    assert!(store.list_modules(&seminar).unwrap().is_empty());
    assert_eq!(
        ids(store.list_materials(&seminar).unwrap()),
        [format!("{}/announcement/711", f.source)]
    );

    // Everything the full sync read is still there, with the new announcement.
    let after = ids(store.list_materials(&demo).unwrap());
    for id in &materials_before {
        assert!(after.contains(id), "{id} was removed");
    }
    assert_eq!(after.len(), materials_before.len() + 1);
    assert!(has_material(&f, "/announcement/702"));
    assert_eq!(store.list_modules(&demo).unwrap().len(), modules_before);
    assert_eq!(store.search("photosynthesis", None, 5).unwrap().len(), 1);
    assert_eq!(store.search("thursday", None, 5).unwrap().len(), 1);
    assert!(
        store
            .course_syllabus_text(&demo)
            .unwrap()
            .unwrap()
            .contains("a midterm")
    );

    // Due dates follow the planner. A known one moves and keeps its kind and its link.
    let events = all_events(&store);
    let event = |title: &str| events.iter().find(|e| e.title == title).unwrap();
    let ps1 = event("Problem Set 1");
    assert_eq!(
        (&ps1.id, &ps1.url),
        (&was("Problem Set 1").id, &was("Problem Set 1").url)
    );
    assert_eq!(
        ps1.due_at.unwrap().timestamp(),
        moved.timestamp(),
        "the date moved"
    );
    let quiz_a = event("Quiz A");
    assert_eq!(
        (quiz_a.kind, &quiz_a.url),
        (EventKind::QuizDue, &was("Quiz A").url)
    );
    assert!(quiz_a.due_at.unwrap() > was("Quiz A").due_at.unwrap());
    // One the planner doesn't list is never removed or changed.
    let report_due = event("Lab report");
    assert_eq!(
        (&report_due.id, report_due.due_at, &report_due.url),
        (
            &was("Lab report").id,
            was("Lab report").due_at,
            &was("Lab report").url
        )
    );
    // New ones are added under the assignment's id, with the assignment's link (never the
    // student's submission page).
    let ps2 = event("Problem Set 2");
    assert_eq!(ps2.id, format!("{}/assignment/10", f.source));
    assert!(
        ps2.url
            .as_deref()
            .is_some_and(|url| url.ends_with("/courses/101/assignments/10"))
    );
    let quiz = event("Quiz 1");
    assert_eq!(quiz.id, format!("{}/assignment/12", f.source));
    assert_eq!(
        (quiz.kind, quiz.course_id.as_deref()),
        (EventKind::QuizDue, Some(seminar.as_str()))
    );
    assert!(
        quiz.url
            .as_deref()
            .is_some_and(|url| url.ends_with("/courses/404/assignments/12"))
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e.title == "Buy a lab coat")
            .count(),
        1
    );
    // The discussion is on the planner as before; no assignment event is made up for it.
    assert!(
        events
            .iter()
            .all(|e| e.id != format!("{}/assignment/14", f.source))
    );

    // Rule 4 holds in a light run: an assignment or quiz is its name, due date and link.
    // Nothing else the planner says about it (details, points, submission, feedback) is
    // stored anywhere, in any table or in the files of the database.
    for due in [ps1, quiz_a, ps2, quiz] {
        let extra = (due.starts_at, due.ends_at, due.course_hint.as_deref());
        assert_eq!(extra, (None, None, None), "{}", due.title);
        assert!(due.due_at.is_some() && !due.title.is_empty());
        assert!(!due.url.as_deref().unwrap().contains("submissions"));
    }
    for entry in std::fs::read_dir(f.db.parent().unwrap()).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            let bytes = std::fs::read(&path).unwrap();
            let text = String::from_utf8_lossy(&bytes);
            assert!(!text.contains("SECRET-"), "found in {}", path.display());
        }
    }

    // Like any sync, a light run removes what it read completely and no longer finds: an
    // announcement that left the listing, a syllabus that became empty, a planner note that is
    // gone. What it didn't read stays: files, pages, links, modules and assignment dates.
    let events_before = all_events(&store).len();
    drop(store);
    f.canvas.reset().await;
    f.get("/users/self", json!({"id": 1, "name": "Demo Student"}))
        .await;
    f.get(
        "/courses",
        json!([
            {"id": 101, "name": "Intro to Demo Studies", "course_code": "DEMO101"},
            {"id": 202, "name": "Advanced Demo Studies", "course_code": "DEMO202"},
            {"id": 404, "name": "Demo Seminar", "course_code": "DEMO404"}
        ]),
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/announcements"))
        .and(query_param("context_codes[]", "course_101"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 702, "title": "Midterm date", "message": "<p>The midterm is on a Thursday.</p>", "posted_at": "2026-09-29T12:00:00Z"}
        ])))
        .with_priority(1)
        .mount(&f.canvas)
        .await;
    f.get("/announcements", json!([])).await;
    f.get("/planner/items", json!([])).await;
    let report = sync_with(&f.api_for(true), &f.db, &f.source, &light, &no_progress)
        .await
        .unwrap();
    assert!(report.user_level_read && report.warnings.is_empty());
    let store = f.store();
    assert!(
        !has_material(&f, "/announcement/701"),
        "it left the listing"
    );
    assert!(has_material(&f, "/announcement/702"));
    assert!(
        !has_material(&f, "/syllabus/101"),
        "the syllabus is empty now"
    );
    assert_eq!(store.course_syllabus_text(&demo).unwrap(), None);
    let events = all_events(&store);
    // The planner's own items went (the note, and the discussion it listed as a to-do).
    for gone in ["Buy a lab coat", "Week 3 discussion"] {
        assert!(events.iter().all(|e| e.title != gone), "{gone}");
    }
    assert_eq!(events.len(), events_before - 2, "and nothing else");
    for kept in [
        "/file/501",
        "/file/502",
        "/file/503",
        "/page/601",
        "/link/13",
    ] {
        assert!(has_material(&f, kept), "{kept}");
    }
    assert_eq!(store.list_modules(&demo).unwrap().len(), modules_before);
    for title in [
        "Problem Set 1",
        "Quiz A",
        "Lab report",
        "Problem Set 2",
        "Quiz 1",
    ] {
        assert!(events.iter().any(|e| e.title == title), "{title}");
    }
    drop(store);

    // The next full sync reads the new course, and takes the dates from the assignments again.
    f.canvas.reset().await;
    f.standard().await;
    for (at, body) in [
        (
            "/api/v1/courses",
            json!([
                {"id": 101, "name": "Intro to Demo Studies", "course_code": "DEMO101"},
                {"id": 404, "name": "Demo Seminar", "course_code": "DEMO404"}
            ]),
        ),
        (
            "/api/v1/courses/404/modules",
            json!([{"id": 4, "name": "Week 1", "items": [
                {"id": 41, "type": "ExternalUrl", "external_url": "https://video.example.edu/s1", "title": "Reading"}]}]),
        ),
    ] {
        Mock::given(method("GET"))
            .and(path(at))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .with_priority(1)
            .mount(&f.canvas)
            .await;
    }
    // (Its tabs and assignments answer 404: nothing there for this student.)
    let full = f.sync(&f.options(false)).await.unwrap();
    assert!(
        full.read_courses.contains(&seminar),
        "{:?}",
        full.read_courses
    );
    assert_eq!(f.store().list_modules(&seminar).unwrap().len(), 1);
    let events = all_events(&f.store());
    let ps1 = events.iter().find(|e| e.title == "Problem Set 1").unwrap();
    assert!(ps1.due_at.unwrap() < moved - TimeDelta::days(1));
    assert!(
        events.iter().all(|e| e.title != "Problem Set 2"),
        "an assignment Canvas no longer lists goes at a full sync"
    );
}

/// A light run that can't read the planner says so (the facade then doesn't count it), and a
/// run that fails after it stored a new course has already recorded that the course waits.
#[tokio::test]
async fn a_user_level_sync_that_misses_deadlines_or_fails_still_records_new_courses() {
    use pagelamp_core::auto_sync::{LIGHT_SYNC_KEY, LightSync};

    let f = Fixture::new().await;
    f.standard().await;
    let full = f.sync(&f.options(false)).await.unwrap();
    assert!(full.user_level_read);
    let light = SyncOptions {
        automatic: true,
        user_level_only: true,
        ..f.options(false)
    };
    let waiting = |f: &Fixture| -> Vec<String> {
        let record: LightSync = f
            .store()
            .setting_or_absent(LIGHT_SYNC_KEY)
            .unwrap()
            .unwrap_or_default();
        record.structure_pending.into_iter().collect()
    };
    assert!(waiting(&f).is_empty());
    let course = |id: u64| json!({"id": id, "name": format!("Demo {id}"), "course_code": format!("DEMO{id}")});
    let seminar = format!("{}/course/404", f.source);
    let lab = format!("{}/course/505", f.source);

    // The planner answers 500: not fatal, but the deadlines weren't read.
    f.canvas.reset().await;
    f.get("/users/self", json!({"id": 1, "name": "Demo Student"}))
        .await;
    f.get("/courses", json!([course(101), course(202), course(404)]))
        .await;
    f.get("/announcements", json!([])).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/planner/items"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&f.canvas)
        .await;
    let report = sync_with(&f.api_for(true), &f.db, &f.source, &light, &no_progress)
        .await
        .unwrap();
    assert!(!report.user_level_read);
    assert!(
        report.warnings.iter().any(|w| w.contains("Planner items")),
        "{:?}",
        report.warnings
    );
    assert_eq!(report.new_courses, std::slice::from_ref(&seminar));
    assert_eq!(waiting(&f), std::slice::from_ref(&seminar));

    // Canvas keeps throttling the planner: the run fails as a whole, after it stored the
    // course it met. That course is recorded as waiting all the same.
    f.canvas.reset().await;
    f.get("/users/self", json!({"id": 1, "name": "Demo Student"}))
        .await;
    f.get(
        "/courses",
        json!([course(101), course(202), course(404), course(505)]),
    )
    .await;
    f.get("/announcements", json!([])).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/planner/items"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&f.canvas)
        .await;
    let failed = sync_with(&f.api_for(true), &f.db, &f.source, &light, &no_progress)
        .await
        .unwrap_err();
    assert_eq!(failed.kind, SourceErrorKind::RateLimited);
    assert_eq!(waiting(&f), [seminar, lab]);
    let requests = f.canvas.received_requests().await.unwrap();
    assert!(
        requests
            .iter()
            .all(|r| !r.url.path().starts_with("/api/v1/courses/"))
    );

    // Every other way of not reading it all says so too: one course's announcements fail,
    // or the course list or the planner ends in a next link that isn't followed.
    let elsewhere = "<https://elsewhere.example.org/api/v1/more?page=2>; rel=\"next\"";
    let list = json!([course(101), course(202)]);
    for missing in ["announcements", "courses", "planner"] {
        f.canvas.reset().await;
        f.get("/users/self", json!({"id": 1, "name": "Demo Student"}))
            .await;
        let mut courses = ResponseTemplate::new(200).set_body_json(list.clone());
        let mut planner = ResponseTemplate::new(200).set_body_json(json!([]));
        match missing {
            "announcements" => {
                Mock::given(method("GET"))
                    .and(path("/api/v1/announcements"))
                    .and(query_param("context_codes[]", "course_202"))
                    .respond_with(ResponseTemplate::new(500))
                    .with_priority(1)
                    .mount(&f.canvas)
                    .await;
            }
            "courses" => courses = courses.insert_header("Link", elsewhere),
            _ => planner = planner.insert_header("Link", elsewhere),
        }
        for (at, response) in [
            ("/api/v1/courses", courses),
            ("/api/v1/planner/items", planner),
        ] {
            Mock::given(method("GET"))
                .and(path(at))
                .respond_with(response)
                .mount(&f.canvas)
                .await;
        }
        f.get("/announcements", json!([])).await;
        let report = sync_with(&f.api_for(true), &f.db, &f.source, &light, &no_progress)
            .await
            .unwrap();
        assert!(!report.user_level_read, "{missing}");
        assert!(!report.warnings.is_empty(), "{missing}");
    }
}

/// Which courses a full sync read the structure of: a module listing that fails in a way
/// that may pass leaves the course unread; one the student may not see counts as read.
#[tokio::test]
async fn a_full_sync_says_which_courses_it_could_not_read() {
    let f = Fixture::new().await;
    one_course(&f, json!([{"id": "home"}, {"id": "modules"}])).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/courses/101/modules"))
        .respond_with(ResponseTemplate::new(502))
        .mount(&f.canvas)
        .await;
    let report = f.sync(&f.options(false)).await.unwrap();
    assert_eq!(report.unread_courses, [course101(&f)]);
    assert!(report.read_courses.is_empty());

    f.canvas.reset().await;
    one_course(&f, json!([{"id": "home"}])).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/courses/101/modules"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({"status": "unauthorized"})))
        .mount(&f.canvas)
        .await;
    let report = f.sync(&f.options(false)).await.unwrap();
    assert_eq!(report.read_courses, [course101(&f)]);
    assert!(report.unread_courses.is_empty());
}

#[test]
fn sync_future_is_send() {
    fn assert_send<T: Send>(_: &T) {}
    let config = crate::CanvasConfig {
        base_url: "https://lms.example.edu".into(),
        token: TOKEN.into(),
    };
    let options = SyncOptions {
        download_files: false,
        max_file_bytes: 1,
        files_dir: PathBuf::from("/demo"),
        only_courses: Vec::new(),
        extractor: Default::default(),
        automatic: false,
        user_level_only: false,
    };
    assert_send(&crate::sync(
        Path::new("/demo/db"),
        &config,
        &options,
        &no_progress,
    ));
    assert_send(&crate::check_token(&config));
    assert!(!format!("{config:?}").contains(TOKEN));
}

// ----- regressions from the independent review (each scenario once lost data or leaked) -----

async fn one_course(f: &Fixture, tabs: Value) {
    f.get("/users/self", json!({"id": 1, "name": "Demo Student"}))
        .await;
    f.get(
        "/courses",
        json!([{"id": 101, "name": "Intro to Demo Studies", "course_code": "DEMO101"}]),
    )
    .await;
    f.get("/courses/101/tabs", tabs).await;
    f.get("/courses/101/assignments", json!([])).await;
    f.get("/announcements", json!([])).await;
    f.get("/planner/items", json!([])).await;
}

fn course101(f: &Fixture) -> String {
    format!("{}/course/101", f.source)
}

fn has_material(f: &Fixture, suffix: &str) -> bool {
    f.store()
        .list_materials(&course101(f))
        .unwrap()
        .iter()
        .any(|m| m.id.ends_with(suffix))
}

#[tokio::test]
async fn a_failed_module_file_fetch_keeps_the_material() {
    let f = Fixture::new().await;
    one_course(&f, json!([{"id": "home"}, {"id": "modules"}])).await; // Files + Pages hidden
    f.get(
        "/courses/101/modules",
        json!([{"id": 1, "name": "Week 1", "items": [
        {"id": 11, "type": "File", "content_id": 501, "title": "Slides"}]}]),
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/courses/101/files/501"))
        .respond_with(ResponseTemplate::new(200).set_body_json(f.file(501, "slides.txt", 10)))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&f.canvas)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/courses/101/files/501"))
        .respond_with(ResponseTemplate::new(500))
        .with_priority(2)
        .mount(&f.canvas)
        .await;
    f.sync(&f.options(false)).await.unwrap();
    assert!(has_material(&f, "/file/501"));
    f.sync(&f.options(false)).await.unwrap();
    assert!(
        has_material(&f, "/file/501"),
        "a transient 500 must not delete the file"
    );
}

#[tokio::test]
async fn b_hiding_the_files_tab_keeps_files_outside_modules() {
    let f = Fixture::new().await;
    f.standard().await;
    f.sync(&f.options(false)).await.unwrap();
    assert!(has_material(&f, "/file/503"));
    f.canvas.reset().await;
    f.standard().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/courses/101/tabs"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{"id": "home"}, {"id": "modules"}, {"id": "pages"}])),
        )
        .with_priority(1)
        .mount(&f.canvas)
        .await;
    f.sync(&f.options(false)).await.unwrap();
    assert!(has_material(&f, "/file/503"));
}

#[tokio::test]
async fn c_only_courses_keeps_other_courses_planner_events() {
    let f = Fixture::new().await;
    f.standard().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/planner/items"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"plannable_id": 901, "plannable_type": "calendar_event", "course_id": 101,
             "plannable_date": (Utc::now() + TimeDelta::days(3)).to_rfc3339(),
             "plannable": {"title": "Midterm exam DEMO101"}}])))
        .with_priority(1)
        .mount(&f.canvas)
        .await;
    f.sync(&f.options(false)).await.unwrap();
    let mut only = f.options(false);
    only.only_courses = vec!["DEMO202".into()];
    f.sync(&only).await.unwrap();
    assert!(
        all_events(&f.store())
            .iter()
            .any(|e| e.title == "Midterm exam DEMO101")
    );
    only.only_courses = vec!["NO-SUCH-COURSE".into()];
    f.sync(&only).await.unwrap();
    assert!(
        all_events(&f.store())
            .iter()
            .any(|e| e.title == "Midterm exam DEMO101")
    );
}

#[tokio::test]
async fn d_odd_fields_and_restricted_courses_keep_the_course_and_its_settings() {
    let f = Fixture::new().await;
    f.standard().await;
    f.sync(&f.options(false)).await.unwrap();
    let c202 = format!("{}/course/202", f.source);
    f.store().set_course_hidden(&c202, true).unwrap();
    f.canvas.reset().await;
    f.standard().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/courses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 101, "name": "Intro to Demo Studies", "course_code": "DEMO101"},
            {"id": 202, "name": "Advanced Demo Studies", "course_code": "DEMO202", "start_at": "2026-09-07"}
        ])))
        .with_priority(1)
        .mount(&f.canvas)
        .await;
    f.sync(&f.options(false)).await.unwrap();
    let course = f.store().get_course(&c202).unwrap().expect("course kept");
    assert!(course.hidden);
    assert_eq!(course.term_start.unwrap().to_string(), "2026-09-07");

    // At term end Canvas returns a date-restricted stub: keep the course.
    f.canvas.reset().await;
    f.standard().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/courses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 101, "name": "Intro to Demo Studies", "course_code": "DEMO101"},
            {"id": 202, "access_restricted_by_date": true}
        ])))
        .with_priority(1)
        .mount(&f.canvas)
        .await;
    f.sync(&f.options(false)).await.unwrap();
    assert!(f.store().get_course(&c202).unwrap().unwrap().hidden);
    // ...and record that Canvas restricts it (it could not be synced again once removed).
    let lms = f.store().course_term_data(&c202).unwrap().unwrap().lms;
    assert_eq!(lms.access_restricted, Some(true));
    assert_eq!(lms.course_start.unwrap().to_string(), "2026-09-07");
}

#[tokio::test]
async fn e_a_throttled_or_unsavable_download_is_only_a_warning() {
    let f = Fixture::new().await;
    let long_name: String = "講".repeat(100) + ".txt";
    Mock::given(method("GET"))
        .and(path("/api/v1/courses/101/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            f.file(501, &long_name, 20),
            f.file(502, "notes.txt", 20)
        ])))
        .with_priority(1)
        .mount(&f.canvas)
        .await;
    f.standard().await;
    for id in [501, 502] {
        Mock::given(method("GET"))
            .and(path(format!("/files/{id}/download")))
            .respond_with(ResponseTemplate::new(302).insert_header(
                "Location",
                format!("{}/blob/{id}", f.storage.uri()).as_str(),
            ))
            .mount(&f.canvas)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/blob/501"))
        .respond_with(ResponseTemplate::new(200).set_body_string("long name ok"))
        .mount(&f.storage)
        .await;
    Mock::given(method("GET"))
        .and(path("/blob/502"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&f.storage)
        .await;
    let report = f.sync(&f.options(true)).await.unwrap();
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("Week 2 notes could not be downloaded")),
        "{:?}",
        report.warnings
    );
    assert!(
        !all_events(&f.store()).is_empty(),
        "events are still written"
    );
    assert_eq!(
        f.store().search("name", None, 5).unwrap().len(),
        1,
        "the long CJK file name worked"
    );
}

#[tokio::test]
async fn g_relative_and_unusable_next_links() {
    let f = Fixture::new().await;
    f.standard().await;
    f.sync(&f.options(false)).await.unwrap();
    assert!(has_material(&f, "/file/503"));

    // Relative next link: followed.
    f.canvas.reset().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/courses/101/files"))
        .and(query_param("page", "2"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!([f.file(503, "huge.txt", 5000)])),
        )
        .with_priority(1)
        .mount(&f.canvas)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/courses/101/files"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([f.file(501, "slides.txt", 20)]))
                .insert_header(
                    "Link",
                    "</api/v1/courses/101/files?page=2&per_page=100>; rel=\"next\"",
                ),
        )
        .with_priority(2)
        .mount(&f.canvas)
        .await;
    f.standard().await;
    f.sync(&f.options(false)).await.unwrap();
    assert!(has_material(&f, "/file/503"));

    // Unusable next link: the listing is incomplete, nothing is pruned.
    f.canvas.reset().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/courses/101/files"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([f.file(501, "slides.txt", 20)]))
                .insert_header("Link", "<http://[invalid>; rel=\"next\""),
        )
        .with_priority(1)
        .mount(&f.canvas)
        .await;
    f.standard().await;
    let report = f.sync(&f.options(false)).await.unwrap();
    assert!(has_material(&f, "/file/503"), "{:?}", report.warnings);
}

#[tokio::test]
async fn h_a_redirect_back_to_canvas_is_refused() {
    let f = Fixture::new().await;
    f.standard().await;
    Mock::given(method("GET"))
        .and(path("/files/501/download"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("Location", format!("{}/bounce", f.storage.uri()).as_str()),
        )
        .mount(&f.canvas)
        .await;
    Mock::given(method("GET"))
        .and(path("/bounce"))
        .respond_with(ResponseTemplate::new(302).insert_header(
            "Location",
            format!("{}/api/v1/conversations?scope=unread", f.canvas.uri()).as_str(),
        ))
        .mount(&f.storage)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/conversations"))
        .respond_with(ResponseTemplate::new(200).set_body_string("private inbox"))
        .expect(0)
        .mount(&f.canvas)
        .await;
    let report = f.sync(&f.options(true)).await.unwrap();
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("could not be downloaded"))
    );
    assert!(f.store().search("inbox", None, 5).unwrap().is_empty());
}

#[tokio::test]
async fn i_a_locked_file_without_its_copy_loses_its_old_text() {
    let f = Fixture::new().await;
    f.standard().await;
    f.downloads().await;
    f.sync(&f.options(true)).await.unwrap();
    assert_eq!(f.store().search("stomata", None, 5).unwrap().len(), 1);
    // The copy is gone (the student cleaned up), and the file is locked now.
    let materials = f.store().list_materials(&course101(&f)).unwrap();
    let notes = material(&materials, "/file/502").clone();
    std::fs::remove_file(notes.local_path.as_deref().unwrap()).unwrap();
    f.canvas.reset().await;
    f.storage.reset().await;
    let mut locked = f.file(502, "notes.txt", 20);
    locked["url"] = Value::Null;
    locked["locked_for_user"] = json!(true);
    Mock::given(method("GET"))
        .and(path("/api/v1/courses/101/files"))
        .and(query_param("page", "2"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([locked, f.file(503, "huge.txt", 5000)])),
        )
        .with_priority(1)
        .mount(&f.canvas)
        .await;
    f.standard().await;
    f.sync(&f.options(true)).await.unwrap();

    let store = f.store();
    let notes = store.get_material(&notes.id).unwrap().unwrap();
    assert_eq!(notes.text_status, TextStatus::NotDownloaded);
    // No reader can see the old text: not search, not read_material, not a model's context.
    assert_eq!(store.chunk_count(&notes.id).unwrap(), 0);
    assert!(store.search("stomata", None, 5).unwrap().is_empty());
    let read = pagelamp_core::views::read_material(&store, &notes.id, 0, 12_000).unwrap();
    assert!(read.chunks.is_empty());
}

#[tokio::test]
async fn i_a_locked_file_keeps_its_earlier_copy() {
    let f = Fixture::new().await;
    f.standard().await;
    f.downloads().await;
    f.sync(&f.options(true)).await.unwrap();
    f.canvas.reset().await;
    f.storage.reset().await;
    let mut locked = f.file(502, "notes.txt", 20);
    locked["url"] = Value::Null;
    locked["locked_for_user"] = json!(true);
    locked["updated_at"] = json!("2026-09-25T10:00:00Z");
    Mock::given(method("GET"))
        .and(path("/api/v1/courses/101/files"))
        .and(query_param("page", "2"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([locked, f.file(503, "huge.txt", 5000)])),
        )
        .with_priority(1)
        .mount(&f.canvas)
        .await;
    f.standard().await;
    f.sync(&f.options(true)).await.unwrap();
    let materials = f.store().list_materials(&course101(&f)).unwrap();
    let notes = material(&materials, "/file/502");
    assert_eq!(notes.text_status, TextStatus::Ok);
    assert_eq!(notes.download_blocked, None, "it has a copy");
    assert!(
        notes
            .local_path
            .as_deref()
            .is_some_and(|p| Path::new(p).is_file())
    );
    assert_eq!(f.store().search("stomata", None, 5).unwrap().len(), 1);
}

#[tokio::test]
async fn files_that_cannot_be_downloaded_say_why_until_they_can() {
    let f = Fixture::new().await;
    let mut locked = f.file(502, "notes.txt", 20);
    locked["url"] = Value::Null;
    locked["locked_for_user"] = json!(true);
    Mock::given(method("GET"))
        .and(path("/api/v1/courses/101/files"))
        .and(query_param("page", "2"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([locked, f.file(503, "huge.txt", 5000)])),
        )
        .with_priority(1)
        .mount(&f.canvas)
        .await;
    f.standard().await;
    f.sync(&f.options(false)).await.unwrap();
    let materials = f.store().list_materials(&course101(&f)).unwrap();
    let state = |id: &str| {
        let m = material(&materials, id);
        (m.text_status, m.download_blocked)
    };
    assert_eq!(state("/file/501"), (TextStatus::NotDownloaded, None));
    assert_eq!(
        state("/file/502"),
        (TextStatus::NotDownloaded, Some(DownloadBlock::Locked))
    );
    assert_eq!(
        state("/file/503"),
        (TextStatus::NotDownloaded, Some(DownloadBlock::TooLarge))
    );

    // Unlocked later: the reason is cleared, the file can be downloaded on request. The big
    // file's size is no longer listed: it stays "too large". A file without a download link
    // can't be downloaded either.
    f.canvas.reset().await;
    let mut size_unknown = f.file(503, "huge.txt", 0);
    size_unknown["size"] = Value::Null;
    let mut no_link = f.file(504, "linkless.txt", 20);
    no_link["url"] = json!("not a url");
    Mock::given(method("GET"))
        .and(path("/api/v1/courses/101/files"))
        .and(query_param("page", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            f.file(502, "notes.txt", 20),
            size_unknown,
            no_link
        ])))
        .with_priority(1)
        .mount(&f.canvas)
        .await;
    f.standard().await;
    f.sync(&f.options(false)).await.unwrap();
    let materials = f.store().list_materials(&course101(&f)).unwrap();
    let state = |id: &str| {
        let m = material(&materials, id);
        (m.text_status, m.download_blocked)
    };
    assert_eq!(state("/file/502"), (TextStatus::NotDownloaded, None));
    assert_eq!(
        state("/file/503"),
        (TextStatus::NotDownloaded, Some(DownloadBlock::TooLarge))
    );
    assert_eq!(
        state("/file/504"),
        (TextStatus::NotDownloaded, Some(DownloadBlock::Locked))
    );
}

#[tokio::test]
async fn l_a_date_only_due_date_keeps_the_deadline() {
    let f = Fixture::new().await;
    f.standard().await;
    f.sync(&f.options(false)).await.unwrap();
    f.canvas.reset().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/courses/101/assignments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 9, "name": "Problem Set 1", "due_at": "2026-10-01", "submission_types": ["online_upload"]},
            {"id": "bad/id", "name": "Broken"}
        ])))
        .with_priority(1)
        .mount(&f.canvas)
        .await;
    f.standard().await;
    f.sync(&f.options(false)).await.unwrap();
    assert!(
        all_events(&f.store())
            .iter()
            .any(|e| e.title == "Problem Set 1")
    );
}

#[tokio::test]
async fn m_an_interrupted_download_is_retried_next_time() {
    let f = Fixture::new().await;
    f.standard().await;
    f.downloads().await;
    f.sync(&f.options(true)).await.unwrap();
    // Both files change; storage throttles the first download.
    f.canvas.reset().await;
    f.storage.reset().await;
    let mut a = f.file(501, "slides.txt", 20);
    a["updated_at"] = json!("2026-09-25T10:00:00Z");
    let mut b = f.file(502, "notes.txt", 20);
    b["updated_at"] = json!("2026-09-25T10:00:00Z");
    Mock::given(method("GET"))
        .and(path("/api/v1/courses/101/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            a,
            b,
            f.file(503, "huge.txt", 5000)
        ])))
        .with_priority(1)
        .mount(&f.canvas)
        .await;
    f.standard().await;
    for id in [501, 502] {
        Mock::given(method("GET"))
            .and(path(format!("/files/{id}/download")))
            .respond_with(ResponseTemplate::new(302).insert_header(
                "Location",
                format!("{}/blob/{id}", f.storage.uri()).as_str(),
            ))
            .mount(&f.canvas)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/blob/501"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&f.storage)
        .await;
    Mock::given(method("GET"))
        .and(path("/blob/502"))
        .respond_with(ResponseTemplate::new(200).set_body_string("stomata v2 exam relocated"))
        .mount(&f.storage)
        .await;
    f.sync(&f.options(true)).await.unwrap();
    assert_eq!(
        f.store().search("chlorophyll", None, 5).unwrap().len(),
        1,
        "old text kept meanwhile"
    );
    // Next sync: storage healthy → the stale file is fetched again.
    f.storage.reset().await;
    Mock::given(method("GET"))
        .and(path("/blob/501"))
        .respond_with(ResponseTemplate::new(200).set_body_string("chlorophyll v2 revised"))
        .mount(&f.storage)
        .await;
    f.sync(&f.options(true)).await.unwrap();
    let hits: Vec<String> = f
        .storage
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.url.path().to_string())
        .collect();
    assert_eq!(
        hits,
        ["/blob/501"],
        "only the stale file is downloaded again"
    );
    let revised = f.store().search("revised", None, 5).unwrap();
    assert_eq!(revised.len(), 1, "{revised:?}");
    assert_eq!(f.store().search("relocated", None, 5).unwrap().len(), 1);
}

#[tokio::test]
async fn o_a_cached_copy_the_text_reader_could_not_read_is_read_again_without_downloading() {
    use pagelamp_core::ingest::failure_fingerprint;
    let f = Fixture::new().await;
    f.standard().await;
    f.downloads().await; // each file may be downloaded once (`expect(1)`, checked on drop)
    f.sync(&f.options(true)).await.unwrap();
    let materials = f.store().list_materials(&course101(&f)).unwrap();
    let slides = material(&materials, "/file/501").id.clone();
    let notes = material(&materials, "/file/502").id.clone();
    // Last time the worker couldn't start for the slides, and the notes hit a limit.
    let store = f.store();
    for (id, kind) in [
        (&slides, TextErrorKind::SpawnFailed),
        (&notes, TextErrorKind::TimedOut),
    ] {
        store
            .set_text_state(id, TextStatus::Error, Some("demo"), None)
            .unwrap();
        store.replace_chunks(id, &[]).unwrap();
        store
            .set_text_error_kind(id, kind, &failure_fingerprint())
            .unwrap();
    }

    let report = f.sync(&f.options(true)).await.unwrap();
    assert_eq!(report.files_downloaded, 0);
    let store = f.store();
    assert_eq!(store.search("chlorophyll", None, 5).unwrap().len(), 1);
    assert_eq!(store.search("stomata", None, 5).unwrap().len(), 0);
    let notes_row = store.get_material(&notes).unwrap().unwrap();
    assert_eq!(notes_row.text_error_kind, Some(TextErrorKind::TimedOut));

    // Recorded by another app version: read again too.
    store
        .set_text_error_kind(&notes, TextErrorKind::TimedOut, "worker0/0.0.1")
        .unwrap();
    let report = f.sync(&f.options(true)).await.unwrap();
    assert_eq!(report.files_downloaded, 0);
    assert_eq!(f.store().search("stomata", None, 5).unwrap().len(), 1);
}

#[tokio::test]
async fn n_old_announcements_outside_the_window_are_kept() {
    let f = Fixture::new().await;
    f.standard().await;
    f.sync(&f.options(false)).await.unwrap();
    // An announcement from last term, stored by an earlier sync.
    let store = f.store();
    let old = MaterialUpsert {
        id: format!("{}/announcement/42", f.source),
        course_id: course101(&f),
        module_id: None,
        kind: MaterialKind::Announcement,
        title: "Welcome back".into(),
        url: None,
        local_path: None,
        mime: None,
        published_at: Some(Utc::now() - TimeDelta::days(200)),
        week_hint: None,
    };
    store.upsert_material(&old).unwrap();
    f.sync(&f.options(false)).await.unwrap();
    assert!(has_material(&f, "/announcement/42"));
    assert!(!has_material(&f, "/announcement/999"));
}

// ----- diagnostics never leak (pagelamp sync -v) -------------------------------------------

/// Everything every crate logs at TRACE, from all tests of this binary (one global
/// subscriber: tracing caches per-callsite interest globally, so per-thread subscribers miss
/// events when tests run in parallel). Absence checks therefore cover every test's output.
fn global_logs() -> std::sync::Arc<std::sync::Mutex<Vec<u8>>> {
    use std::sync::{Arc, Mutex, OnceLock};
    static LOGS: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();
    LOGS.get_or_init(|| {
        #[derive(Clone)]
        struct Buf(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for Buf {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(b);
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let buf = Arc::new(Mutex::new(Vec::new()));
        let writer = Buf(buf.clone());
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::set_global_default(subscriber).expect("only this test sets one");
        buf
    })
    .clone()
}

/// Run `f` and return all log output produced meanwhile (by any test).
async fn capture_logs<F: std::future::Future<Output = ()>>(f: F) -> String {
    let logs = global_logs();
    f.await;
    String::from_utf8_lossy(&logs.lock().unwrap()).into_owned()
}

#[tokio::test]
async fn debug_output_shows_requests_but_never_secrets() {
    let f = Fixture::new().await;
    f.standard().await;
    // 403 with a Canvas error body, a 500, and downloads with verifier/signature parameters.
    Mock::given(method("GET"))
        .and(path("/api/v1/courses/202/modules"))
        .respond_with(
            ResponseTemplate::new(500)
                .set_body_string("Internal Server Error <html>secret-body</html>"),
        )
        .with_priority(1)
        .mount(&f.canvas)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/courses/101/pages"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "status": "unauthorized",
            "errors": [{"message": "user not authorized to perform that action"}]
        })))
        .with_priority(1)
        .mount(&f.canvas)
        .await;
    Mock::given(method("GET"))
        .and(path("/files/501/download"))
        .respond_with(
            ResponseTemplate::new(302).insert_header(
                "Location",
                format!(
                    "{}/blob/501?X-Amz-Signature=S1GNATURE&X-Amz-Credential=CR3D",
                    f.storage.uri()
                )
                .as_str(),
            ),
        )
        .mount(&f.canvas)
        .await;
    Mock::given(method("GET"))
        .and(path("/blob/501"))
        .respond_with(ResponseTemplate::new(200).set_body_string("SLIDE-TEXT-CONTENT"))
        .mount(&f.storage)
        .await;
    let mut file = f.file(501, "slides.txt", 20);
    file["url"] = json!(format!(
        "{}/files/501/download?download_frd=1&verifier=V3R1F13R",
        f.canvas.uri()
    ));
    Mock::given(method("GET"))
        .and(path("/api/v1/courses/101/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([file])))
        .with_priority(1)
        .mount(&f.canvas)
        .await;
    let options = f.options(true);
    let api = f.api();
    let logs = capture_logs(async {
        let _ = sync_with(&api, &f.db, &f.source, &options, &no_progress).await;
    })
    .await;

    // Useful: requests, statuses, Canvas's own error message.
    assert!(logs.contains("GET /api/v1/users/self → 200"), "{logs}");
    assert!(logs.contains("GET /api/v1/courses/202/modules?include[]=items&include[]=content_details&per_page=100 → 500"), "{logs}");
    assert!(
        logs.contains("Canvas says: user not authorized to perform that action"),
        "{logs}"
    );
    assert!(logs.contains("GET /files/501/download → 302"), "{logs}");
    assert!(
        logs.contains("GET file storage (127.0.0.1) → 200"),
        "{logs}"
    );
    assert!(logs.contains("rate limit remaining"), "{logs}");
    // Never: the token, verifiers, signatures, response bodies, course content.
    for secret in [
        TOKEN,
        "V3R1F13R",
        "S1GNATURE",
        "CR3D",
        "secret-body",
        "SLIDE-TEXT-CONTENT",
        "Photosynthesis introduction",
        "SECRET-INSTRUCTIONS",
    ] {
        assert!(
            !logs.contains(secret),
            "{secret} leaked into debug output:\n{logs}"
        );
    }

    // A rejected token (401) says so without the token.
    let g = Fixture::new().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/users/self"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({"errors": [{"message": "Invalid access token."}]})),
        )
        .mount(&g.canvas)
        .await;
    let api = g.api();
    let options = g.options(false);
    let logs = capture_logs(async {
        let _ = sync_with(&api, &g.db, &g.source, &options, &no_progress).await;
    })
    .await;
    assert!(
        logs.contains("→ 401") && logs.contains("Canvas says: Invalid access token."),
        "{logs}"
    );
    assert!(
        !logs.contains(TOKEN) && !logs.to_lowercase().contains("bearer"),
        "{logs}"
    );
}

// ----- real-world shapes (modelled on the Canvas REST docs) ---------------------------------

/// One course whose modules contain every item type Canvas has, locked content, a hidden
/// Files tab (401 on /files) and HTML with iframes/LTI embeds.
#[tokio::test]
async fn every_module_item_type_locked_items_and_embeds() {
    let f = Fixture::new().await;
    f.get("/users/self", json!({"id": 1, "name": "Demo Student"}))
        .await;
    f.get(
        "/courses",
        json!([{"id": 101, "name": "Intro to Demo Studies", "course_code": "DEMO101"}]),
    )
    .await;
    f.get(
        "/courses/101/tabs",
        json!([{"id": "home"}, {"id": "modules"}, {"id": "pages"}]),
    )
    .await;
    let future = (Utc::now() + TimeDelta::days(30)).to_rfc3339();
    f.get(
        "/courses/101/modules",
        json!([{"id": 1, "name": "Week 1", "position": 1, "items": [
            {"id": 10, "type": "SubHeader", "title": "Readings"},
            {"id": 11, "type": "File", "content_id": 501, "title": "Week 1 slides"},
            {"id": 12, "type": "Page", "page_url": "welcome", "title": "Welcome"},
            {"id": 13, "type": "ExternalUrl", "external_url": "https://video.example.edu/w1", "title": "Lecture video"},
            {"id": 14, "type": "ExternalTool", "title": "Textbook (LTI)", "url": "https://lms.example.edu/api/v1/courses/101/external_tools/sessionless_launch"},
            {"id": 15, "type": "Quiz", "content_id": 3, "title": "Quiz 1"},
            {"id": 16, "type": "Discussion", "content_id": 4, "title": "Intro discussion"},
            {"id": 17, "type": "Assignment", "content_id": 9, "title": "Problem Set 1"},
            {"id": 18, "type": "Page", "page_url": "locked-notes", "title": "Locked notes"},
            {"id": 19, "type": "SomethingNew", "title": "Future item type"}
        ]},
        {"id": 2, "name": "Week 5", "position": 2, "unlock_at": future, "items": []}]),
    )
    .await;
    // Files tab hidden: the list 401s (resource, not token), module files still resolve.
    Mock::given(method("GET"))
        .and(path("/api/v1/courses/101/files"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({"status": "unauthorized", "errors": [{"message": "user not authorized to perform that action"}]})))
        .expect(0) // the tab is hidden, so it isn't even asked
        .mount(&f.canvas)
        .await;
    let mut locked_file = f.file(501, "slides.pdf", 20);
    locked_file["locked_for_user"] = json!(true);
    f.get("/courses/101/files/501", locked_file).await;
    f.get(
        "/courses/101/pages",
        json!([
            {"page_id": 601, "url": "welcome", "title": "Welcome", "updated_at": "2026-09-08T10:00:00Z"},
            {"page_id": 602, "url": "locked-notes", "title": "Locked notes", "locked_for_user": true, "lock_explanation": "This page is locked until Oct 1"}
        ]),
    )
    .await;
    f.get(
        "/courses/101/pages/welcome",
        json!({"page_id": 601, "url": "welcome", "title": "Welcome", "updated_at": "2026-09-08T10:00:00Z",
               "body": "<h1>Welcome</h1><p>Chlorophyll absorbs light.</p><iframe src=\"https://video.example.edu/embed/1\"></iframe><form action=\"https://lti.example.com/launch\" method=\"post\"><input type=\"hidden\" name=\"oauth_signature\" value=\"LTI-SIGNATURE\"></form><script>launchLti()</script>"}),
    )
    .await;
    // The locked page's body is never requested.
    Mock::given(method("GET"))
        .and(path("/api/v1/courses/101/pages/locked-notes"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&f.canvas)
        .await;
    f.get("/courses/101/assignments", json!([])).await;
    f.get(
        "/announcements",
        json!([{"id": 701, "title": "Slides posted", "message": "<p>See attached.</p>", "posted_at": "2026-09-22T12:00:00Z",
                "attachments": [{"id": 9001, "display_name": "extra.pdf", "url": "https://lms.example.edu/files/9001/download?verifier=X"}]}]),
    )
    .await;
    f.get("/planner/items", json!([])).await;

    let report = f.sync(&f.options(true)).await.unwrap();
    let store = f.store();
    let materials = store.list_materials(&course101(&f)).unwrap();
    let kinds: Vec<(String, MaterialKind)> = materials
        .iter()
        .map(|m| (m.title.clone(), m.kind))
        .collect();
    // Only File/Page/ExternalUrl items become materials; the rest are structure we skip.
    for skipped in [
        "Readings",
        "Textbook (LTI)",
        "Quiz 1",
        "Intro discussion",
        "Problem Set 1",
        "Future item type",
    ] {
        assert!(
            !kinds.iter().any(|(t, _)| t == skipped),
            "{skipped} should not be a material: {kinds:?}"
        );
    }
    let slides = material(&materials, "/file/501");
    assert_eq!(
        slides.text_status,
        TextStatus::NotDownloaded,
        "locked files are not downloaded"
    );
    assert_eq!(
        material(&materials, "/page/602").text_status,
        TextStatus::Pending,
        "locked page: title only"
    );
    assert_eq!(
        material(&materials, "/link/13").kind,
        MaterialKind::ExternalLink
    );
    // The embed-heavy page is indexed as text; scripts and LTI signatures are not.
    assert_eq!(store.search("chlorophyll", None, 5).unwrap().len(), 1);
    assert!(store.search("launchLti", None, 5).unwrap().is_empty());
    assert!(store.search("SIGNATURE", None, 5).unwrap().is_empty());
    // Announcement attachments are not followed.
    assert!(
        !f.canvas
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|r| r.url.path().contains("9001"))
    );
    // A future-unlocking module is stored with its date.
    let modules = store.list_modules(&course101(&f)).unwrap();
    assert!(
        modules
            .iter()
            .any(|m| m.name == "Week 5" && m.unlock_at.is_some())
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("Files tab hidden"))
    );
}

#[tokio::test]
async fn two_hundred_pages_paginate_and_are_skipped_when_unchanged() {
    let f = Fixture::new().await;
    f.get("/users/self", json!({"id": 1, "name": "Demo Student"}))
        .await;
    f.get(
        "/courses",
        json!([{"id": 101, "name": "Intro to Demo Studies", "course_code": "DEMO101"}]),
    )
    .await;
    f.get(
        "/courses/101/tabs",
        json!([{"id": "home"}, {"id": "pages"}]),
    )
    .await;
    f.get("/courses/101/modules", json!([])).await;
    f.get("/courses/101/assignments", json!([])).await;
    f.get("/announcements", json!([])).await;
    f.get("/planner/items", json!([])).await;
    let pages: Vec<Value> = (1..=230)
        .map(|n| json!({"page_id": 1000 + n, "url": format!("page-{n}"), "title": format!("Page {n}"), "updated_at": "2026-09-08T10:00:00Z"}))
        .collect();
    for (index, chunk) in pages.chunks(100).enumerate() {
        let page = index + 1;
        let mut response = ResponseTemplate::new(200).set_body_json(json!(chunk));
        if page < 3 {
            let next = format!(
                "<{}/api/v1/courses/101/pages?page={}&per_page=100>; rel=\"next\"",
                f.canvas.uri(),
                page + 1
            );
            response = response.insert_header("Link", next.as_str());
        }
        let mut mock = Mock::given(method("GET")).and(path("/api/v1/courses/101/pages"));
        mock = if page == 1 {
            mock.and(wiremock::matchers::query_param_is_missing("page"))
        } else {
            mock.and(query_param("page", page.to_string().as_str()))
        };
        mock.respond_with(response).mount(&f.canvas).await;
    }
    Mock::given(method("GET"))
        .and(wiremock::matchers::path_regex(
            r"^/api/v1/courses/101/pages/page-\d+$",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"updated_at": "2026-09-08T10:00:00Z", "body": "<p>lecture notes</p>"}),
        ))
        .mount(&f.canvas)
        .await;
    f.sync(&f.options(false)).await.unwrap();
    let count = |requests: &[Request]| {
        requests
            .iter()
            .filter(|r| r.url.path().starts_with("/api/v1/courses/101/pages/page-"))
            .count()
    };
    assert_eq!(count(&f.canvas.received_requests().await.unwrap()), 230);
    let pages = f.store().list_materials(&course101(&f)).unwrap();
    assert_eq!(
        pages
            .iter()
            .filter(|m| m.kind == MaterialKind::Page)
            .count(),
        230
    );

    // Second sync: nothing changed → no page bodies fetched again.
    let before = count(&f.canvas.received_requests().await.unwrap());
    f.sync(&f.options(false)).await.unwrap();
    assert_eq!(count(&f.canvas.received_requests().await.unwrap()), before);
}

#[tokio::test]
async fn a_5xx_in_one_course_does_not_stop_the_others() {
    let f = Fixture::new().await;
    f.standard().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/courses/202/tabs"))
        .respond_with(ResponseTemplate::new(502))
        .with_priority(1)
        .mount(&f.canvas)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/courses/202/assignments"))
        .respond_with(ResponseTemplate::new(503))
        .with_priority(1)
        .mount(&f.canvas)
        .await;
    let report = f.sync(&f.options(false)).await.unwrap();
    assert_eq!(report.courses, 2);
    assert!(
        report.warnings.iter().any(|w| w.starts_with("DEMO202")),
        "{:?}",
        report.warnings
    );
    assert_eq!(
        f.store().search("photosynthesis", None, 5).unwrap().len(),
        1,
        "DEMO101 fully synced"
    );
    let summary = report
        .course_summaries
        .iter()
        .find(|c| c.course == "DEMO101")
        .unwrap();
    assert!(
        summary.files >= 2 && summary.pages == 1 && summary.events == 1,
        "{summary:?}"
    );
    assert!(report.requests > 10);
}

#[tokio::test]
async fn p_a_stopped_sync_ends_between_courses_as_cancelled() {
    let f = Fixture::new().await;
    f.standard().await;
    let cancel = pagelamp_core::source::CancelFlag::new();
    let mut options = f.options(false);
    options.extractor = pagelamp_core::ingest::Extractor::default().cancellable(cancel.clone());
    cancel.cancel();
    let err = f.sync(&options).await.unwrap_err();
    assert!(err.cancelled, "{err:?}");
    // It stopped before the first course: nothing was written for any course.
    assert!(f.store().list_courses(true).unwrap().is_empty());
}
