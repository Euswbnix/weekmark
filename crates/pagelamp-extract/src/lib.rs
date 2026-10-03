//! Text extraction for course materials, plus chunking for full-text search.
//!
//! Output is a list of `Segment`s, each carrying a human-readable `locator` used for
//! citations ("p. 3", "slide 5", "cell 4", "§ Heading"). Chunking splits long segments
//! while keeping their locator.
//!
//! Supported (by extension, case-insensitive; `mime_hint` may override):
//! - `.pdf`  → one segment per page, locator "p. N" (1-based)
//! - `.pptx` → one segment per slide (slide order from `ppt/presentation.xml` sldIdLst,
//!   falling back to numeric order of `ppt/slides/slideN.xml`), locator "slide N";
//!   speaker notes (`ppt/notesSlides`) appended to the slide's text as "Notes: …"
//! - `.docx` → paragraphs from `word/document.xml`, grouped under headings, locator "§ Heading"
//!   (or None before the first heading)
//! - `.ipynb`→ markdown + code cells, locator "cell N"
//! - `.md`, `.markdown`, `.txt`, `.tex`, `.py`, `.java`, `.c`, `.cpp`, `.h`, `.hs`, `.rkt`,
//!   `.r`, `.sql`, `.js`, `.ts` → text; markdown split by headings with locator "§ Heading"
//! - `.html`, `.htm` → via `extract_html`
//!
//! Anything else → `ExtractError::Unsupported`.
//!
//! Safety limits: refuse files > 200 MB; cap total extracted text at 5 MB per file (truncate
//! and note it); zip-based formats must guard against zip bombs (cap per-entry and total
//! decompressed size at 100 MB).
//!
//! On top of that (see `Limits`): `.pptx`/`.docx` may list at most 10,000 zip entries; a
//! PDF may list at most 5,000 pages, and PDF pages that would make `pdf-extract` recurse
//! endlessly (which aborts the process) are skipped. `lopdf` inflates PDF streams without
//! any size limit, so `pdf_inflate` decodes every stream with `FlateDecode` in its filter
//! chain with a cap (64 MB per stream, 256 MB together) and refuses the file above that, or
//! when another filter precedes a `FlateDecode`: once over the raw file before `lopdf`
//! loads it, and once over what `lopdf` parsed, where image streams are also emptied (text
//! extraction never needs their bytes). Known gaps are listed in `pdf_inflate`; closing them
//! needs extraction in a child process.
//!
//! Where things live (for maintainers):
//! - `format` — which extractor handles a file (MIME/extension rules);
//! - `pdf`, `pptx`, `docx`, `notebook`, `html`, `text` — one module per format;
//! - `pdf_check` — skips PDF pages that would make `pdf-extract` overflow its stack;
//! - `pdf_inflate` — refuses PDFs whose streams would inflate past `Limits`;
//! - `ooxml` — zip + XML plumbing shared by `.pptx` and `.docx` (zip-bomb limits live here);
//! - `chunk` — `chunk_segments`;
//! - `failure` — `failure_kind`, the kind of a `Failed` message;
//! - `util` — text decoding, whitespace normalisation, panic isolation.
//!
//! "MB" means 1024 × 1024 bytes. Text is never split inside a UTF-8 character.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use thiserror::Error;

mod chunk;
mod docx;
mod failure;
mod format;
mod html;
mod notebook;
mod ooxml;
mod pdf;
mod pdf_check;
mod pdf_inflate;
mod pptx;
#[cfg(test)]
mod test_support;
mod text;
mod util;
pub mod worker;

pub use failure::{FailureKind, failure_kind};
pub use util::panic_is_expected;

use format::{FileFormat, describe_file_type};
use util::{decode_text, failed, normalize_whitespace, read_capped, truncate_to_bytes};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub locator: Option<String>,
    pub text: String,
}

#[derive(Debug, Error)]
pub enum ExtractError {
    #[error("unsupported file type: {0}")]
    Unsupported(String),
    #[error("extraction failed: {0}")]
    Failed(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Extract text from a file on disk. Segments with only whitespace are dropped.
/// A PDF with no extractable text (scanned) returns `Ok(vec![])` — callers mark it so.
///
/// Segment text is whitespace-normalised (see `chunk_segments`). Errors: `Unsupported` for
/// file types we do not read (checked before touching the file), `Io` when the file cannot
/// be read, `Failed` for files that are too large or malformed (never a panic).
pub fn extract_file(path: &Path, mime_hint: Option<&str>) -> Result<Vec<Segment>, ExtractError> {
    extract_file_with_limits(path, mime_hint, &Limits::DEFAULT)
}

/// Convert LMS page HTML to plain text segments split at h1–h3 headings
/// (locator "§ Heading"). Scripts/styles removed; links rendered as "text (url)".
///
/// Text before the first heading has no locator; each section's text starts with its
/// heading. The 5 MB text cap applies here too.
pub fn extract_html(html: &str) -> Vec<Segment> {
    finish_segments(html::segments(html), Limits::DEFAULT.max_text_bytes)
}

/// Whether a local file is a document the operating system's default app may open (a shell's
/// "Open"): a format this crate reads that is not code, a script or HTML, whose MIME hint
/// (if any) agrees with its extension.
pub fn opens_as_document(path: &Path, mime_hint: Option<&str>) -> bool {
    format::opens_as_document(path, mime_hint)
}

/// Whether `extract_file` would attempt this file (by extension / mime).
pub fn is_supported(path: &Path, mime_hint: Option<&str>) -> bool {
    FileFormat::detect(path, mime_hint).is_some()
}

/// A chunk ready to store: locator + text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkText {
    pub locator: Option<String>,
    pub text: String,
}

/// Split segments into chunks of at most `max_chars` characters (not bytes), preferring
/// paragraph, then sentence, then whitespace boundaries; never split inside a UTF-8 char.
/// Short consecutive segments are NOT merged (each keeps its own locator). Consecutive
/// chunks from one long segment overlap by ~`max_chars / 10` characters.
/// Whitespace is normalised (runs of blank lines collapsed, trailing spaces trimmed).
///
/// Chunks are trimmed and never empty. A `max_chars` of 0 is treated as 1.
pub fn chunk_segments(segments: &[Segment], max_chars: usize) -> Vec<ChunkText> {
    chunk::chunk_segments(segments, max_chars)
}

/// Default chunk size used by sync.
pub const DEFAULT_CHUNK_CHARS: usize = 1800;

const MB: u64 = 1024 * 1024;

/// Safety limits for one file. The public functions always use [`Limits::DEFAULT`]; the
/// values are parameters only so tests can use tiny limits instead of 100 MB fixtures.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    /// Files larger than this are refused before reading them.
    pub(crate) max_file_bytes: u64,
    /// Extracted text kept per file (UTF-8 bytes); the rest is dropped with a note.
    pub(crate) max_text_bytes: usize,
    /// Zip-based formats: most entries an archive may list.
    pub(crate) max_zip_entries: usize,
    /// Zip-based formats: most bytes one part may decompress to.
    pub(crate) max_zip_entry_bytes: u64,
    /// Zip-based formats: most bytes all parts we read may decompress to together.
    pub(crate) max_zip_total_bytes: u64,
    /// PDF: most page entries the page tree may list. `pdf-extract`'s work grows with the
    /// square of this (see `pdf.rs`): measured in a release build, 1,000 pages take about
    /// 0.2 s, 5,000 pages 4 s, 10,000 pages 18 s. The 5 MB text cap is usually reached long
    /// before page 5,000 anyway.
    pub(crate) max_pdf_pages: usize,
    /// PDF: most bytes one `FlateDecode` stream may inflate to (`pdf_inflate`).
    pub(crate) max_pdf_stream_bytes: u64,
    /// PDF: most bytes all `FlateDecode` streams may inflate to together (image streams
    /// are emptied instead, see `pdf_inflate`).
    pub(crate) max_pdf_inflated_bytes: u64,
}

impl Limits {
    pub(crate) const DEFAULT: Limits = Limits {
        max_file_bytes: 200 * MB,
        max_text_bytes: 5 * MB as usize,
        max_zip_entries: 10_000,
        max_zip_entry_bytes: 100 * MB,
        max_zip_total_bytes: 100 * MB,
        max_pdf_pages: 5_000,
        max_pdf_stream_bytes: 64 * MB,
        max_pdf_inflated_bytes: 256 * MB,
    };
}

fn extract_file_with_limits(
    path: &Path,
    mime_hint: Option<&str>,
    limits: &Limits,
) -> Result<Vec<Segment>, ExtractError> {
    let Some(format) = FileFormat::detect(path, mime_hint) else {
        return Err(ExtractError::Unsupported(describe_file_type(
            path, mime_hint,
        )));
    };
    let size = std::fs::metadata(path)?.len();
    if size > limits.max_file_bytes {
        return Err(too_large(size, limits.max_file_bytes));
    }

    let segments = match format {
        FileFormat::Pdf => {
            let bytes = read_file(path, limits)?;
            let caps = pdf_inflate::Caps {
                per_stream: limits.max_pdf_stream_bytes,
                total: limits.max_pdf_inflated_bytes,
            };
            pdf_inflate::check(&bytes, caps)?;
            pdf::extract(&bytes, limits.max_pdf_pages, caps)?
        }
        FileFormat::Pptx => pptx::extract(open_package(path, limits)?)?,
        FileFormat::Docx => docx::extract(open_package(path, limits)?)?,
        FileFormat::Notebook => notebook::extract(&read_text_file(path, limits)?)?,
        FileFormat::Html => html::segments(&read_text_file(path, limits)?),
        FileFormat::Markdown => text::markdown_segments(&read_text_file(path, limits)?),
        FileFormat::PlainText => text::plain_segments(&read_file(path, limits)?)?,
    };
    Ok(finish_segments(segments, limits.max_text_bytes))
}

fn too_large(size: u64, limit: u64) -> ExtractError {
    failed(format!(
        "file is too large to index ({}; the limit is {})",
        util::size_text(size),
        util::size_text(limit)
    ))
}

/// Read the whole file, enforcing the size limit while reading (the file could have grown
/// since its size was checked).
fn read_file(path: &Path, limits: &Limits) -> Result<Vec<u8>, ExtractError> {
    read_capped(File::open(path)?, limits.max_file_bytes)?
        .ok_or_else(|| too_large(limits.max_file_bytes + 1, limits.max_file_bytes))
}

fn read_text_file(path: &Path, limits: &Limits) -> Result<String, ExtractError> {
    Ok(decode_text(&read_file(path, limits)?))
}

fn open_package(
    path: &Path,
    limits: &Limits,
) -> Result<ooxml::Package<BufReader<File>>, ExtractError> {
    ooxml::Package::open(BufReader::new(File::open(path)?), limits)
}

/// Final step for every format: normalise whitespace, drop empty segments, and keep at most
/// `max_text_bytes` of text in total. When text is cut off, a last segment (no locator)
/// says so, so readers know the material continues.
fn finish_segments(segments: Vec<Segment>, max_text_bytes: usize) -> Vec<Segment> {
    let mut finished = Vec::new();
    let mut total_bytes = 0;
    for segment in segments {
        let text = normalize_whitespace(&segment.text);
        if text.is_empty() {
            continue;
        }
        let room = max_text_bytes - total_bytes;
        if text.len() > room {
            let kept = truncate_to_bytes(&text, room).trim_end();
            if !kept.is_empty() {
                finished.push(Segment {
                    locator: segment.locator,
                    text: kept.to_string(),
                });
            }
            finished.push(truncation_note(max_text_bytes));
            return finished;
        }
        total_bytes += text.len();
        finished.push(Segment {
            locator: segment.locator,
            text,
        });
    }
    finished
}

fn truncation_note(max_text_bytes: usize) -> Segment {
    Segment {
        locator: None,
        text: format!(
            "[Text truncated: only the first {:.1} MB of this file's text was extracted.]",
            max_text_bytes as f64 / MB as f64
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        document_xml, kind_of, notes_xml, para, pdf_bytes, pdf_bytes_compressed, presentation_xml,
        rels_xml, slide_xml, styles_xml, write_file, zip_text,
    };

    fn locators(segments: &[Segment]) -> Vec<Option<&str>> {
        segments.iter().map(|s| s.locator.as_deref()).collect()
    }

    #[test]
    fn extracts_pdf_pages_and_drops_blank_ones() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_file(
            &dir,
            "lecture.pdf",
            &pdf_bytes(&[Some("Alpha page one"), None, Some("Gamma page three")]),
        );
        let segments = extract_file(&path, None).unwrap();
        assert_eq!(
            segments,
            vec![
                Segment {
                    locator: Some("p. 1".into()),
                    text: "Alpha page one".into()
                },
                Segment {
                    locator: Some("p. 3".into()),
                    text: "Gamma page three".into()
                },
            ]
        );
    }

    #[test]
    fn scanned_pdf_without_text_is_empty_ok() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_file(&dir, "scan.pdf", &pdf_bytes(&[None, None]));
        assert_eq!(extract_file(&path, None).unwrap(), vec![]);
    }

    #[test]
    fn malformed_files_are_failed_not_panics() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["bad.pdf", "bad.pptx", "bad.docx", "bad.ipynb"] {
            let path = write_file(&dir, name, b"%PDF-1.4 this is not a real file {");
            let result = extract_file(&path, None);
            assert!(
                matches!(result, Err(ExtractError::Failed(_))),
                "{name}: {result:?}"
            );
            assert_eq!(kind_of(&result), Some(FailureKind::Malformed), "{name}");
        }
    }

    #[test]
    fn extracts_pptx_through_public_api() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = zip_text(&[
            ("ppt/presentation.xml", &presentation_xml(&["rId2", "rId1"])),
            (
                "ppt/_rels/presentation.xml.rels",
                &rels_xml(&[
                    ("rId1", "slide", "slides/slide1.xml"),
                    ("rId2", "slide", "slides/slide2.xml"),
                ]),
            ),
            ("ppt/slides/slide1.xml", &slide_xml(&[&["Second shown"]])),
            ("ppt/slides/slide2.xml", &slide_xml(&[&["First shown"]])),
            (
                "ppt/slides/_rels/slide1.xml.rels",
                &rels_xml(&[("rId3", "notesSlide", "../notesSlides/notesSlide1.xml")]),
            ),
            (
                "ppt/notesSlides/notesSlide1.xml",
                &notes_xml("Speaker note."),
            ),
        ]);
        let path = write_file(&dir, "deck.PPTX", &bytes);
        let segments = extract_file(&path, None).unwrap();
        assert_eq!(
            segments,
            vec![
                Segment {
                    locator: Some("slide 1".into()),
                    text: "First shown".into()
                },
                Segment {
                    locator: Some("slide 2".into()),
                    text: "Second shown\n\nNotes: Speaker note.".into()
                },
            ]
        );
    }

    #[test]
    fn extracts_docx_through_public_api() {
        let dir = tempfile::tempdir().unwrap();
        let body = [
            para(None, "Syllabus draft"),
            para(Some("Heading1"), "Assessment"),
            para(None, "Two quizzes."),
        ]
        .concat();
        let bytes = zip_text(&[
            ("word/document.xml", &document_xml(&body)),
            ("word/styles.xml", &styles_xml(&[("Heading1", "heading 1")])),
        ]);
        let path = write_file(&dir, "syllabus.docx", &bytes);
        let segments = extract_file(&path, None).unwrap();
        assert_eq!(locators(&segments), vec![None, Some("§ Assessment")]);
        assert_eq!(segments[1].text, "Assessment\n\nTwo quizzes.");
    }

    #[test]
    fn extracts_notebook_markdown_html_and_text_files() {
        let dir = tempfile::tempdir().unwrap();
        let notebook = write_file(
            &dir,
            "lab.ipynb",
            br#"{"cells": [{"cell_type": "markdown", "source": "Lab intro"}]}"#,
        );
        let markdown = write_file(
            &dir,
            "notes.md",
            "\u{FEFF}Intro\n\n# Week 1\nBody\n".as_bytes(),
        );
        let html = write_file(&dir, "page.html", b"<h2>Week 2</h2><p>Page body</p>");
        let code = write_file(&dir, "demo.py", b"def f():\r\n    return 1   \r\n");

        let segments = extract_file(&notebook, None).unwrap();
        assert_eq!(locators(&segments), vec![Some("cell 1")]);

        let segments = extract_file(&markdown, None).unwrap();
        assert_eq!(locators(&segments), vec![None, Some("§ Week 1")]);
        assert_eq!(segments[0].text, "Intro", "BOM must be stripped");

        let segments = extract_file(&html, None).unwrap();
        assert_eq!(locators(&segments), vec![Some("§ Week 2")]);
        assert_eq!(segments[0].text, "Week 2\n\nPage body");

        let segments = extract_file(&code, None).unwrap();
        assert_eq!(
            segments,
            vec![Segment {
                locator: None,
                text: "def f():\n    return 1".into()
            }]
        );
    }

    #[test]
    fn mime_hint_overrides_extension() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_file(&dir, "download", b"<h1>Title</h1><p>x</p>");
        assert!(matches!(
            extract_file(&path, None),
            Err(ExtractError::Unsupported(_))
        ));
        let segments = extract_file(&path, Some("text/html; charset=utf-8")).unwrap();
        assert_eq!(locators(&segments), vec![Some("§ Title")]);
    }

    #[test]
    fn unsupported_types_are_reported_before_reading() {
        // The file does not even exist: Unsupported must win over Io.
        for (name, mime) in [
            ("old.ppt", None),
            ("old.doc", None),
            ("photo.png", Some("image/png")),
            ("clip.mp4", Some("video/mp4")),
        ] {
            let path = Path::new("/nonexistent/dir").join(name);
            assert!(!is_supported(&path, mime));
            assert!(matches!(
                extract_file(&path, mime),
                Err(ExtractError::Unsupported(_))
            ));
        }
    }

    #[test]
    fn is_supported_agrees_with_extract_file() {
        let dir = tempfile::tempdir().unwrap();
        let cases = [
            ("a.txt", None),
            ("a.bin", Some("text/plain")),
            ("a.docx", Some("application/octet-stream")),
            ("a.ts", Some("video/mp2t")),
            ("a.xls", None),
            ("a.md", Some("text/plain")),
            ("a", Some("application/x-ipynb+json")),
        ];
        for (name, mime) in cases {
            let path = write_file(&dir, name, b"{}");
            let unsupported =
                matches!(extract_file(&path, mime), Err(ExtractError::Unsupported(_)));
            assert_eq!(is_supported(&path, mime), !unsupported, "{name} {mime:?}");
        }
    }

    #[test]
    fn typescript_with_video_mime_hint_is_read_but_real_video_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let code = write_file(&dir, "app.ts", b"export const demo = 1;\n");
        for hint in ["video/mp2t", "video/vnd.dlna.mpeg-tts"] {
            assert!(is_supported(&code, Some(hint)), "{hint}");
            let segments = extract_file(&code, Some(hint)).unwrap();
            assert_eq!(segments[0].text, "export const demo = 1;", "{hint}");
        }
        // An MPEG transport stream: 188-byte packets starting with the 0x47 sync byte.
        let mut video = Vec::new();
        for _ in 0..20 {
            video.push(0x47);
            video.extend([0u8; 187]);
        }
        let video = write_file(&dir, "movie.ts", &video);
        let result = extract_file(&video, Some("video/mp2t"));
        assert!(
            matches!(&result, Err(ExtractError::Failed(m)) if m.contains("binary")),
            "{result:?}"
        );
        assert_eq!(kind_of(&result), Some(FailureKind::Malformed));
    }

    #[test]
    fn notebook_with_plain_text_hint_is_still_read_as_a_notebook() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_file(
            &dir,
            "lab.ipynb",
            br#"{"cells": [{"cell_type": "markdown", "source": "Intro"},
                {"cell_type": "code", "source": "x = 1", "outputs": [{"text": "OUTPUT_TEXT"}]}]}"#,
        );
        let segments = extract_file(&path, Some("text/plain")).unwrap();
        assert_eq!(locators(&segments), vec![Some("cell 1"), Some("cell 2")]);
        assert!(!segments.iter().any(|s| s.text.contains("OUTPUT_TEXT")));
    }

    #[test]
    fn missing_file_is_io_error() {
        let result = extract_file(Path::new("/nonexistent/dir/notes.txt"), None);
        assert!(matches!(result, Err(ExtractError::Io(_))));
    }

    #[test]
    fn files_over_the_size_limit_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_file(&dir, "big.txt", &[b'a'; 2_000]);
        let limits = Limits {
            max_file_bytes: 1_000,
            ..Limits::DEFAULT
        };
        let result = extract_file_with_limits(&path, None, &limits);
        assert_eq!(kind_of(&result), Some(FailureKind::TooLarge));
        assert!(matches!(result, Err(ExtractError::Failed(m)) if m.contains("too large")));
    }

    #[test]
    fn pdf_whose_streams_inflate_too_much_is_refused_before_it_is_parsed() {
        let dir = tempfile::tempdir().unwrap();
        let long_line = "Alpha ".repeat(50_000);
        let bytes = pdf_bytes_compressed(&[Some(&long_line)]);
        assert!(bytes.len() < 100_000, "{} bytes", bytes.len());
        let path = write_file(&dir, "bomb.pdf", &bytes);
        let limits = Limits {
            max_pdf_stream_bytes: 100_000,
            ..Limits::DEFAULT
        };
        let result = extract_file_with_limits(&path, None, &limits);
        assert!(
            matches!(&result, Err(ExtractError::Failed(m)) if m.contains("would expand")),
            "{result:?}"
        );
        assert_eq!(kind_of(&result), Some(FailureKind::TooLarge));
        // Within the default limits the same file is read normally.
        let segments = extract_file_with_limits(&path, None, &Limits::DEFAULT).unwrap();
        assert!(segments[0].text.contains("Alpha Alpha"));
    }

    #[test]
    fn zip_bomb_like_docx_is_failed() {
        let dir = tempfile::tempdir().unwrap();
        let huge_paragraph = para(None, &"x".repeat(5_000));
        let bytes = zip_text(&[("word/document.xml", &document_xml(&huge_paragraph))]);
        let path = write_file(&dir, "bomb.docx", &bytes);
        let limits = Limits {
            max_zip_entry_bytes: 1_000,
            ..Limits::DEFAULT
        };
        let result = extract_file_with_limits(&path, None, &limits);
        assert!(matches!(result, Err(ExtractError::Failed(_))), "{result:?}");
        assert_eq!(kind_of(&result), Some(FailureKind::TooLarge));
    }

    #[test]
    fn text_over_the_cap_is_truncated_with_a_note() {
        let dir = tempfile::tempdir().unwrap();
        // Section One is ~606 bytes ("é" is 2 bytes), so section Two only partly fits.
        let markdown = format!(
            "# One\n{}\n# Two\n{}\n# Three\nlast\n",
            "é".repeat(300),
            "b".repeat(600)
        );
        let path = write_file(&dir, "long.md", markdown.as_bytes());
        let limits = Limits {
            max_text_bytes: 1_000,
            ..Limits::DEFAULT
        };
        let segments = extract_file_with_limits(&path, None, &limits).unwrap();
        assert_eq!(
            locators(&segments),
            vec![Some("§ One"), Some("§ Two"), None]
        );
        let kept: usize = segments[..2].iter().map(|s| s.text.len()).sum();
        assert!(kept <= 1_000);
        assert!(segments[2].text.contains("truncated"));
        assert!(!segments.iter().any(|s| s.text.contains("last")));
    }

    #[test]
    fn finish_segments_keeps_everything_under_the_cap() {
        let segments = vec![
            Segment {
                locator: Some("p. 1".into()),
                text: "  one  \n\n\n".into(),
            },
            Segment {
                locator: Some("p. 2".into()),
                text: " \n ".into(),
            },
        ];
        assert_eq!(
            finish_segments(segments, 100),
            vec![Segment {
                locator: Some("p. 1".into()),
                text: "  one".into()
            }]
        );
    }

    #[test]
    fn extract_html_public_function() {
        let segments = extract_html(
            "<script>x()</script><p>Hi <a href=\"https://example.edu/demo\">demo</a></p><h3>Next</h3><p>More</p>",
        );
        assert_eq!(
            segments,
            vec![
                Segment {
                    locator: None,
                    text: "Hi demo (https://example.edu/demo)".into()
                },
                Segment {
                    locator: Some("§ Next".into()),
                    text: "Next\n\nMore".into()
                },
            ]
        );
    }

    #[test]
    fn chunking_extracted_segments_keeps_locators() {
        let segments = extract_html(&format!(
            "<h1>Long</h1><p>{}</p>",
            "Demo sentence number one. ".repeat(200)
        ));
        let chunks = chunk_segments(&segments, 500);
        assert!(chunks.len() > 5);
        assert!(
            chunks
                .iter()
                .all(|c| c.locator.as_deref() == Some("§ Long"))
        );
        assert!(chunks.iter().all(|c| c.text.chars().count() <= 500));
    }
}
