//! Sync-time indexing shared by all sources: hash → (skip if unchanged) → extract → chunk →
//! store chunks → update text state. Heavy work happens here, never in MCP tool calls.
//!
//! Every `index_*` function follows the same steps:
//! 1. Read the material row (`Error::NotFound` if it does not exist).
//! 2. Hash the content (SHA-256). If the hash equals the stored `content_hash` and
//!    `text_status` is already ok, stop: `IndexOutcome::Unchanged`, nothing is written.
//! 3. Extract and chunk the text. No transaction is open during this step: extracting a big
//!    PDF can take seconds, and holding SQLite's write lock that long would make other writers
//!    (e.g. an MCP server saving a study plan) wait.
//! 4. Write the new text state, the new hash and the chunks in ONE short transaction, so
//!    readers see either the old or the new index of a material, never a mix.
//!
//! Because of step 4, these functions must not be called inside `Store::in_transaction`
//! (SQLite transactions do not nest; the call would fail with a database error).
//!
//! Files are read by an `Extractor`: in `pagelamp extract-worker` processes (v0.3 M0.5) when
//! the surface set a worker, else in this process. A file the worker could not read because of
//! the file itself (`TextErrorKind::is_hard`) is recorded with `failure_fingerprint` and not
//! tried again while its content, the worker protocol and the app version stay the same.

use std::borrow::Cow;
use std::fs::File;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use pagelamp_extract::worker::{self, WorkerFailure, WorkerLimits};
use pagelamp_extract::{DEFAULT_CHUNK_CHARS, ExtractError, Segment, chunk_segments};
use sha2::{Digest, Sha256};

use crate::brand;
use crate::model::{Chunk, Material, TextErrorKind, TextStatus};
use crate::source::CancelFlag;
use crate::store::Store;
use crate::{Error, Result};

/// `text_error` stored for `IndexOutcome::Empty` (a supported file without any text).
pub const NO_TEXT_NOTE: &str = "no extractable text (scanned?)";

/// `sha256_file` reads the file in pieces of this size, so memory use does not depend on
/// the file size.
const HASH_BUFFER_BYTES: usize = 64 * 1024;

/// Most text `index_text` indexes per material (UTF-8 bytes). Same value as the per-file cap
/// inside `pagelamp_extract` (which is not public), so every material stays below it.
const MAX_TEXT_BYTES: usize = 5 * 1024 * 1024;

/// Appended by `index_text` when it cut the text off at `MAX_TEXT_BYTES`.
const TRUNCATED_NOTE: &str = "[Text truncated: only the first 5.0 MB of this text was indexed.]";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IndexOutcome {
    /// Text extracted; this many chunks stored. text_status = ok.
    Indexed { chunks: u32 },
    /// Content hash equal to the stored one and text_status already ok — nothing done.
    Unchanged,
    /// Not a supported type. text_status = unsupported, chunks cleared.
    Unsupported,
    /// Supported but no text (e.g. scanned PDF). text_status = ok with 0 chunks,
    /// text_error = "no extractable text (scanned?)".
    Empty,
    /// Extraction error. text_status = error, text_error = message, chunks cleared; for a
    /// worker failure also `text_error_kind` and its fingerprint.
    Failed(String),
    /// The worker already failed on this content (a hard `TextErrorKind`) with the same worker
    /// protocol and app version — nothing done.
    Skipped(TextErrorKind),
    /// The worker could not run (`spawn_failed` / `protocol_mismatch`) and the file may not be
    /// read in this process: recorded like `Failed` and tried again on the next sync. Not worth
    /// a warning per file: `Extractor::take_warning` gives one for the whole sync.
    Deferred(TextErrorKind),
}

/// The worker protocol and app version a worker failure is recorded with
/// (`materials.text_error_fingerprint`), e.g. "worker1/0.3.0".
pub fn failure_fingerprint() -> String {
    format!("worker{}/{}", worker::PROTOCOL, env!("CARGO_PKG_VERSION"))
}

/// Whether a stored worker failure should be tried again although the content is unchanged:
/// the worker could not run at all (not the file's fault), or the failure was recorded by
/// another worker protocol or app version. (Callers that re-read files only when they change,
/// like the Canvas sync with its cached copies, use this to re-index them.)
pub fn needs_retry(material: &Material) -> bool {
    material.text_status == TextStatus::Error
        && material.text_error_kind.is_some_and(|kind| {
            !kind.is_hard()
                || material.text_error_fingerprint.as_deref() != Some(&failure_fingerprint())
        })
}

/// A user-facing sentence for a worker failure (stored as `text_error`, shown as a warning).
pub fn worker_failure_message(kind: TextErrorKind) -> &'static str {
    match kind {
        TextErrorKind::TimedOut => {
            "reading this file took too long, so it was stopped; it is tried again when the \
             file changes or the app is updated"
        }
        TextErrorKind::CpuLimit => {
            "reading this file needed too much processing time, so it was stopped; it is tried \
             again when the file changes or the app is updated"
        }
        TextErrorKind::MemoryLimit => {
            "reading this file needed too much memory, so it was stopped; it is tried again \
             when the file changes or the app is updated"
        }
        TextErrorKind::Crashed => {
            "the text reader stopped unexpectedly on this file; it is tried again when the file \
             changes or the app is updated"
        }
        TextErrorKind::BadOutput => {
            "the text reader gave an unreadable answer for this file; it is tried again on the \
             next sync"
        }
        TextErrorKind::SpawnFailed => {
            "the text reader could not start; this file is tried again on the next sync"
        }
        TextErrorKind::ProtocolMismatch => {
            "the text reader belongs to another version; this file is tried again on the next \
             sync"
        }
    }
}

/// How files are turned into text: in `pagelamp extract-worker` processes, or in this process.
///
/// Create one per sync and share it (clones share its state): after the first
/// `spawn_failed` / `protocol_mismatch` it stops starting workers for the rest of the sync
/// (each attempt can be slow when antivirus is involved) and applies the fallback rule to every
/// later file: small non-PDF files are read in-process
/// (`worker::in_process_fallback_allowed`), everything else is marked and retried on the next
/// sync. `take_warning` then returns one warning for the sync.
#[derive(Clone, Debug, Default)]
pub struct Extractor {
    inner: Arc<ExtractorInner>,
}

#[derive(Debug, Default)]
struct ExtractorInner {
    /// `None`: in this process.
    worker: Option<(PathBuf, WorkerLimits)>,
    /// Why the worker could not run in this sync, once it failed to.
    unavailable: Mutex<Option<WorkerFailure>>,
    /// Whether `take_warning` already returned the warning.
    warned: AtomicBool,
    /// The sync's stop request, if it can be stopped.
    cancel: CancelFlag,
}

impl Extractor {
    /// Extract in this process (tests, and surfaces that did not set a worker).
    pub fn in_process() -> Extractor {
        Extractor::default()
    }

    /// Extract in `<exe> extract-worker` processes with the default limits.
    pub fn worker(exe: PathBuf) -> Extractor {
        Extractor::worker_with_limits(exe, WorkerLimits::default())
    }

    pub fn worker_with_limits(exe: PathBuf, limits: WorkerLimits) -> Extractor {
        Extractor {
            inner: Arc::new(ExtractorInner {
                worker: Some((exe, limits)),
                ..ExtractorInner::default()
            }),
        }
    }

    /// The same extractor, stopped when `cancel` is set: a file being read in a worker is
    /// abandoned at once (`Error::Cancelled`); nothing is recorded for it.
    pub fn cancellable(self, cancel: CancelFlag) -> Extractor {
        Extractor {
            inner: Arc::new(ExtractorInner {
                worker: self.inner.worker.clone(),
                cancel,
                ..ExtractorInner::default()
            }),
        }
    }

    /// Whether the sync this extractor belongs to was stopped.
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancel.is_cancelled()
    }

    /// The worker executable, when files are extracted in worker processes.
    pub fn worker_exe(&self) -> Option<&Path> {
        self.inner.worker.as_ref().map(|(exe, _)| exe.as_path())
    }

    /// Once per extractor: a warning that the worker could not run in this sync, if so.
    pub fn take_warning(&self) -> Option<String> {
        let failure = (*self.unavailable())?;
        if self.inner.warned.swap(true, Ordering::Relaxed) {
            return None;
        }
        let product = brand::PRODUCT_NAME;
        let why = match failure {
            WorkerFailure::ProtocolMismatch => format!(
                "{product}'s text reader belongs to another version (reinstalling {product} \
                 fixes this)"
            ),
            _ => format!(
                "{product}'s text reader could not start (security software may be blocking it)"
            ),
        };
        Some(format!(
            "{why}. Small files other than PDFs were read directly; the rest are tried again on \
             the next sync."
        ))
    }

    fn unavailable(&self) -> std::sync::MutexGuard<'_, Option<WorkerFailure>> {
        self.inner
            .unavailable
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Extract `path` (of `size` bytes).
    fn extract(&self, path: &Path, mime: Option<&str>, size: u64) -> Extracted {
        if self.is_cancelled() {
            return Extracted::Cancelled;
        }
        let Some((exe, limits)) = &self.inner.worker else {
            return Extracted::Done(pagelamp_extract::extract_file(path, mime));
        };
        let known = *self.unavailable();
        let failure = match known {
            Some(failure) => failure,
            None => {
                let cancel = self.inner.cancel.as_atomic();
                match worker::extract_in_worker_cancellable(exe, path, mime, *limits, cancel) {
                    Ok(result) => return Extracted::Done(result),
                    Err(WorkerFailure::Cancelled) => return Extracted::Cancelled,
                    Err(
                        failure @ (WorkerFailure::SpawnFailed | WorkerFailure::ProtocolMismatch),
                    ) => {
                        *self.unavailable() = Some(failure);
                        failure
                    }
                    Err(failure) => return worker_failure(failure),
                }
            }
        };
        if worker::in_process_fallback_allowed(path, mime, size) {
            Extracted::Done(pagelamp_extract::extract_file(path, mime))
        } else {
            worker_failure(failure)
        }
    }
}

/// A worker failure as what extraction gave (a stop request isn't a failure of the file).
fn worker_failure(failure: WorkerFailure) -> Extracted {
    let kind = match failure {
        WorkerFailure::TimedOut => TextErrorKind::TimedOut,
        WorkerFailure::CpuLimit => TextErrorKind::CpuLimit,
        WorkerFailure::MemoryLimit => TextErrorKind::MemoryLimit,
        WorkerFailure::Crashed => TextErrorKind::Crashed,
        WorkerFailure::BadOutput => TextErrorKind::BadOutput,
        WorkerFailure::SpawnFailed => TextErrorKind::SpawnFailed,
        WorkerFailure::ProtocolMismatch => TextErrorKind::ProtocolMismatch,
        WorkerFailure::Cancelled => return Extracted::Cancelled,
    };
    Extracted::Worker(kind)
}

/// What extracting one file gave.
#[derive(Debug)]
enum Extracted {
    /// The extractor's own answer (from the worker or this process).
    Done(std::result::Result<Vec<Segment>, ExtractError>),
    /// The worker failed, and the file was not read in this process.
    Worker(TextErrorKind),
    /// The sync was stopped: nothing is recorded.
    Cancelled,
}

/// Index a file at `path` for `material_id` (material row must exist), extracting it in this
/// process. Syncs use `index_file_using` with the surface's `Extractor`.
pub fn index_file(
    store: &Store,
    material_id: &str,
    path: &Path,
    mime: Option<&str>,
) -> Result<IndexOutcome> {
    index_file_using(store, material_id, path, mime, &Extractor::in_process())
}

/// Index a file at `path` for `material_id` (material row must exist) with `extractor`.
/// Only `Err` for store/IO problems; extraction problems are reported via `IndexOutcome`.
///
/// `mime` is a hint for the extractor (see `pagelamp_extract::extract_file`). A missing or
/// unreadable file is an `Error::Io` and leaves the stored state untouched, so the next sync
/// simply tries again; the same goes for a file whose size or modification time changed
/// while it was being indexed. In every other case the file's hash is stored, whatever the
/// outcome.
pub fn index_file_using(
    store: &Store,
    material_id: &str,
    path: &Path,
    mime: Option<&str>,
    extractor: &Extractor,
) -> Result<IndexOutcome> {
    index_file_with(store, material_id, path, mime, |path, mime, size| {
        extractor.extract(path, mime, size)
    })
}

/// `index_file_using` with the extraction passed in (always `Extractor::extract`, except in
/// tests, which use this to change the file while it is being extracted).
fn index_file_with(
    store: &Store,
    material_id: &str,
    path: &Path,
    mime: Option<&str>,
    extract: impl FnOnce(&Path, Option<&str>, u64) -> Extracted,
) -> Result<IndexOutcome> {
    let material = require_material(store, material_id)?;
    let stamp_before = file_stamp(path)?;
    let hash = sha256_file(path)?;
    if is_indexed(&material, &hash) {
        return Ok(IndexOutcome::Unchanged);
    }
    if let Some(kind) = known_failure(&material, &hash) {
        return Ok(IndexOutcome::Skipped(kind));
    }
    // Slow step, deliberately outside any transaction (see the module docs).
    let extracted = extract(path, mime, stamp_before.0);
    // Hashing and extracting are two separate reads of the file. If it was rewritten in
    // between, the hash may not describe the stored text, and a matching hash would then
    // keep that wrong text forever ("unchanged"). So store nothing; the next sync retries.
    if file_stamp(path)? != stamp_before {
        return Err(Error::Io(std::io::Error::other(format!(
            "{} changed while it was being indexed; it will be indexed on the next sync",
            path.display()
        ))));
    }
    save_extraction(store, material_id, &hash, extracted)
}

/// Index LMS page / announcement / syllabus HTML.
///
/// The hash is taken over the HTML text itself; locators come from its h1–h3 headings
/// ("§ Heading", see `pagelamp_extract::extract_html`).
pub fn index_html(store: &Store, material_id: &str, html: &str) -> Result<IndexOutcome> {
    let material = require_material(store, material_id)?;
    let hash = sha256_hex(html.as_bytes());
    if is_indexed(&material, &hash) {
        return Ok(IndexOutcome::Unchanged);
    }
    let segments = pagelamp_extract::extract_html(html);
    save_extraction(store, material_id, &hash, Extracted::Done(Ok(segments)))
}

/// Index plain text (e.g. already-converted content).
///
/// The text is treated as one segment without a locator (long text still becomes several
/// chunks). Whitespace-only text gives `IndexOutcome::Empty`. Like extracted files, only the
/// first 5 MB are indexed, followed by a note that the text was cut off; the hash still
/// covers the whole text.
pub fn index_text(store: &Store, material_id: &str, text: &str) -> Result<IndexOutcome> {
    let material = require_material(store, material_id)?;
    let hash = sha256_hex(text.as_bytes());
    if is_indexed(&material, &hash) {
        return Ok(IndexOutcome::Unchanged);
    }
    let segments = vec![Segment {
        locator: None,
        text: capped_text(text, MAX_TEXT_BYTES),
    }];
    save_extraction(store, material_id, &hash, Extracted::Done(Ok(segments)))
}

/// Lower-case hex SHA-256.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Lower-case hex SHA-256 of a file's bytes (same format as `sha256_hex`).
///
/// The file is read in 64 KiB pieces, never loaded into memory as a whole.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; HASH_BUFFER_BYTES];
    loop {
        let read = match file.read(&mut buffer) {
            Ok(0) => break, // end of file
            Ok(read) => read,
            // A signal interrupted the read before any data arrived: just read again.
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

// ----- helpers ------------------------------------------------------------------------------

/// Size and last-modified time of a file: a cheap way to notice that it was rewritten.
/// (A same-size rewrite within the file system's timestamp resolution goes unnoticed; on
/// macOS/Linux/Windows file systems that resolution is far below a millisecond.)
fn file_stamp(path: &Path) -> std::io::Result<(u64, Option<SystemTime>)> {
    let metadata = std::fs::metadata(path)?;
    Ok((metadata.len(), metadata.modified().ok()))
}

/// `text` cut to at most `max_bytes` (never inside a UTF-8 character) plus `TRUNCATED_NOTE`,
/// or `text` unchanged when it already fits.
fn capped_text(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n\n{TRUNCATED_NOTE}", &text[..end])
}

/// The material row, or `Error::NotFound` (same message style as the store's errors).
fn require_material(store: &Store, material_id: &str) -> Result<Material> {
    store
        .get_material(material_id)?
        .ok_or_else(|| Error::NotFound(format!("material '{material_id}'")))
}

/// True when `material` already holds the successfully indexed text of content `hash`.
///
/// Any other stored status (pending, error, unsupported, …) is retried even with the same
/// hash: the extractor may have improved, or `upsert_material` reset the status to pending
/// because the file moved.
fn is_indexed(material: &Material, hash: &str) -> bool {
    material.text_status == TextStatus::Ok && material.content_hash.as_deref() == Some(hash)
}

/// The hard worker failure stored for content `hash`, unless it should be tried again
/// (`needs_retry`: another protocol or app version).
fn known_failure(material: &Material, hash: &str) -> Option<TextErrorKind> {
    let kind = material.text_error_kind?;
    (material.content_hash.as_deref() == Some(hash) && !needs_retry(material)).then_some(kind)
}

/// Turn an extraction result into the material's new text state (one row of the
/// `IndexOutcome` table) and store it together with the content `hash`.
///
/// `ExtractError::Io` means the file could not be read (e.g. deleted after it was hashed).
/// That says nothing about the file's content, so it is returned as `Error::Io` and nothing
/// is written.
fn save_extraction(
    store: &Store,
    material_id: &str,
    hash: &str,
    extracted: Extracted,
) -> Result<IndexOutcome> {
    let extracted = match extracted {
        Extracted::Cancelled => return Err(Error::Cancelled),
        Extracted::Done(result) => result,
        Extracted::Worker(kind) => {
            let message = worker_failure_message(kind);
            store.in_transaction(|store| {
                store.set_text_state(material_id, TextStatus::Error, Some(message), Some(hash))?;
                store.set_text_error_kind(material_id, kind, &failure_fingerprint())?;
                store.replace_chunks(material_id, &[])
            })?;
            return Ok(match kind {
                TextErrorKind::SpawnFailed | TextErrorKind::ProtocolMismatch => {
                    IndexOutcome::Deferred(kind)
                }
                _ => IndexOutcome::Failed(message.to_string()),
            });
        }
    };
    match extracted {
        Ok(mut segments) => {
            // Whatever produced the text (a file, the worker, plain text): no address in it,
            // or in a heading that became a locator, keeps a parameter that gives access to
            // a file.
            for segment in &mut segments {
                if let Cow::Owned(clean) = pagelamp_extract::scrub::scrub_text(&segment.text) {
                    segment.text = clean;
                }
                if let Some(locator) = &mut segment.locator
                    && let Cow::Owned(clean) = pagelamp_extract::scrub::scrub_text(locator)
                {
                    *locator = clean;
                }
            }
            let chunks = to_chunks(material_id, &segments);
            if chunks.is_empty() {
                write_text_state(
                    store,
                    material_id,
                    hash,
                    TextStatus::Ok,
                    Some(NO_TEXT_NOTE),
                    &[],
                )?;
                Ok(IndexOutcome::Empty)
            } else {
                write_text_state(store, material_id, hash, TextStatus::Ok, None, &chunks)?;
                // More than u32::MAX chunks would take terabytes of text; saturate, never panic.
                let count = u32::try_from(chunks.len()).unwrap_or(u32::MAX);
                Ok(IndexOutcome::Indexed { chunks: count })
            }
        }
        Err(ExtractError::Unsupported(_)) => {
            write_text_state(store, material_id, hash, TextStatus::Unsupported, None, &[])?;
            Ok(IndexOutcome::Unsupported)
        }
        Err(ExtractError::Failed(message)) => {
            write_text_state(
                store,
                material_id,
                hash,
                TextStatus::Error,
                Some(&message),
                &[],
            )?;
            Ok(IndexOutcome::Failed(message))
        }
        Err(ExtractError::Io(error)) => Err(Error::Io(error)),
    }
}

/// Chunk the segments for search. `ord` counts from 0; each chunk keeps its locator.
fn to_chunks(material_id: &str, segments: &[Segment]) -> Vec<Chunk> {
    // `(0..)` numbers the chunks as u32 directly, so no integer casts are needed.
    (0..)
        .zip(chunk_segments(segments, DEFAULT_CHUNK_CHARS))
        .map(|(ord, chunk)| Chunk {
            material_id: material_id.to_string(),
            ord,
            locator: chunk.locator,
            text: chunk.text,
        })
        .collect()
}

/// Store text status/error, the content hash and the chunks (replacing all old chunks) in one
/// short transaction. An empty `chunks` clears the material's chunks.
fn write_text_state(
    store: &Store,
    material_id: &str,
    hash: &str,
    status: TextStatus,
    text_error: Option<&str>,
    chunks: &[Chunk],
) -> Result<()> {
    store.in_transaction(|store| {
        // First, so a material deleted meanwhile fails with NotFound, not a foreign-key error.
        store.set_text_state(material_id, status, text_error, Some(hash))?;
        store.replace_chunks(material_id, chunks)
    })
}

#[cfg(test)]
mod tests {
    //! Paths that the public API reaches only through a race (a file changing or vanishing
    //! mid-sync, a material deleted mid-sync). All data is synthetic.

    use std::path::PathBuf;

    use super::*;
    use crate::model::{CourseUpsert, MaterialKind, MaterialUpsert, SourceKind, SourceRecord};

    const COURSE: &str = "folder:demo/course/DEMO101";
    const MATERIAL: &str = "folder:demo/course/DEMO101/material/notes.txt";

    /// In-memory store with one material ("DEMO101 Intro to Demo Studies" / notes.txt).
    fn demo_store() -> Store {
        let store = Store::open_in_memory().unwrap();
        store
            .upsert_source(&SourceRecord {
                id: "folder:demo".to_string(),
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
                source_id: "folder:demo".to_string(),
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
            .upsert_material(&MaterialUpsert {
                id: MATERIAL.to_string(),
                course_id: COURSE.to_string(),
                module_id: None,
                kind: MaterialKind::File,
                title: "notes.txt".to_string(),
                url: None,
                local_path: None,
                mime: None,
                published_at: None,
                week_hint: None,
            })
            .unwrap();
        store
    }

    fn write_notes(dir: &tempfile::TempDir, text: &str) -> PathBuf {
        let path = dir.path().join("notes.txt");
        std::fs::write(&path, text).unwrap();
        path
    }

    /// Text status, error, hash and chunk count of the demo material.
    fn state(store: &Store) -> (TextStatus, Option<String>, Option<String>, u32) {
        let material = store.get_material(MATERIAL).unwrap().unwrap();
        let chunks = store.chunk_count(MATERIAL).unwrap();
        (
            material.text_status,
            material.text_error,
            material.content_hash,
            chunks,
        )
    }

    fn one_segment(text: &str) -> Vec<Segment> {
        vec![Segment {
            locator: None,
            text: text.to_string(),
        }]
    }

    #[test]
    fn capped_text_cuts_at_a_char_boundary_and_adds_a_note() {
        assert_eq!(capped_text("héllo", 10), "héllo");
        assert_eq!(capped_text("héllo", 6), "héllo"); // exactly 6 bytes ("é" takes 2)
        assert_eq!(capped_text("héllo", 2), format!("h\n\n{TRUNCATED_NOTE}"));
        assert_eq!(capped_text("héllo", 3), format!("hé\n\n{TRUNCATED_NOTE}"));
    }

    #[test]
    fn extraction_io_error_writes_nothing() {
        let store = demo_store();
        let dir = tempfile::tempdir().unwrap();
        let path = write_notes(&dir, "Demo notes about omicronword.");
        index_file(&store, MATERIAL, &path, None).unwrap();
        let before = state(&store);

        let gone = ExtractError::Io(std::io::Error::other("gone"));
        let result = save_extraction(
            &store,
            MATERIAL,
            &sha256_hex(b"new"),
            Extracted::Done(Err(gone)),
        );

        assert!(matches!(result, Err(Error::Io(_))), "{result:?}");
        assert_eq!(state(&store), before);
    }

    #[test]
    fn material_deleted_during_extraction_is_not_found() {
        let store = demo_store();
        let missing = "folder:demo/course/DEMO101/material/deleted.txt";

        let result = save_extraction(
            &store,
            missing,
            "hash",
            Extracted::Done(Ok(one_segment("Demo text"))),
        );

        assert!(matches!(result, Err(Error::NotFound(_))), "{result:?}");
        assert_eq!(store.counts().unwrap().chunks, 0);
    }

    #[test]
    fn file_rewritten_during_extraction_writes_nothing() {
        let store = demo_store();
        let dir = tempfile::tempdir().unwrap();
        let path = write_notes(&dir, "First draft about piword.");
        index_file(&store, MATERIAL, &path, None).unwrap();
        let before = state(&store);
        std::fs::write(&path, "Second draft about rhoword.").unwrap();

        // An editor rewrites the file while it is being extracted: the extractor sees
        // other bytes than the ones that were hashed.
        let result = index_file_with(&store, MATERIAL, &path, None, |path, mime, _| {
            std::fs::write(path, "Third draft, longer, about sigmaword.").unwrap();
            Extracted::Done(pagelamp_extract::extract_file(path, mime))
        });

        assert!(matches!(result, Err(Error::Io(_))), "{result:?}");
        assert_eq!(state(&store), before);
        // The next sync indexes the file as it is now.
        assert_eq!(
            index_file(&store, MATERIAL, &path, None).unwrap(),
            IndexOutcome::Indexed { chunks: 1 }
        );
        assert_eq!(state(&store).2, Some(sha256_file(&path).unwrap()));
    }

    /// Text status, error kind and fingerprint of `id`.
    fn failure(store: &Store, id: &str) -> (TextStatus, Option<TextErrorKind>, Option<String>) {
        let material = store.get_material(id).unwrap().unwrap();
        (
            material.text_status,
            material.text_error_kind,
            material.text_error_fingerprint,
        )
    }

    /// A second material (a PDF) in the demo course.
    fn add_material(store: &Store, name: &str) -> String {
        let id = format!("folder:demo/course/DEMO101/material/{name}");
        store
            .upsert_material(&MaterialUpsert {
                id: id.clone(),
                course_id: COURSE.to_string(),
                module_id: None,
                kind: MaterialKind::File,
                title: name.to_string(),
                url: None,
                local_path: None,
                mime: None,
                published_at: None,
                week_hint: None,
            })
            .unwrap();
        id
    }

    fn not_again(_: &Path, _: Option<&str>, _: u64) -> Extracted {
        panic!("the file was extracted again")
    }

    #[test]
    fn a_hard_worker_failure_is_not_retried_until_something_changes() {
        let store = demo_store();
        let dir = tempfile::tempdir().unwrap();
        let path = write_notes(&dir, "Demo notes about upsilonword.");
        let timed_out =
            |_: &Path, _: Option<&str>, _: u64| Extracted::Worker(TextErrorKind::TimedOut);

        let outcome = index_file_with(&store, MATERIAL, &path, None, timed_out).unwrap();
        assert_eq!(
            outcome,
            IndexOutcome::Failed(worker_failure_message(TextErrorKind::TimedOut).to_string())
        );
        assert_eq!(
            failure(&store, MATERIAL),
            (
                TextStatus::Error,
                Some(TextErrorKind::TimedOut),
                Some(failure_fingerprint())
            )
        );
        assert_eq!(state(&store).2, Some(sha256_file(&path).unwrap()));

        // Same content, protocol and version: skipped without extracting.
        let again = index_file_with(&store, MATERIAL, &path, None, not_again).unwrap();
        assert_eq!(again, IndexOutcome::Skipped(TextErrorKind::TimedOut));
        assert!(!needs_retry(
            &store.get_material(MATERIAL).unwrap().unwrap()
        ));

        // Another app version or worker protocol recorded it: tried again.
        store
            .set_text_error_kind(MATERIAL, TextErrorKind::TimedOut, "worker0/0.0.1")
            .unwrap();
        assert!(needs_retry(&store.get_material(MATERIAL).unwrap().unwrap()));
        assert_eq!(
            index_file(&store, MATERIAL, &path, None).unwrap(),
            IndexOutcome::Indexed { chunks: 1 }
        );
        assert_eq!(failure(&store, MATERIAL), (TextStatus::Ok, None, None));

        // New content after a failure is extracted too.
        index_file_with(&store, MATERIAL, &path, None, timed_out).unwrap();
        std::fs::write(&path, "Revised demo notes about phiword.").unwrap();
        assert_eq!(
            index_file(&store, MATERIAL, &path, None).unwrap(),
            IndexOutcome::Indexed { chunks: 1 }
        );
    }

    #[test]
    fn worker_failures_that_are_not_the_files_fault_are_retried() {
        let store = demo_store();
        let dir = tempfile::tempdir().unwrap();
        let path = write_notes(&dir, "Demo notes about chiword.");
        for kind in [
            TextErrorKind::SpawnFailed,
            TextErrorKind::ProtocolMismatch,
            TextErrorKind::BadOutput,
        ] {
            let failed = index_file_with(&store, MATERIAL, &path, None, |_, _, _| {
                Extracted::Worker(kind)
            })
            .unwrap();
            let expected = match kind {
                TextErrorKind::BadOutput => {
                    IndexOutcome::Failed(worker_failure_message(kind).to_string())
                }
                _ => IndexOutcome::Deferred(kind),
            };
            assert_eq!(failed, expected);
            assert_eq!(failure(&store, MATERIAL).1, Some(kind));
            assert!(needs_retry(&store.get_material(MATERIAL).unwrap().unwrap()));
        }
        assert_eq!(
            index_file(&store, MATERIAL, &path, None).unwrap(),
            IndexOutcome::Indexed { chunks: 1 }
        );
        assert_eq!(failure(&store, MATERIAL), (TextStatus::Ok, None, None));
    }

    #[test]
    fn a_worker_that_cannot_start_falls_back_for_small_non_pdf_files_only() {
        let store = demo_store();
        let dir = tempfile::tempdir().unwrap();
        let notes = write_notes(&dir, "Demo notes about psiword.");
        let slides_id = add_material(&store, "slides.pdf");
        let slides = dir.path().join("slides.pdf");
        std::fs::write(&slides, b"%PDF-1.4 demo").unwrap();
        let extractor = Extractor::worker(dir.path().join("no-such-pagelamp"));
        assert!(extractor.take_warning().is_none());

        // The PDF first: the spawn fails, and a PDF is never parsed in-process.
        let outcome = index_file_using(&store, &slides_id, &slides, None, &extractor).unwrap();
        assert_eq!(outcome, IndexOutcome::Deferred(TextErrorKind::SpawnFailed));
        assert_eq!(
            failure(&store, &slides_id),
            (
                TextStatus::Error,
                Some(TextErrorKind::SpawnFailed),
                Some(failure_fingerprint())
            )
        );
        let warning = extractor.take_warning().expect("one warning");
        assert!(warning.contains("could not start"), "{warning}");

        // A small text file is read in this process; no second warning.
        assert_eq!(
            index_file_using(&store, MATERIAL, &notes, None, &extractor).unwrap(),
            IndexOutcome::Indexed { chunks: 1 }
        );
        assert!(extractor.take_warning().is_none());
        assert!(
            extractor.clone().take_warning().is_none(),
            "clones share it"
        );

        // The next sync tries the PDF again.
        let next = Extractor::worker(dir.path().join("no-such-pagelamp"));
        let outcome = index_file_using(&store, &slides_id, &slides, None, &next).unwrap();
        assert_eq!(outcome, IndexOutcome::Deferred(TextErrorKind::SpawnFailed));
        assert!(next.take_warning().is_some());
    }

    #[test]
    fn a_new_text_state_or_a_moved_file_clears_the_worker_failure() {
        let store = demo_store();
        store
            .set_text_error_kind(MATERIAL, TextErrorKind::CpuLimit, &failure_fingerprint())
            .unwrap();
        store
            .set_text_state(MATERIAL, TextStatus::Error, Some("x"), None)
            .unwrap();
        assert_eq!(failure(&store, MATERIAL), (TextStatus::Error, None, None));

        store
            .set_text_error_kind(MATERIAL, TextErrorKind::CpuLimit, &failure_fingerprint())
            .unwrap();
        let mut moved = MaterialUpsert {
            id: MATERIAL.to_string(),
            course_id: COURSE.to_string(),
            module_id: None,
            kind: MaterialKind::File,
            title: "notes.txt".to_string(),
            url: None,
            local_path: None,
            mime: None,
            published_at: None,
            week_hint: None,
        };
        store.upsert_material(&moved).unwrap();
        assert_eq!(failure(&store, MATERIAL).1, Some(TextErrorKind::CpuLimit));
        moved.local_path = Some("/demo/courses/DEMO101/notes.txt".to_string());
        store.upsert_material(&moved).unwrap();
        assert_eq!(failure(&store, MATERIAL), (TextStatus::Pending, None, None));
    }

    #[test]
    fn unreadable_files_are_counted_by_kind() {
        let store = demo_store();
        assert!(store.unreadable_file_counts().unwrap().is_empty());
        let slides = add_material(&store, "slides.pdf");
        let deck = add_material(&store, "deck.pdf");
        for (id, kind) in [
            (MATERIAL, TextErrorKind::MemoryLimit),
            (slides.as_str(), TextErrorKind::TimedOut),
            (deck.as_str(), TextErrorKind::MemoryLimit),
        ] {
            store
                .set_text_state(id, TextStatus::Error, Some("x"), Some("hash"))
                .unwrap();
            store
                .set_text_error_kind(id, kind, &failure_fingerprint())
                .unwrap();
        }
        assert_eq!(
            store.unreadable_file_counts().unwrap(),
            vec![
                (TextErrorKind::TimedOut, 1),
                (TextErrorKind::MemoryLimit, 2)
            ]
        );
    }

    #[test]
    fn same_size_rewrite_during_extraction_is_noticed_by_its_mtime() {
        let store = demo_store();
        let dir = tempfile::tempdir().unwrap();
        let path = write_notes(&dir, "Draft about tauword.");

        let result = index_file_with(&store, MATERIAL, &path, None, |path, mime, _| {
            // Same length, different time: like saving an undo of the same size.
            let file = std::fs::File::options().write(true).open(path).unwrap();
            let earlier = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1);
            file.set_modified(earlier).unwrap();
            Extracted::Done(pagelamp_extract::extract_file(path, mime))
        });

        assert!(matches!(result, Err(Error::Io(_))), "{result:?}");
        assert_eq!(state(&store).0, TextStatus::Pending);
        assert_eq!(store.chunk_count(MATERIAL).unwrap(), 0);
    }
}
