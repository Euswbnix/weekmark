//! A material's local file for the shells (calendar design §7.10): only inside its source's
//! root, only a regular file that exists, and to open it only a document. Synthetic files only.

use std::path::Path;
use std::sync::Arc;

use pagelamp_app::{App, AppErrorKind, LocalFileUse};
use pagelamp_core::model::*;
use pagelamp_core::paths;
use pagelamp_core::secrets::MemorySecrets;
use pagelamp_core::store::Store;
use serde_json::json;

const FOLDER: &str = "folder:demo";
const CANVAS: &str = "canvas:lms.example.edu";

fn add_source(store: &Store, id: &str, kind: SourceKind, config: serde_json::Value) {
    store
        .upsert_source(&SourceRecord {
            id: id.into(),
            kind,
            label: id.into(),
            config,
            last_synced_at: None,
            last_error: None,
            last_error_kind: None,
        })
        .unwrap();
    store
        .upsert_course(&CourseUpsert {
            id: format!("{id}/course/DEMO101"),
            source_id: id.into(),
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

fn add_file(store: &Store, source: &str, name: &str, kind: MaterialKind, path: &Path) -> String {
    let id = format!("{source}/material/{name}");
    store
        .upsert_material(&MaterialUpsert {
            id: id.clone(),
            course_id: format!("{source}/course/DEMO101"),
            module_id: None,
            kind,
            title: name.into(),
            url: None,
            local_path: Some(path.to_string_lossy().into_owned()),
            mime: None,
            published_at: None,
            week_hint: None,
        })
        .unwrap();
    id
}

struct Fixture {
    _temp: tempfile::TempDir,
    app: App,
    root: std::path::PathBuf,
    outside: std::path::PathBuf,
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let app = App::open_at_with_secrets(temp.path().join("data"), Arc::new(MemorySecrets::new()))
        .unwrap();
    let root = temp.path().join("courses");
    std::fs::create_dir_all(root.join("DEMO101/Tool.app/Contents")).unwrap();
    for name in ["Outline.pdf", "run.command", "setup.exe", "gone.pdf"] {
        std::fs::write(root.join("DEMO101").join(name), b"demo").unwrap();
    }
    let outside = temp.path().join("elsewhere");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("private.pdf"), b"demo").unwrap();
    let store = Store::open(&app.db_path()).unwrap();
    add_source(&store, FOLDER, SourceKind::Folder, json!({ "path": root }));
    add_source(
        &store,
        CANVAS,
        SourceKind::Canvas,
        json!({ "base_url": "https://lms.example.edu" }),
    );
    Fixture {
        _temp: temp,
        app,
        root,
        outside,
    }
}

fn file(f: &Fixture, id: &str, purpose: LocalFileUse) -> Option<String> {
    f.app.material_local_file(id, purpose).unwrap()
}

#[test]
fn a_folder_document_opens_and_executables_never_do() {
    let f = fixture();
    let store = Store::open(&f.app.db_path()).unwrap();
    let course = f.root.join("DEMO101");
    let pdf = add_file(
        &store,
        FOLDER,
        "Outline.pdf",
        MaterialKind::File,
        &course.join("Outline.pdf"),
    );
    let opened = file(&f, &pdf, LocalFileUse::Open).expect("a PDF in the folder");
    assert!(opened.ends_with("Outline.pdf"));
    assert_eq!(
        Path::new(&opened),
        std::fs::canonicalize(course.join("Outline.pdf")).unwrap()
    );

    for name in ["run.command", "setup.exe"] {
        let id = add_file(&store, FOLDER, name, MaterialKind::File, &course.join(name));
        assert_eq!(file(&f, &id, LocalFileUse::Open), None, "{name}");
        // Revealing runs nothing.
        assert!(file(&f, &id, LocalFileUse::Reveal).is_some(), "{name}");
    }
    // A bundle is a directory, not a file.
    let app_bundle = add_file(
        &store,
        FOLDER,
        "Tool.app",
        MaterialKind::File,
        &course.join("Tool.app"),
    );
    assert_eq!(file(&f, &app_bundle, LocalFileUse::Open), None);
    assert_eq!(file(&f, &app_bundle, LocalFileUse::Reveal), None);
    // A deleted file.
    let gone = add_file(
        &store,
        FOLDER,
        "gone.pdf",
        MaterialKind::File,
        &course.join("gone.pdf"),
    );
    std::fs::remove_file(course.join("gone.pdf")).unwrap();
    assert_eq!(file(&f, &gone, LocalFileUse::Open), None);
    // A stored path outside the root.
    let outside = add_file(
        &store,
        FOLDER,
        "private.pdf",
        MaterialKind::File,
        &f.outside.join("private.pdf"),
    );
    assert_eq!(file(&f, &outside, LocalFileUse::Reveal), None);
    // Only files.
    let page = add_file(
        &store,
        FOLDER,
        "page.pdf",
        MaterialKind::Page,
        &course.join("Outline.pdf"),
    );
    assert_eq!(file(&f, &page, LocalFileUse::Open), None);
    // Unknown materials are NotFound.
    let err = f
        .app
        .material_local_file("nope", LocalFileUse::Open)
        .unwrap_err();
    assert_eq!(err.kind, AppErrorKind::NotFound);
}

#[cfg(unix)]
#[test]
fn a_symlink_out_of_the_root_is_refused() {
    let f = fixture();
    let store = Store::open(&f.app.db_path()).unwrap();
    let link = f.root.join("DEMO101/escape.pdf");
    std::os::unix::fs::symlink(f.outside.join("private.pdf"), &link).unwrap();
    let id = add_file(&store, FOLDER, "escape.pdf", MaterialKind::File, &link);
    assert_eq!(file(&f, &id, LocalFileUse::Open), None);
    assert_eq!(file(&f, &id, LocalFileUse::Reveal), None);
    // A link named like a document to a script inside the root: the target's type decides.
    let disguised = f.root.join("DEMO101/notes.pdf");
    std::os::unix::fs::symlink(f.root.join("DEMO101/run.command"), &disguised).unwrap();
    let id = add_file(&store, FOLDER, "notes.pdf", MaterialKind::File, &disguised);
    assert_eq!(file(&f, &id, LocalFileUse::Open), None);
}

#[test]
fn a_canvas_download_opens_from_the_cache_only() {
    let f = fixture();
    let store = Store::open(&f.app.db_path()).unwrap();
    let cache = paths::files_dir_in(f.app.data_dir()).join("DEMO101-1");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(cache.join("Syllabus.pdf"), b"demo").unwrap();
    let cached = add_file(
        &store,
        CANVAS,
        "Syllabus.pdf",
        MaterialKind::File,
        &cache.join("Syllabus.pdf"),
    );
    assert!(file(&f, &cached, LocalFileUse::Open).is_some());
    // A Canvas material pointing outside the cache (e.g. into a folder source).
    let stray = add_file(
        &store,
        CANVAS,
        "Outline.pdf",
        MaterialKind::File,
        &f.root.join("DEMO101/Outline.pdf"),
    );
    assert_eq!(file(&f, &stray, LocalFileUse::Open), None);
}
