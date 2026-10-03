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
use crate::transport::{RetryPolicy, TokenTransport};
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
        let base = url::Url::parse(&self.canvas.uri()).unwrap();
        let retry = RetryPolicy {
            base_delay: Duration::from_millis(1),
            max_tries: 3,
        };
        Api::new(
            TokenTransport::new(base.clone(), TOKEN, retry).unwrap(),
            base,
        )
    }

    fn options(&self, download_files: bool) -> SyncOptions {
        SyncOptions {
            download_files,
            max_file_bytes: 1024,
            files_dir: self.files.clone(),
            only_courses: Vec::new(),
            only_files: None,
            extractor: Default::default(),
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
async fn only_the_files_the_student_chose_are_downloaded() {
    let f = Fixture::new().await;
    f.standard().await;
    f.downloads().await;
    let chosen = |id: &str| SyncOptions {
        only_files: Some([format!("{}/file/{id}", f.source)].into()),
        ..f.options(true)
    };
    let warnings = Mutex::new(Vec::new());
    let report = sync_with(&f.api(), &f.db, &f.source, &chosen("501"), &|p| {
        if let SyncProgress::Warning(w) = p {
            warnings.lock().unwrap().push(w);
        }
    })
    .await
    .unwrap();
    assert_eq!(report.files_downloaded, 1);
    // A file nobody asked for isn't "skipped": it just isn't downloaded.
    assert!(
        warnings
            .into_inner()
            .unwrap()
            .iter()
            .all(|w| !w.contains("huge.txt"))
    );
    let course = format!("{}/course/101", f.source);
    let materials = f.store().list_materials(&course).unwrap();
    assert_eq!(
        material(&materials, "/file/501").text_status,
        TextStatus::Ok
    );
    assert_eq!(
        material(&materials, "/file/502").text_status,
        TextStatus::NotDownloaded
    );
    // The next choice downloads that one and keeps the first (each blob is fetched once).
    let report = f.sync(&chosen("502")).await.unwrap();
    assert_eq!(report.files_downloaded, 1);
    let materials = f.store().list_materials(&course).unwrap();
    assert_eq!(
        material(&materials, "/file/501").text_status,
        TextStatus::Ok
    );
    assert_eq!(
        material(&materials, "/file/502").text_status,
        TextStatus::Ok
    );
}

#[tokio::test]
async fn the_syllabus_links_and_the_front_page_mark_calendar_candidates() {
    let f = Fixture::new().await;
    // The first sync sees a syllabus linking two files (one of another course, one unknown)
    // and a pages list naming the front page; later syncs see the standard fixture.
    Mock::given(method("GET"))
        .and(path("/api/v1/courses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 101, "name": "Intro to Demo Studies", "course_code": "DEMO101",
             "syllabus_body": "<p>See the <a href=\"/courses/101/files/502/download?wrap=1\">outline</a>, \
                 <a href=\"/courses/999/files/777\">last year</a> and \
                 <a data-api-endpoint=\"https://lms.example.edu/api/v1/courses/101/files/888\">a gone file</a>.</p>"}
        ])))
        .with_priority(1)
        .up_to_n_times(1)
        .mount(&f.canvas)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/courses/101/pages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"page_id": 601, "url": "week-1-overview", "title": "Overview",
             "updated_at": "2026-09-08T10:00:00Z", "front_page": true}
        ])))
        .with_priority(1)
        .up_to_n_times(1)
        .mount(&f.canvas)
        .await;
    f.standard().await;
    f.sync(&f.options(false)).await.unwrap();
    let course = format!("{}/course/101", f.source);
    let signals = f.store().calendar_signals(&course).unwrap();
    assert_eq!(
        signals.linked_from_syllabus,
        [format!("{}/file/502", f.source)].into()
    );
    assert_eq!(
        signals.front_page,
        [format!("{}/page/601", f.source)].into()
    );

    // The standard fixture: a syllabus without links clears them; a pages list that doesn't
    // say which page is the front page leaves the flag as it was.
    f.sync(&f.options(false)).await.unwrap();
    let signals = f.store().calendar_signals(&course).unwrap();
    assert!(signals.linked_from_syllabus.is_empty());
    assert_eq!(
        signals.front_page,
        [format!("{}/page/601", f.source)].into()
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
        only_files: None,
        extractor: Default::default(),
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
