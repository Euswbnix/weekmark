//! Tests of `pagelamp_core::ingest` on an in-memory store with tiny files in temp dirs.
//! All data is synthetic ("DEMO101 Intro to Demo Studies").

use std::path::{Path, PathBuf};

use pagelamp_core::Error;
use pagelamp_core::ingest::{
    IndexOutcome, NO_TEXT_NOTE, index_file, index_html, index_text, sha256_file, sha256_hex,
};
use pagelamp_core::model::*;
use pagelamp_core::store::Store;
use tempfile::TempDir;

// ----- fixtures -----------------------------------------------------------------------------

const SOURCE: &str = "folder:demo";
const COURSE: &str = "folder:demo/course/DEMO101";

/// In-memory store with one source and one course ("DEMO101 Intro to Demo Studies").
fn demo_store() -> Store {
    let store = Store::open_in_memory().unwrap();
    store
        .upsert_source(&SourceRecord {
            id: SOURCE.to_string(),
            kind: SourceKind::Folder,
            label: "Demo courses".to_string(),
            config: serde_json::json!({ "path": "/demo/courses" }),
            last_synced_at: None,
            last_error: None,
            last_error_kind: None,
        })
        .unwrap();
    store
        .upsert_course(&CourseUpsert {
            id: COURSE.to_string(),
            source_id: SOURCE.to_string(),
            external_id: "DEMO101".to_string(),
            code: Some("DEMO101".to_string()),
            name: "Intro to Demo Studies".to_string(),
            term_start: None,
            term_end: None,
            url: None,
            syllabus_text: None,
            lms: Default::default(),
        })
        .unwrap();
    store
}

fn material_upsert(name: &str, local_path: Option<&Path>) -> MaterialUpsert {
    MaterialUpsert {
        id: format!("{COURSE}/material/{name}"),
        course_id: COURSE.to_string(),
        module_id: None,
        kind: MaterialKind::File,
        title: name.to_string(),
        url: None,
        local_path: local_path.map(|path| path.display().to_string()),
        mime: None,
        published_at: None,
        week_hint: Some(1),
    }
}

/// Insert a material row (text_status pending) and return its id.
fn add_material(store: &Store, name: &str) -> String {
    let material = material_upsert(name, None);
    store.upsert_material(&material).unwrap();
    material.id
}

fn write_file(dir: &TempDir, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

fn material(store: &Store, id: &str) -> Material {
    store.get_material(id).unwrap().unwrap()
}

fn locators(chunks: &[Chunk]) -> Vec<Option<&str>> {
    chunks
        .iter()
        .map(|chunk| chunk.locator.as_deref())
        .collect()
}

fn ords(chunks: &[Chunk]) -> Vec<u32> {
    chunks.iter().map(|chunk| chunk.ord).collect()
}

/// Material ids of the search hits for `query`.
fn search_ids(store: &Store, query: &str) -> Vec<String> {
    store
        .search(query, None, 10)
        .unwrap()
        .into_iter()
        .map(|hit| hit.material_id)
        .collect()
}

const WEEK1_MARKDOWN: &str = "Notes for DEMO101 Intro to Demo Studies.\n\
    \n\
    # Week 1: Demo Basics\n\
    \n\
    The zymurgy lecture covers demo fermentation.\n\
    \n\
    ## Readings\n\
    \n\
    Chapter one of the quokkanomics textbook.\n";

// ----- index_file ---------------------------------------------------------------------------

#[test]
fn markdown_file_is_indexed_with_locators_and_searchable() {
    let store = demo_store();
    let dir = tempfile::tempdir().unwrap();
    let path = write_file(&dir, "week1.md", WEEK1_MARKDOWN.as_bytes());
    let id = add_material(&store, "week1.md");

    let outcome = index_file(&store, &id, &path, Some("text/markdown")).unwrap();

    assert_eq!(outcome, IndexOutcome::Indexed { chunks: 3 });
    let chunks = store.get_chunks(&id, 0, None).unwrap();
    assert_eq!(ords(&chunks), vec![0, 1, 2]);
    assert_eq!(
        locators(&chunks),
        vec![None, Some("§ Week 1: Demo Basics"), Some("§ Readings")]
    );
    assert!(chunks.iter().all(|chunk| chunk.material_id == id));

    let stored = material(&store, &id);
    assert_eq!(stored.text_status, TextStatus::Ok);
    assert_eq!(stored.text_error, None);
    assert_eq!(stored.content_hash, Some(sha256_file(&path).unwrap()));

    let hits = store.search("zymurgy", None, 10).unwrap();
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].material_id, id);
    assert_eq!(hits[0].chunk_ord, 1);
    assert_eq!(hits[0].locator.as_deref(), Some("§ Week 1: Demo Basics"));
    assert_eq!(hits[0].course_code.as_deref(), Some("DEMO101"));
}

#[test]
fn reindexing_the_same_file_is_unchanged_and_writes_nothing() {
    let store = demo_store();
    let dir = tempfile::tempdir().unwrap();
    let path = write_file(&dir, "week1.md", WEEK1_MARKDOWN.as_bytes());
    let id = add_material(&store, "week1.md");
    index_file(&store, &id, &path, None).unwrap();

    let changes_before = store.conn().total_changes();
    let outcome = index_file(&store, &id, &path, None).unwrap();

    assert_eq!(outcome, IndexOutcome::Unchanged);
    assert_eq!(store.conn().total_changes(), changes_before);
    assert_eq!(store.chunk_count(&id).unwrap(), 3);
}

#[test]
fn modified_file_is_reindexed_and_old_chunks_leave_search() {
    let store = demo_store();
    let dir = tempfile::tempdir().unwrap();
    let path = write_file(&dir, "notes.txt", b"The alphaword draft of the demo notes.");
    let id = add_material(&store, "notes.txt");
    index_file(&store, &id, &path, None).unwrap();
    assert_eq!(search_ids(&store, "alphaword"), vec![id.clone()]);
    let old_hash = material(&store, &id).content_hash;

    std::fs::write(&path, b"The betaword revision of the demo notes.").unwrap();
    let outcome = index_file(&store, &id, &path, None).unwrap();

    assert_eq!(outcome, IndexOutcome::Indexed { chunks: 1 });
    assert!(search_ids(&store, "alphaword").is_empty());
    assert_eq!(search_ids(&store, "betaword"), vec![id.clone()]);
    let new_hash = material(&store, &id).content_hash;
    assert_ne!(new_hash, old_hash);
    assert_eq!(new_hash, Some(sha256_file(&path).unwrap()));
}

#[test]
fn pending_material_is_reindexed_even_when_the_hash_is_unchanged() {
    let store = demo_store();
    let dir = tempfile::tempdir().unwrap();
    let path = write_file(&dir, "notes.txt", b"Demo notes about gammaword.");
    let id = add_material(&store, "notes.txt");
    index_file(&store, &id, &path, None).unwrap();

    // A changed local_path resets text_status to pending (the stored hash is kept).
    store
        .upsert_material(&material_upsert("notes.txt", Some(&path)))
        .unwrap();
    assert_eq!(material(&store, &id).text_status, TextStatus::Pending);

    let outcome = index_file(&store, &id, &path, None).unwrap();
    assert_eq!(outcome, IndexOutcome::Indexed { chunks: 1 });
    assert_eq!(material(&store, &id).text_status, TextStatus::Ok);
}

#[test]
fn unsupported_file_clears_chunks_and_stores_hash() {
    let store = demo_store();
    let dir = tempfile::tempdir().unwrap();
    let text = write_file(&dir, "old.txt", b"Earlier deltaword text.");
    let video = write_file(&dir, "lecture.mp4", b"not really a video");
    let id = add_material(&store, "lecture");
    index_file(&store, &id, &text, None).unwrap();
    assert_eq!(store.chunk_count(&id).unwrap(), 1);

    let outcome = index_file(&store, &id, &video, Some("video/mp4")).unwrap();

    assert_eq!(outcome, IndexOutcome::Unsupported);
    let stored = material(&store, &id);
    assert_eq!(stored.text_status, TextStatus::Unsupported);
    assert_eq!(stored.text_error, None);
    assert_eq!(stored.content_hash, Some(sha256_file(&video).unwrap()));
    assert_eq!(store.chunk_count(&id).unwrap(), 0);
    assert!(search_ids(&store, "deltaword").is_empty());

    // Only an ok status counts as "already indexed", so the same file is looked at again.
    assert_eq!(
        index_file(&store, &id, &video, Some("video/mp4")).unwrap(),
        IndexOutcome::Unsupported
    );
}

#[test]
fn corrupt_docx_fails_with_error_state_and_cleared_chunks() {
    let store = demo_store();
    let dir = tempfile::tempdir().unwrap();
    let text = write_file(&dir, "old.txt", b"Earlier epsilonword text.");
    let docx = write_file(&dir, "syllabus.docx", b"this is not a zip archive");
    let id = add_material(&store, "syllabus");
    index_file(&store, &id, &text, None).unwrap();

    let outcome = index_file(&store, &id, &docx, None).unwrap();

    let IndexOutcome::Failed(message) = outcome else {
        panic!("expected Failed, got {outcome:?}");
    };
    assert!(!message.is_empty());
    let stored = material(&store, &id);
    assert_eq!(stored.text_status, TextStatus::Error);
    assert_eq!(stored.text_error.as_deref(), Some(message.as_str()));
    assert_eq!(stored.content_hash, Some(sha256_file(&docx).unwrap()));
    assert_eq!(store.chunk_count(&id).unwrap(), 0);
    assert!(search_ids(&store, "epsilonword").is_empty());

    // Failed materials are retried on the next sync, not reported as unchanged.
    assert!(matches!(
        index_file(&store, &id, &docx, None).unwrap(),
        IndexOutcome::Failed(_)
    ));
}

#[test]
fn empty_text_file_is_empty_with_note() {
    let store = demo_store();
    let dir = tempfile::tempdir().unwrap();
    let path = write_file(&dir, "blank.txt", b"");
    let id = add_material(&store, "blank.txt");

    let outcome = index_file(&store, &id, &path, None).unwrap();

    assert_eq!(outcome, IndexOutcome::Empty);
    let stored = material(&store, &id);
    assert_eq!(stored.text_status, TextStatus::Ok);
    assert_eq!(stored.text_error.as_deref(), Some(NO_TEXT_NOTE));
    assert_eq!(stored.content_hash, Some(sha256_hex(b"")));
    assert_eq!(store.chunk_count(&id).unwrap(), 0);

    // Status ok + same hash: nothing to do next time.
    assert_eq!(
        index_file(&store, &id, &path, None).unwrap(),
        IndexOutcome::Unchanged
    );
}

#[test]
fn file_that_becomes_empty_loses_its_old_chunks() {
    let store = demo_store();
    let dir = tempfile::tempdir().unwrap();
    let path = write_file(&dir, "handout.txt", b"Demo handout about muword.");
    let id = add_material(&store, "handout.txt");
    index_file(&store, &id, &path, None).unwrap();
    assert_eq!(search_ids(&store, "muword"), vec![id.clone()]);

    // E.g. a text PDF replaced by a scanned copy: same material, no text any more.
    std::fs::write(&path, b"  \n\n  ").unwrap();
    let outcome = index_file(&store, &id, &path, None).unwrap();

    assert_eq!(outcome, IndexOutcome::Empty);
    assert_eq!(store.chunk_count(&id).unwrap(), 0);
    assert!(search_ids(&store, "muword").is_empty());
    let stored = material(&store, &id);
    assert_eq!(stored.text_status, TextStatus::Ok);
    assert_eq!(stored.text_error.as_deref(), Some(NO_TEXT_NOTE));
    assert_eq!(stored.content_hash, Some(sha256_file(&path).unwrap()));
}

#[test]
fn missing_file_is_an_io_error_and_leaves_the_state_alone() {
    let store = demo_store();
    let dir = tempfile::tempdir().unwrap();
    let path = write_file(&dir, "notes.txt", b"Demo notes about zetaword.");
    let id = add_material(&store, "notes.txt");
    index_file(&store, &id, &path, None).unwrap();
    let before = material(&store, &id);

    let missing = dir.path().join("gone.pdf");
    let result = index_file(&store, &id, &missing, None);

    assert!(matches!(result, Err(Error::Io(_))), "{result:?}");
    let after = material(&store, &id);
    assert_eq!(after.text_status, TextStatus::Ok);
    assert_eq!(after.content_hash, before.content_hash);
    assert_eq!(search_ids(&store, "zetaword"), vec![id.clone()]);
}

#[test]
fn missing_material_is_not_found() {
    let store = demo_store();
    let dir = tempfile::tempdir().unwrap();
    let path = write_file(&dir, "notes.txt", b"Demo notes.");
    let id = format!("{COURSE}/material/does-not-exist");

    let results = [
        index_file(&store, &id, &path, None),
        index_html(&store, &id, "<p>Demo page</p>"),
        index_text(&store, &id, "Demo text"),
    ];

    for result in results {
        assert!(matches!(result, Err(Error::NotFound(_))), "{result:?}");
    }
    assert_eq!(store.counts().unwrap().chunks, 0);
}

// ----- index_html / index_text -------------------------------------------------------------

#[test]
fn html_headings_become_locators() {
    let store = demo_store();
    let id = add_material(&store, "welcome-page");
    let html = "<p>Welcome to DEMO101 Intro to Demo Studies.</p>\
        <h1>Week 1</h1><p>We start with the thetaword method.</p>\
        <h2>Readings</h2><ul><li>Demo chapter one</li></ul>\
        <script>ignored()</script>";

    let outcome = index_html(&store, &id, html).unwrap();

    assert_eq!(outcome, IndexOutcome::Indexed { chunks: 3 });
    let chunks = store.get_chunks(&id, 0, None).unwrap();
    assert_eq!(ords(&chunks), vec![0, 1, 2]);
    assert_eq!(
        locators(&chunks),
        vec![None, Some("§ Week 1"), Some("§ Readings")]
    );
    let hits = store.search("thetaword", None, 10).unwrap();
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].locator.as_deref(), Some("§ Week 1"));
    assert!(search_ids(&store, "ignored").is_empty());

    let stored = material(&store, &id);
    assert_eq!(stored.text_status, TextStatus::Ok);
    assert_eq!(stored.content_hash, Some(sha256_hex(html.as_bytes())));
    assert_eq!(
        index_html(&store, &id, html).unwrap(),
        IndexOutcome::Unchanged
    );
}

#[test]
fn plain_text_is_one_unlocated_segment() {
    let store = demo_store();
    let id = add_material(&store, "announcement");

    let outcome = index_text(&store, &id, "Demo announcement about iotaword.").unwrap();

    assert_eq!(outcome, IndexOutcome::Indexed { chunks: 1 });
    let chunks = store.get_chunks(&id, 0, None).unwrap();
    assert_eq!(locators(&chunks), vec![None]);
    assert_eq!(chunks[0].text, "Demo announcement about iotaword.");
    assert_eq!(search_ids(&store, "iotaword"), vec![id.clone()]);
    assert_eq!(
        index_text(&store, &id, "Demo announcement about iotaword.").unwrap(),
        IndexOutcome::Unchanged
    );
}

#[test]
fn long_text_becomes_several_ordered_chunks() {
    let store = demo_store();
    let id = add_material(&store, "long-reading");
    let sentence = "Demo studies examine the kappaword pattern in detail. ";
    let text = sentence.repeat(200); // about 10 800 characters

    let outcome = index_text(&store, &id, &text).unwrap();

    let IndexOutcome::Indexed { chunks: count } = outcome else {
        panic!("expected Indexed, got {outcome:?}");
    };
    assert!(count > 1, "{count}");
    let chunks = store.get_chunks(&id, 0, None).unwrap();
    assert_eq!(ords(&chunks), (0..count).collect::<Vec<u32>>());
    assert!(chunks.iter().all(|chunk| chunk.locator.is_none()));
}

#[test]
fn huge_text_is_capped_at_5_mb_like_extracted_files() {
    let store = demo_store();
    let id = add_material(&store, "huge-reading");
    // About 6 MB: "nuword" at the start, "xiword" only after the 5 MB cap.
    let mut text = String::from("Opening remarks about nuword.\n\n");
    let sentence = "Demo studies examine many patterns in careful detail. ";
    while text.len() < 6 * 1024 * 1024 {
        text.push_str(sentence);
    }
    text.push_str("Closing remarks about xiword.");

    let outcome = index_text(&store, &id, &text).unwrap();

    assert!(
        matches!(outcome, IndexOutcome::Indexed { .. }),
        "{outcome:?}"
    );
    assert_eq!(search_ids(&store, "nuword"), vec![id.clone()]);
    assert!(search_ids(&store, "xiword").is_empty());
    let chunks = store.get_chunks(&id, 0, None).unwrap();
    let last = chunks.last().unwrap();
    assert!(last.text.ends_with("was indexed.]"), "{}", last.text);
    // The hash still identifies the whole input, so the same text is not indexed twice.
    assert_eq!(
        material(&store, &id).content_hash,
        Some(sha256_hex(text.as_bytes()))
    );
    assert_eq!(
        index_text(&store, &id, &text).unwrap(),
        IndexOutcome::Unchanged
    );
}

#[test]
fn whitespace_only_text_is_empty() {
    let store = demo_store();
    let id = add_material(&store, "blank-page");

    assert_eq!(
        index_text(&store, &id, " \n\t\n ").unwrap(),
        IndexOutcome::Empty
    );
    let stored = material(&store, &id);
    assert_eq!(stored.text_status, TextStatus::Ok);
    assert_eq!(stored.text_error.as_deref(), Some(NO_TEXT_NOTE));
    assert_eq!(store.chunk_count(&id).unwrap(), 0);
}

#[test]
fn successful_reindex_clears_a_previous_error() {
    let store = demo_store();
    let dir = tempfile::tempdir().unwrap();
    let docx = write_file(&dir, "handout.docx", b"broken");
    let text = write_file(&dir, "handout.txt", b"Fixed handout about lambdaword.");
    let id = add_material(&store, "handout");
    assert!(matches!(
        index_file(&store, &id, &docx, None).unwrap(),
        IndexOutcome::Failed(_)
    ));

    assert_eq!(
        index_file(&store, &id, &text, None).unwrap(),
        IndexOutcome::Indexed { chunks: 1 }
    );
    let stored = material(&store, &id);
    assert_eq!(stored.text_status, TextStatus::Ok);
    assert_eq!(stored.text_error, None);
}

// ----- hashing ------------------------------------------------------------------------------

#[test]
fn sha256_hex_matches_known_vectors() {
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn sha256_file_matches_sha256_hex_across_buffer_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    // Larger than the 64 KiB read buffer and not a multiple of it.
    let bytes: Vec<u8> = (0..200_003u32).map(|i| (i % 251) as u8).collect();
    let path = write_file(&dir, "big.bin", &bytes);

    assert_eq!(sha256_file(&path).unwrap(), sha256_hex(&bytes));
    let empty = write_file(&dir, "empty.bin", b"");
    assert_eq!(sha256_file(&empty).unwrap(), sha256_hex(b""));
}

#[test]
fn sha256_file_reports_missing_files() {
    let dir = tempfile::tempdir().unwrap();
    let error = sha256_file(&dir.path().join("missing.pdf")).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
}

/// Plain text doesn't pass through the extractor: it is cleaned where it is saved, like every
/// other text (`pagelamp_core::scrub`), and so is a heading that became a locator.
#[test]
fn indexed_text_and_locators_keep_no_access_parameter() {
    let store = demo_store();
    let id = add_material(&store, "notes");
    index_text(
        &store,
        &id,
        "Slides: https://lms.example.edu/courses/1/files/5/download?verifier=Ab12Cd34Zz&wrap=1 (week 1)",
    )
    .unwrap();
    let chunks = store.get_chunks(&id, 0, None).unwrap();
    assert_eq!(
        chunks[0].text,
        "Slides: https://lms.example.edu/courses/1/files/5/download (week 1)"
    );

    let html = "<h2>Notes at https://lms.example.edu/files/8/preview?verifier=Ab12Cd34Zz</h2><p>thetaword</p>";
    index_html(&store, &id, html).unwrap();
    let hits = store.search("thetaword", None, 5).unwrap();
    assert_eq!(
        hits[0].locator.as_deref(),
        Some("§ Notes at https://lms.example.edu/files/8/preview")
    );
    assert!(store.search("Ab12Cd34Zz", None, 5).unwrap().is_empty());
}
