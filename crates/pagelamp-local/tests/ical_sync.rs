//! iCal feed fetch + sync against a local wiremock server (synthetic feeds only).

use pagelamp_core::model::*;
use pagelamp_core::source::no_progress;
use pagelamp_core::store::Store;
use pagelamp_local::{fetch_ical, sync_ical};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SOURCE: &str = "ical:demo";

fn feed(events: &str) -> String {
    format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Demo//EN\r\n{events}END:VCALENDAR\r\n")
}

const TWO_EVENTS: &str = "BEGIN:VEVENT\r\nUID:ps1\r\nDTSTART:20260930T035900Z\r\n\
    DTEND:20260930T035900Z\r\nSUMMARY:Problem Set 1 [DEMO101 F LEC0101]\r\nEND:VEVENT\r\n\
    BEGIN:VEVENT\r\nUID:talk\r\nDTSTART:20261001T180000Z\r\nDTEND:20261001T190000Z\r\n\
    SUMMARY:Campus talk\r\nEND:VEVENT\r\n";

/// A tempfile DB with the iCal source and one course DEMO101.
fn db() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("pagelamp.db");
    let store = Store::open(&db).unwrap();
    for (id, kind) in [
        (SOURCE, SourceKind::Ical),
        ("folder:demo", SourceKind::Folder),
    ] {
        store
            .upsert_source(&SourceRecord {
                id: id.into(),
                kind,
                label: id.into(),
                config: json!({}),
                last_synced_at: None,
                last_error: None,
                last_error_kind: None,
            })
            .unwrap();
    }
    store
        .upsert_course(&CourseUpsert {
            id: "folder:demo/course/DEMO101".into(),
            source_id: "folder:demo".into(),
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
    (dir, db)
}

async fn serve(status: u16, body: String) -> (MockServer, String) {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed/private-token-abc.ics"))
        .respond_with(ResponseTemplate::new(status).set_body_string(body))
        .mount(&server)
        .await;
    let url = format!("{}/feed/private-token-abc.ics", server.uri());
    (server, url)
}

#[tokio::test]
async fn sync_stores_events_and_links_courses() {
    let (_dir, db) = db();
    let (_server, url) = serve(200, feed(TWO_EVENTS)).await;
    let report = sync_ical(&db, SOURCE, &url, &no_progress).await.unwrap();
    assert_eq!(report.events, 2);
    assert_eq!(report.matched_to_courses, 1);

    let store = Store::open(&db).unwrap();
    let from = chrono::DateTime::<chrono::Utc>::MIN_UTC;
    let to = chrono::DateTime::<chrono::Utc>::MAX_UTC;
    let events = store.list_events(from, to, None).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].title, "Problem Set 1");
    assert_eq!(
        events[0].course_id.as_deref(),
        Some("folder:demo/course/DEMO101")
    );
}

#[tokio::test]
async fn failures_keep_existing_events_and_never_leak_the_url() {
    let (_dir, db) = db();
    let (_server, url) = serve(200, feed(TWO_EVENTS)).await;
    sync_ical(&db, SOURCE, &url, &no_progress).await.unwrap();

    for (status, kind) in [
        (401, SourceErrorKind::AuthExpiredOrRevoked),
        (403, SourceErrorKind::AuthExpiredOrRevoked),
        (404, SourceErrorKind::NotFound),
        (410, SourceErrorKind::NotFound),
        // A server having trouble may recover by itself: like a network failure.
        (500, SourceErrorKind::Network),
        (503, SourceErrorKind::Network),
        (302, SourceErrorKind::Other),
    ] {
        let (_server, url) = serve(status, String::new()).await;
        let err = sync_ical(&db, SOURCE, &url, &no_progress)
            .await
            .unwrap_err();
        assert_eq!(err.kind, kind, "HTTP {status}");
        assert!(
            !err.message.contains("private-token-abc"),
            "{}",
            err.message
        );
    }
    let store = Store::open(&db).unwrap();
    let all = store
        .list_events(
            chrono::DateTime::<chrono::Utc>::MIN_UTC,
            chrono::DateTime::<chrono::Utc>::MAX_UTC,
            None,
        )
        .unwrap();
    assert_eq!(all.len(), 2, "existing events kept");
}

#[tokio::test]
async fn unreachable_server_is_a_network_error_without_the_url() {
    // Bind and release a port so nothing listens on it (wiremock servers are pooled and
    // keep listening after being dropped, so they can't be used for this).
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let url = format!("http://127.0.0.1:{port}/feed/private-token-abc.ics");
    let err = fetch_ical(&url).await.unwrap_err();
    assert_eq!(err.kind, SourceErrorKind::Network);
    assert!(
        !err.message.contains("private-token-abc"),
        "{}",
        err.message
    );
}

#[tokio::test]
async fn non_calendar_and_oversized_bodies_are_rejected() {
    let (_server, url) = serve(200, "<html>login page</html>".into()).await;
    let err = fetch_ical(&url).await.unwrap_err();
    assert!(
        err.message.contains("did not return a calendar"),
        "{}",
        err.message
    );

    let huge = format!("BEGIN:VCALENDAR\r\n{}", "X".repeat(10 * 1024 * 1024 + 1));
    let (_server, url) = serve(200, huge).await;
    let err = fetch_ical(&url).await.unwrap_err();
    assert!(err.message.contains("larger than 10 MB"), "{}", err.message);
}

#[tokio::test]
async fn redirects_never_send_the_feed_url_as_referer_and_stay_on_https() {
    let calendar = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/moved.ics"))
        .respond_with(ResponseTemplate::new(200).set_body_string(feed(TWO_EVENTS)))
        .mount(&calendar)
        .await;
    let feed_host = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed/private-token-abc.ics"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("Location", format!("{}/moved.ics", calendar.uri()).as_str()),
        )
        .mount(&feed_host)
        .await;
    Mock::given(method("GET"))
        .and(path("/feed/downgrade.ics"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("Location", "http://calendar.example.edu/x.ics"),
        )
        .mount(&feed_host)
        .await;
    Mock::given(method("GET"))
        .and(path("/feed/loop.ics"))
        .respond_with(ResponseTemplate::new(302).insert_header("Location", "/feed/loop.ics"))
        .mount(&feed_host)
        .await;

    let text = fetch_ical(&format!("{}/feed/private-token-abc.ics", feed_host.uri()))
        .await
        .unwrap();
    assert!(text.contains("BEGIN:VCALENDAR"));
    for request in calendar.received_requests().await.unwrap() {
        assert!(!request.headers.contains_key("referer"), "{request:?}");
    }

    for (bad, message) in [
        (
            "downgrade.ics",
            "The calendar feed redirected to an insecure (http) address.",
        ),
        ("loop.ics", "The calendar feed redirected too many times."),
    ] {
        let err = fetch_ical(&format!("{}/feed/{bad}", feed_host.uri()))
            .await
            .unwrap_err();
        // Refused by the policy, not failed on the network (a regressed policy would try
        // to reach calendar.example.edu and fail with a DNS error instead).
        assert_eq!(err.kind, SourceErrorKind::Other, "{}", err.message);
        assert!(err.invalid_input, "{}", err.message);
        assert_eq!(err.message, message);
    }
    let loops = feed_host
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/feed/loop.ics")
        .count();
    assert_eq!(loops, 6, "the first request plus at most 5 redirects");
}

#[tokio::test]
async fn sync_future_is_send() {
    fn assert_send<T: Send>(_: &T) {}
    let (_dir, db) = db();
    assert_send(&sync_ical(
        &db,
        SOURCE,
        "https://calendar.example.edu/f.ics",
        &no_progress,
    ));
    assert_send(&fetch_ical("https://calendar.example.edu/f.ics"));
}
