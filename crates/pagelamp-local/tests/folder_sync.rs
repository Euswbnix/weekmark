//! Folder source against real temporary directory trees (synthetic course content).

use std::fs;
use std::path::Path;
use std::sync::Mutex;

use pagelamp_core::ingest::Extractor;
use pagelamp_core::model::*;
use pagelamp_core::source::{SyncProgress, SyncStage, no_progress};
use pagelamp_core::store::Store;
use pagelamp_local::sync_folder;
use serde_json::json;

const SOURCE: &str = "folder:demo";

fn store() -> Store {
    let store = Store::open_in_memory().unwrap();
    store
        .upsert_source(&SourceRecord {
            id: SOURCE.into(),
            kind: SourceKind::Folder,
            label: "~/Courses".into(),
            config: json!({}),
            last_synced_at: None,
            last_error: None,
            last_error_kind: None,
        })
        .unwrap();
    store
}

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

/// DEMO101 with weeks, a nested folder, an unsupported file, hidden files and metadata.
fn demo_tree(root: &Path) {
    write(
        root,
        "DEMO101H1 Intro to Demo Studies/Week 1/notes.md",
        "# Basics\nphotosynthesis basics",
    );
    write(
        root,
        "DEMO101H1 Intro to Demo Studies/Week 2/Lectures/methods.txt",
        "chlorophyll methods",
    );
    write(
        root,
        "DEMO101H1 Intro to Demo Studies/Readings/week03-reading.md",
        "light reactions",
    );
    write(
        root,
        "DEMO101H1 Intro to Demo Studies/syllabus.txt",
        "grading policy",
    );
    write(
        root,
        "DEMO101H1 Intro to Demo Studies/recording.mp4",
        "not really a video",
    );
    write(root, "DEMO101H1 Intro to Demo Studies/.DS_Store", "junk");
    write(root, "DEMO101H1 Intro to Demo Studies/.git/config", "junk");
    write(
        root,
        "DEMO101H1 Intro to Demo Studies/course.toml",
        "term_start = 2026-09-07\n",
    );
    write(root, "Personal Notes/todo.txt", "buy notebook");
    write(root, ".hidden course/x.txt", "x");
}

fn material_titles(store: &Store, course_id: &str) -> Vec<String> {
    let mut titles: Vec<String> = store
        .list_materials(course_id)
        .unwrap()
        .into_iter()
        .map(|m| m.title)
        .collect();
    titles.sort();
    titles
}

const DEMO: &str = "folder:demo/course/DEMO101H1 Intro to Demo Studies";

#[test]
fn syncs_courses_modules_and_materials() {
    let temp = tempfile::tempdir().unwrap();
    demo_tree(temp.path());
    let store = store();
    let events = Mutex::new(Vec::new());
    let report = sync_folder(
        &store,
        SOURCE,
        temp.path(),
        None,
        &Extractor::default(),
        &|p| events.lock().unwrap().push(p),
    )
    .unwrap();

    assert_eq!(report.courses, 2);
    assert_eq!(report.files_indexed, 5);
    assert_eq!(report.materials, 6); // 5 in DEMO101 + 1 personal note

    let courses = store.list_courses(true).unwrap();
    let demo = courses.iter().find(|c| c.id == DEMO).unwrap();
    assert_eq!(demo.code.as_deref(), Some("DEMO101H1"));
    assert_eq!(demo.name, "DEMO101H1 Intro to Demo Studies");
    assert_eq!(
        demo.term_start.map(|d| d.to_string()).as_deref(),
        Some("2026-09-07")
    );
    let notes = courses.iter().find(|c| c.name == "Personal Notes").unwrap();
    assert_eq!(notes.code, None);
    assert!(!courses.iter().any(|c| c.name.starts_with('.')));

    let modules: Vec<(String, Option<u32>)> = store
        .list_modules(DEMO)
        .unwrap()
        .into_iter()
        .map(|m| (m.name, m.week_hint))
        .collect();
    assert_eq!(
        modules,
        [
            ("Readings".to_string(), None),
            ("Week 1".to_string(), Some(1)),
            ("Week 2".to_string(), Some(2))
        ]
    );

    assert_eq!(
        material_titles(&store, DEMO),
        [
            "methods.txt",
            "notes.md",
            "recording.mp4",
            "syllabus.txt",
            "week03-reading.md"
        ]
    );
    let materials = store.list_materials(DEMO).unwrap();
    let by_title = |t: &str| materials.iter().find(|m| m.title == t).unwrap();
    // Week from the file name, else the nearest week folder (nested dirs belong to the
    // top-level module).
    assert_eq!(by_title("week03-reading.md").week_hint, Some(3));
    assert_eq!(by_title("methods.txt").week_hint, Some(2));
    assert!(
        by_title("methods.txt")
            .module_id
            .as_deref()
            .unwrap()
            .ends_with("/Week 2")
    );
    assert_eq!(by_title("syllabus.txt").module_id, None);
    assert_eq!(
        by_title("recording.mp4").text_status,
        TextStatus::Unsupported
    );
    assert_eq!(by_title("recording.mp4").content_hash, None, "never read");
    assert_eq!(by_title("notes.md").text_status, TextStatus::Ok);
    assert!(
        by_title("notes.md")
            .url
            .as_deref()
            .unwrap()
            .starts_with("file://")
    );
    assert!(by_title("notes.md").published_at.is_some());
    assert_eq!(
        by_title("notes.md").id,
        "folder:demo/file/DEMO101H1 Intro to Demo Studies/Week 1/notes.md"
    );

    assert_eq!(store.search("chlorophyll", None, 10).unwrap().len(), 1);
    let events = events.into_inner().unwrap();
    assert!(events.iter().any(|e| matches!(
        e,
        SyncProgress::Step {
            current: Some(_),
            total: Some(_),
            ..
        }
    )));
    // Every step carries its code and course, so the UIs can translate it.
    for stage in [SyncStage::ScanningFiles, SyncStage::IndexingFiles] {
        assert!(
            events.iter().any(|e| matches!(
                e,
                SyncProgress::Step { stage: Some(s), course: Some(c), .. }
                    if *s == stage && c.starts_with("DEMO101")
            )),
            "{stage:?}: {events:?}"
        );
    }
}

#[test]
fn resync_is_cheap_and_prunes_what_disappeared() {
    let temp = tempfile::tempdir().unwrap();
    demo_tree(temp.path());
    let store = store();
    sync_folder(
        &store,
        SOURCE,
        temp.path(),
        None,
        &Extractor::default(),
        &no_progress,
    )
    .unwrap();

    let again = sync_folder(
        &store,
        SOURCE,
        temp.path(),
        None,
        &Extractor::default(),
        &no_progress,
    )
    .unwrap();
    assert_eq!(again.files_indexed, 0);
    assert_eq!(again.files_unchanged, 5);

    fs::remove_file(
        temp.path()
            .join("DEMO101H1 Intro to Demo Studies/Week 2/Lectures/methods.txt"),
    )
    .unwrap();
    fs::remove_dir_all(temp.path().join("Personal Notes")).unwrap();
    write(
        temp.path(),
        "DEMO101H1 Intro to Demo Studies/Week 1/notes.md",
        "# Basics\nstomata only",
    );
    let third = sync_folder(
        &store,
        SOURCE,
        temp.path(),
        None,
        &Extractor::default(),
        &no_progress,
    )
    .unwrap();
    assert_eq!(third.courses, 1);
    assert_eq!(third.files_indexed, 1);
    assert!(!material_titles(&store, DEMO).contains(&"methods.txt".to_string()));
    assert!(store.search("chlorophyll", None, 10).unwrap().is_empty());
    assert!(store.search("photosynthesis", None, 10).unwrap().is_empty());
    assert_eq!(store.search("stomata", None, 10).unwrap().len(), 1);
    assert_eq!(store.list_courses(true).unwrap().len(), 1);
}

#[test]
fn ids_survive_moving_the_root_and_user_settings_survive_resync() {
    let temp = tempfile::tempdir().unwrap();
    let first_root = temp.path().join("Fall");
    demo_tree(&first_root);
    let store = store();
    sync_folder(
        &store,
        SOURCE,
        &first_root,
        None,
        &Extractor::default(),
        &no_progress,
    )
    .unwrap();
    store
        .set_course_policy(DEMO, AiPolicy::LearningAid, None)
        .unwrap();

    let moved = temp.path().join("Fall 2026 (moved)");
    fs::rename(&first_root, &moved).unwrap();
    sync_folder(
        &store,
        SOURCE,
        &moved,
        None,
        &Extractor::default(),
        &no_progress,
    )
    .unwrap();
    let course = store.get_course(DEMO).unwrap().unwrap();
    assert_eq!(course.ai_policy, AiPolicy::LearningAid);
    assert_eq!(store.list_materials(DEMO).unwrap().len(), 5);
}

#[test]
fn source_term_start_is_the_fallback_and_bad_course_files_only_warn() {
    let temp = tempfile::tempdir().unwrap();
    write(temp.path(), "DEMO202 Advanced/notes.txt", "advanced topics");
    write(temp.path(), "DEMO303 Seminar/course.json", "{ not json");
    write(temp.path(), "DEMO303 Seminar/notes.txt", "seminar topics");
    let store = store();
    let default = chrono::NaiveDate::from_ymd_opt(2026, 9, 8);
    let report = sync_folder(
        &store,
        SOURCE,
        temp.path(),
        default,
        &Extractor::default(),
        &no_progress,
    )
    .unwrap();
    assert!(
        report.warnings.iter().any(|w| w.contains("course.json")),
        "{:?}",
        report.warnings
    );
    for course in store.list_courses(true).unwrap() {
        assert_eq!(course.term_start, default, "{}", course.name);
    }
}

#[test]
fn missing_root_is_not_found_and_changes_nothing() {
    let temp = tempfile::tempdir().unwrap();
    demo_tree(temp.path());
    let store = store();
    sync_folder(
        &store,
        SOURCE,
        temp.path(),
        None,
        &Extractor::default(),
        &no_progress,
    )
    .unwrap();
    let err = sync_folder(
        &store,
        SOURCE,
        &temp.path().join("gone"),
        None,
        &Extractor::default(),
        &no_progress,
    )
    .unwrap_err();
    assert_eq!(err.kind, SourceErrorKind::NotFound);
    assert_eq!(
        err.message,
        "The course folder 'gone' does not exist or cannot be read."
    );
    assert_eq!(store.list_courses(true).unwrap().len(), 2, "nothing pruned");
}

#[cfg(unix)]
#[test]
fn symlinks_are_not_followed() {
    let temp = tempfile::tempdir().unwrap();
    let outside = temp.path().join("outside");
    write(&outside, "secret.txt", "private diary");
    let root = temp.path().join("root");
    write(&root, "DEMO101 Intro/notes.txt", "course notes");
    std::os::unix::fs::symlink(&outside, root.join("DEMO101 Intro/linked")).unwrap();
    std::os::unix::fs::symlink(
        outside.join("secret.txt"),
        root.join("DEMO101 Intro/secret.txt"),
    )
    .unwrap();
    std::os::unix::fs::symlink(&outside, root.join("Linked Course")).unwrap();
    let store = store();
    sync_folder(
        &store,
        SOURCE,
        &root,
        None,
        &Extractor::default(),
        &no_progress,
    )
    .unwrap();
    assert_eq!(store.list_courses(true).unwrap().len(), 1);
    assert!(store.search("diary", None, 10).unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn unreadable_subfolders_warn_and_prevent_pruning() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    write(
        temp.path(),
        "DEMO101 Intro/Week 1/notes.txt",
        "week one notes",
    );
    write(
        temp.path(),
        "DEMO101 Intro/Week 2/more.txt",
        "week two notes",
    );
    let store = store();
    sync_folder(
        &store,
        SOURCE,
        temp.path(),
        None,
        &Extractor::default(),
        &no_progress,
    )
    .unwrap();
    let course_id = "folder:demo/course/DEMO101 Intro";
    assert_eq!(store.list_materials(course_id).unwrap().len(), 2);

    let locked = temp.path().join("DEMO101 Intro/Week 2");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let readable_as_root = fs::read_dir(&locked).is_ok(); // e.g. tests running as root
    let report = sync_folder(
        &store,
        SOURCE,
        temp.path(),
        None,
        &Extractor::default(),
        &no_progress,
    )
    .unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    if !readable_as_root {
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.contains("nothing was removed"))
        );
        assert_eq!(store.list_materials(course_id).unwrap().len(), 2);
    }
}

#[test]
fn published_at_is_the_file_modification_time_not_the_sync_time() {
    let temp = tempfile::tempdir().unwrap();
    write(temp.path(), "DEMO101 Intro/Week 1/notes.txt", "demo notes");
    let modified = chrono::DateTime::parse_from_rfc3339("2026-01-15T14:30:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    fs::File::options()
        .write(true)
        .open(temp.path().join("DEMO101 Intro/Week 1/notes.txt"))
        .unwrap()
        .set_modified(modified.into())
        .unwrap();

    let store = store();
    for _ in 0..2 {
        sync_folder(
            &store,
            SOURCE,
            temp.path(),
            None,
            &Extractor::default(),
            &no_progress,
        )
        .unwrap();
        let materials = store
            .list_materials(&format!("{SOURCE}/course/DEMO101 Intro"))
            .unwrap();
        assert_eq!(materials[0].published_at, Some(modified));
    }
}

#[test]
fn a_stopped_sync_ends_before_the_next_file_and_keeps_what_was_done() {
    let temp = tempfile::tempdir().unwrap();
    let course = temp.path().join("DEMO101 Intro to Demo Studies");
    fs::create_dir_all(&course).unwrap();
    for n in 1..=5 {
        fs::write(
            course.join(format!("notes-{n}.md")),
            format!("# Week {n}\nkappaword {n}"),
        )
        .unwrap();
    }
    let store = store();
    let cancel = pagelamp_core::source::CancelFlag::new();
    let extractor = Extractor::default().cancellable(cancel.clone());
    // Stop as soon as the second file is announced.
    let err = sync_folder(&store, SOURCE, temp.path(), None, &extractor, &|progress| {
        if let SyncProgress::Step {
            current: Some(2), ..
        } = progress
        {
            cancel.cancel();
        }
    })
    .unwrap_err();
    assert!(err.cancelled, "{err:?}");
    let indexed = store
        .list_materials(&format!("{SOURCE}/course/DEMO101 Intro to Demo Studies"))
        .unwrap()
        .into_iter()
        .filter(|m| m.text_status == TextStatus::Ok)
        .count();
    assert_eq!(
        indexed, 1,
        "file 1 stays indexed; file 2 was stopped before it was read"
    );
}

/// The flag `course.toml`'s `outline` sets, by material title.
fn named_outlines(store: &Store, course_id: &str) -> Vec<String> {
    let mut statement = store
        .conn()
        .prepare("SELECT title FROM materials WHERE course_id = ?1 AND named_outline = 1")
        .unwrap();
    statement
        .query_map([course_id], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn course_toml_names_the_outline_and_a_missing_one_only_warns() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(
        root,
        "DEMO707 Field Methods/Admin/Course outline.md",
        "outline",
    );
    write(root, "DEMO707 Field Methods/notes.txt", "notes");
    write(
        root,
        "DEMO707 Field Methods/course.toml",
        "outline = \"./Admin/Course outline.md\"\n",
    );
    let store = store();
    let extractor = Extractor::default();
    let report = sync_folder(&store, SOURCE, root, None, &extractor, &no_progress).unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    let course = format!("{SOURCE}/course/DEMO707 Field Methods");
    assert_eq!(named_outlines(&store, &course), ["Course outline.md"]);

    write(
        root,
        "DEMO707 Field Methods/course.toml",
        "outline = \"gone.pdf\"\n",
    );
    let report = sync_folder(&store, SOURCE, root, None, &extractor, &no_progress).unwrap();
    assert!(
        report.warnings.iter().any(|w| w.contains("gone.pdf")),
        "{:?}",
        report.warnings
    );
    assert!(named_outlines(&store, &course).is_empty());
}

/// `institution = "uoft"` in course.toml opts the folder course into UofT's session codes and
/// calendar (calendar design §6.3, D50): sync writes it, and a later sync without it clears it;
/// a school PageLamp doesn't know is a warning, never stored.
#[test]
fn course_toml_names_the_institution() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "DEM332H5 Demo Methods/notes.txt", "notes");
    write(
        root,
        "DEM332H5 Demo Methods/course.toml",
        "institution = \"UofT\"\n",
    );
    let store = store();
    let extractor = Extractor::default();
    let report = sync_folder(&store, SOURCE, root, None, &extractor, &no_progress).unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    let course = format!("{SOURCE}/course/DEM332H5 Demo Methods");
    let institution = |store: &Store| {
        store
            .course_term_data(&course)
            .unwrap()
            .unwrap()
            .institution
    };
    assert_eq!(institution(&store).as_deref(), Some("uoft"));

    write(
        root,
        "DEM332H5 Demo Methods/course.toml",
        "institution = \"elsewhere\"\n",
    );
    let report = sync_folder(&store, SOURCE, root, None, &extractor, &no_progress).unwrap();
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("institution \"elsewhere\" isn't a school PageLamp knows")),
        "{:?}",
        report.warnings
    );
    assert_eq!(institution(&store), None);
}
