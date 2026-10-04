//! Link addresses in stored text never keep a parameter that gives access to a file
//! (`pagelamp_core::scrub`). Text an earlier version stored is cleaned once, when the data is
//! opened. Temporary data dirs, in-memory secrets, synthetic data only.

use std::path::Path;
use std::sync::Arc;

use pagelamp_app::{App, SyncRequest};
use pagelamp_core::secrets::MemorySecrets;
use pagelamp_core::store::Store;

fn open(data: &Path) -> App {
    App::open_at_with_secrets(data.to_path_buf(), Arc::new(MemorySecrets::new())).unwrap()
}

fn store(data: &Path) -> Store {
    Store::open(&data.join("pagelamp.db")).unwrap()
}

/// Every chunk's text and every course's syllabus text.
fn stored_text(data: &Path) -> String {
    let store = store(data);
    let mut all = String::new();
    for sql in [
        "SELECT text FROM chunks ORDER BY id",
        "SELECT COALESCE(syllabus_text, '') FROM courses ORDER BY id",
    ] {
        let mut statement = store.conn().prepare(sql).unwrap();
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap();
        for text in rows {
            all.push_str(&text.unwrap());
            all.push('\n');
        }
    }
    all
}

#[tokio::test]
async fn opening_the_data_cleans_text_stored_by_an_earlier_version() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let courses = temp.path().join("Courses");
    std::fs::create_dir_all(courses.join("DEMO101 Intro")).unwrap();
    // A note with a pasted address: cleaned as it is stored, whatever the file type.
    std::fs::write(
        courses.join("DEMO101 Intro/notes.md"),
        "# Week 1\nlambdaword slides: https://lms.example.edu/courses/1/files/5/download?verifier=SECRET-NOTE&wrap=1\n",
    )
    .unwrap();
    let app = open(&data);
    app.add_folder_source(&courses, None, None).unwrap();
    assert!(
        app.sync_all(SyncRequest::default(), |_| {})
            .await
            .unwrap()
            .ok
    );
    let text = stored_text(&data);
    assert!(
        text.contains("https://lms.example.edu/courses/1/files/5/download\n")
            || text.contains("https://lms.example.edu/courses/1/files/5/download "),
        "{text}"
    );
    assert!(
        !text.contains("SECRET") && !text.contains("verifier"),
        "{text}"
    );

    // What an earlier version could have stored: an address with its parameter, in a chunk
    // and in a syllabus. That version never recorded a clean-up.
    {
        let store = store(&data);
        store
            .conn()
            .execute(
                "UPDATE chunks SET text = text || ' handout (https://lms.example.edu/files/7/preview?verifier=SECRET-OLD) and video https://media.example.edu/v?t=5&access_token=SECRET-OLD'",
                [],
            )
            .unwrap();
        store
            .conn()
            .execute(
                "UPDATE courses SET syllabus_text = 'Outline: https://lms.example.edu/courses/1/files/9/download?sf_verifier=SECRET-OLD'",
                [],
            )
            .unwrap();
        store.remove_setting("text.scrubbed").unwrap();
        assert_eq!(store.search("SECRET", None, 5).unwrap().len(), 1);
    }
    assert!(stored_text(&data).contains("SECRET-OLD"));

    // The next start of an app or the CLI cleans it, the search index included, and keeps
    // everything else of the text.
    let app = open(&data);
    let text = stored_text(&data);
    assert!(
        !text.contains("SECRET") && !text.contains("verifier"),
        "{text}"
    );
    assert!(!text.contains("access_token"), "{text}");
    assert!(
        text.contains("handout (https://lms.example.edu/files/7/preview)"),
        "{text}"
    );
    assert!(
        text.contains("video https://media.example.edu/v?t=5"),
        "{text}"
    );
    assert!(
        text.contains("Outline: https://lms.example.edu/courses/1/files/9/download"),
        "{text}"
    );
    let store = store(&data);
    assert!(store.search("SECRET", None, 5).unwrap().is_empty());
    assert_eq!(store.search("lambdaword", None, 5).unwrap().len(), 1);
    assert_eq!(store.search("handout", None, 5).unwrap().len(), 1);
    let hits = app.search("handout", None, 5).unwrap();
    assert!(hits.iter().all(|hit| !hit.snippet.contains("SECRET")));

    // It is done once per version of the rules: text written past the rules afterwards is
    // left to the output side (this only shows the clean-up doesn't run on every start).
    store
        .conn()
        .execute(
            "UPDATE courses SET syllabus_text = 'https://lms.example.edu/files/9/download?verifier=SECRET-LATER'",
            [],
        )
        .unwrap();
    drop(store);
    let _again = open(&data);
    assert!(stored_text(&data).contains("SECRET-LATER"));
}

/// The clean-up can't be skipped: while another process holds the database, the data isn't
/// opened (this process would show what must not be shown), the error says why, and nothing
/// is recorded. Afterwards it opens and cleans.
#[tokio::test]
async fn a_held_database_refuses_the_open_with_a_reason() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    drop(open(&data));
    {
        let store = store(&data);
        store
            .upsert_source(&pagelamp_core::model::SourceRecord {
                id: "folder:demo".into(),
                kind: pagelamp_core::model::SourceKind::Folder,
                label: "Demo".into(),
                config: serde_json::json!({ "path": "/demo" }),
                last_synced_at: None,
                last_error: None,
                last_error_kind: None,
            })
            .unwrap();
        store
            .conn()
            .execute(
                "INSERT INTO courses (id, source_id, external_id, code, name, syllabus_text, updated_at)
                 VALUES ('folder:demo/course/1', 'folder:demo', '1', 'DEMO101', 'Intro',
                         'Outline: https://lms.example.edu/files/9/download?verifier=Ab12Cd34Zz',
                         '2026-09-20T00:00:00Z')",
                [],
            )
            .unwrap();
        store.remove_setting("text.scrubbed").unwrap();
    }
    let other = rusqlite::Connection::open(data.join("pagelamp.db")).unwrap();
    other.execute_batch("BEGIN IMMEDIATE").unwrap();
    let Err(refused) = App::open_at_with_secrets(data.clone(), Arc::new(MemorySecrets::new()))
    else {
        panic!("opened while the database was held");
    };
    assert_eq!(refused.kind, pagelamp_app::AppErrorKind::Busy);
    assert!(
        refused
            .message
            .contains("another PageLamp window or command")
            && !refused.message.contains("database is locked"),
        "{}",
        refused.message
    );
    other.execute_batch("ROLLBACK").unwrap();
    drop(other);
    assert_eq!(store(&data).scrubbed_text_version().unwrap(), 0);
    assert!(stored_text(&data).contains("Ab12Cd34Zz"));

    let _app = open(&data);
    assert!(!stored_text(&data).contains("Ab12Cd34Zz"));
    assert_eq!(
        store(&data).scrubbed_text_version().unwrap(),
        pagelamp_core::scrub::VERSION
    );
}
