//! Decides which extractor handles a file, from its MIME hint and its extension.
//!
//! `extract_file` and `is_supported` both call [`FileFormat::detect`], so they always agree.
//!
//! Rules, in order:
//! 1. A MIME hint that maps to a supported format wins over the extension — except that a
//!    hint that only says "plain text" (`text/plain`, `text/x-python` …) never replaces a
//!    format known from the extension: `.md` stays Markdown, `.ipynb` stays a notebook, `.pdf`
//!    stays a PDF (servers and sniffers often label anything textual `text/plain`).
//! 2. Otherwise the extension (case-insensitive) decides. MIME types that map to no supported
//!    format are ignored, including `application/octet-stream` and media types: `mime_guess`
//!    calls every `.ts` file `video/vnd.dlna.mpeg-tts`, yet most `.ts` course files are
//!    TypeScript. (A real MPEG video named `.ts` is refused later as binary content, see
//!    `text::plain_segments`.)

use std::path::Path;

/// The formats `extract_file` can read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileFormat {
    Pdf,
    Pptx,
    Docx,
    Notebook,
    Html,
    Markdown,
    /// Plain text and source code: one segment, no locator.
    PlainText,
}

impl FileFormat {
    /// The format to use for `path`, or `None` when the file type is not supported.
    pub(crate) fn detect(path: &Path, mime_hint: Option<&str>) -> Option<FileFormat> {
        let from_extension = extension(path).and_then(|ext| FileFormat::from_extension(&ext));
        let from_mime = mime_hint.and_then(|mime| FileFormat::from_mime(&essence(mime)));
        match from_mime {
            Some(FileFormat::PlainText) if from_extension.is_some() => from_extension,
            Some(format) => Some(format),
            None => from_extension,
        }
    }

    fn from_extension(ext: &str) -> Option<FileFormat> {
        Some(match ext {
            "pdf" => FileFormat::Pdf,
            "pptx" => FileFormat::Pptx,
            "docx" => FileFormat::Docx,
            "ipynb" => FileFormat::Notebook,
            "html" | "htm" => FileFormat::Html,
            "md" | "markdown" => FileFormat::Markdown,
            "txt" | "tex" | "py" | "java" | "c" | "cpp" | "h" | "hs" | "rkt" | "r" | "sql"
            | "js" | "ts" => FileFormat::PlainText,
            _ => return None,
        })
    }

    /// `mime` must already be lower-case without parameters (see [`essence`]).
    fn from_mime(mime: &str) -> Option<FileFormat> {
        Some(match mime {
            "application/pdf" => FileFormat::Pdf,
            "application/vnd.openxmlformats-officedocument.presentationml.presentation" => {
                FileFormat::Pptx
            }
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
                FileFormat::Docx
            }
            "application/x-ipynb+json" => FileFormat::Notebook,
            "text/html" | "application/xhtml+xml" => FileFormat::Html,
            "text/markdown" | "text/x-markdown" => FileFormat::Markdown,
            "text/plain"
            | "text/x-python"
            | "text/x-script.python"
            | "text/x-java"
            | "text/x-java-source"
            | "text/x-c"
            | "text/x-csrc"
            | "text/x-chdr"
            | "text/x-c++src"
            | "text/x-c++hdr"
            | "text/x-haskell"
            | "text/x-racket"
            | "text/x-r"
            | "text/x-rsrc"
            | "text/x-sql"
            | "application/sql"
            | "text/javascript"
            | "application/javascript"
            | "application/x-javascript"
            | "text/x-typescript"
            | "application/typescript"
            | "text/x-tex"
            | "application/x-tex"
            | "application/x-latex" => FileFormat::PlainText,
            _ => return None,
        })
    }
}

/// Plain-text extensions that are documents rather than code (`from_extension`'s
/// `PlainText` line mixes both).
const TEXT_DOCUMENTS: [&str; 2] = ["txt", "tex"];

/// Whether `path` is a document this crate reads that the operating system's default app may
/// open: PDF, Office, notebook, Markdown, plain text or TeX, by extension, and not labelled
/// as another format by `mime_hint`. Never code or scripts (`.py`, `.js` … run when opened on
/// some systems), HTML, or anything this crate doesn't read.
pub(crate) fn opens_as_document(path: &Path, mime_hint: Option<&str>) -> bool {
    let Some(ext) = extension(path) else {
        return false;
    };
    let document = match FileFormat::from_extension(&ext) {
        Some(FileFormat::Html) | None => false,
        Some(FileFormat::PlainText) => TEXT_DOCUMENTS.contains(&ext.as_str()),
        Some(_) => true,
    };
    document && FileFormat::detect(path, mime_hint) == FileFormat::from_extension(&ext)
}

/// Lower-cased file extension without the dot, if any.
fn extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
}

/// MIME type without parameters, trimmed and lower-cased:
/// `"Text/HTML; charset=UTF-8"` → `"text/html"`.
fn essence(mime: &str) -> String {
    mime.split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

/// Human description of a file type for `ExtractError::Unsupported`, e.g. `".ppt"` or
/// `".bin (video/mp4)"`.
pub(crate) fn describe_file_type(path: &Path, mime_hint: Option<&str>) -> String {
    let ext = extension(path).map_or_else(|| "no extension".to_string(), |ext| format!(".{ext}"));
    match mime_hint.map(essence).filter(|mime| !mime.is_empty()) {
        Some(mime) => format!("{ext} ({mime})"),
        None => ext,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_documents_open_with_the_default_app() {
        for name in [
            "Outline.pdf",
            "notes.DOCX",
            "slides.pptx",
            "lab.ipynb",
            "README.md",
            "a.markdown",
            "notes.txt",
            "paper.tex",
        ] {
            assert!(opens_as_document(Path::new(name), None), "{name}");
        }
        for name in [
            "run.command",
            "setup.exe",
            "App.app",
            "install.pkg",
            "disk.dmg",
            "go.sh",
            "script.py",
            "code.js",
            "types.ts",
            "page.html",
            "logo.svg",
            "link.lnk",
            "site.url",
            "place.webloc",
            "app.desktop",
            "query.sql",
            "no_extension",
            "old.ppt",
        ] {
            assert!(!opens_as_document(Path::new(name), None), "{name}");
        }
        // The MIME type must agree with the extension.
        assert!(opens_as_document(
            Path::new("a.pdf"),
            Some("application/pdf")
        ));
        assert!(opens_as_document(Path::new("a.md"), Some("text/plain")));
        assert!(!opens_as_document(Path::new("a.pdf"), Some("text/html")));
        assert!(!opens_as_document(
            Path::new("a.txt"),
            Some("application/pdf")
        ));
    }

    fn detect(name: &str, mime: Option<&str>) -> Option<FileFormat> {
        FileFormat::detect(Path::new(name), mime)
    }

    #[test]
    fn extensions_are_case_insensitive() {
        assert_eq!(detect("a/Lecture.PDF", None), Some(FileFormat::Pdf));
        assert_eq!(detect("slides.PpTx", None), Some(FileFormat::Pptx));
        assert_eq!(detect("notes.DOCX", None), Some(FileFormat::Docx));
        assert_eq!(detect("lab.ipynb", None), Some(FileFormat::Notebook));
        assert_eq!(detect("page.HTM", None), Some(FileFormat::Html));
        assert_eq!(detect("README.Markdown", None), Some(FileFormat::Markdown));
        for ext in [
            "txt", "tex", "py", "java", "c", "cpp", "h", "hs", "rkt", "r", "R", "sql", "js", "ts",
        ] {
            assert_eq!(
                detect(&format!("file.{ext}"), None),
                Some(FileFormat::PlainText),
                "{ext}"
            );
        }
    }

    #[test]
    fn legacy_office_media_and_unknown_files_are_unsupported() {
        for name in [
            "old.ppt",
            "old.doc",
            "old.xls",
            "sheet.xlsx",
            "photo.png",
            "clip.mp4",
            "archive.zip",
            "Makefile",
            "no_ext",
            ".hidden",
        ] {
            assert_eq!(detect(name, None), None, "{name}");
        }
    }

    #[test]
    fn supported_mime_overrides_extension() {
        assert_eq!(
            detect("download", Some("application/pdf")),
            Some(FileFormat::Pdf)
        );
        assert_eq!(
            detect("file.bin", Some("Application/PDF; x=y")),
            Some(FileFormat::Pdf)
        );
        assert_eq!(
            detect(
                "x",
                Some("application/vnd.openxmlformats-officedocument.presentationml.presentation")
            ),
            Some(FileFormat::Pptx)
        );
        assert_eq!(
            detect(
                "x",
                Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document")
            ),
            Some(FileFormat::Docx)
        );
        assert_eq!(
            detect("x", Some("application/x-ipynb+json")),
            Some(FileFormat::Notebook)
        );
        assert_eq!(
            detect("x.txt", Some("text/html; charset=utf-8")),
            Some(FileFormat::Html)
        );
        assert_eq!(
            detect("x.txt", Some("text/markdown")),
            Some(FileFormat::Markdown)
        );
        assert_eq!(
            detect("x", Some("text/x-python")),
            Some(FileFormat::PlainText)
        );
        assert_eq!(
            detect("x.bin", Some("text/plain")),
            Some(FileFormat::PlainText)
        );
    }

    #[test]
    fn generic_text_plain_keeps_specific_text_extension() {
        assert_eq!(
            detect("notes.md", Some("text/plain")),
            Some(FileFormat::Markdown)
        );
        assert_eq!(
            detect("page.html", Some("text/plain")),
            Some(FileFormat::Html)
        );
        assert_eq!(
            detect("code.py", Some("text/plain")),
            Some(FileFormat::PlainText)
        );
    }

    #[test]
    fn unknown_mime_falls_back_to_extension() {
        assert_eq!(
            detect("slides.pptx", Some("application/octet-stream")),
            Some(FileFormat::Pptx)
        );
        assert_eq!(
            detect("x.docx", Some("application/zip")),
            Some(FileFormat::Docx)
        );
        assert_eq!(
            detect("old.ppt", Some("application/vnd.ms-powerpoint")),
            None
        );
        assert_eq!(detect("old.doc", Some("application/msword")), None);
        assert_eq!(detect("x", Some("")), None);
    }

    #[test]
    fn media_mime_hint_does_not_hide_a_supported_extension() {
        // mime_guess maps ".ts" to "video/vnd.dlna.mpeg-tts"; browsers and LMSs often send
        // "video/mp2t". TypeScript files must still be read (binary content is refused later,
        // in `text::plain_segments`).
        assert_eq!(
            detect("app.ts", Some("video/vnd.dlna.mpeg-tts")),
            Some(FileFormat::PlainText)
        );
        assert_eq!(
            detect("app.ts", Some("video/mp2t")),
            Some(FileFormat::PlainText)
        );
        assert_eq!(detect("scan.pdf", Some("image/png")), Some(FileFormat::Pdf));
        // Media files themselves stay unsupported.
        assert_eq!(detect("clip.mp4", Some("video/mp4")), None);
        assert_eq!(detect("photo", Some("image/png")), None);
    }

    #[test]
    fn plain_text_hint_never_replaces_a_known_extension() {
        for (name, expected) in [
            ("lab.ipynb", FileFormat::Notebook),
            ("x.pdf", FileFormat::Pdf),
            ("notes.docx", FileFormat::Docx),
            ("deck.pptx", FileFormat::Pptx),
            ("notes.md", FileFormat::Markdown),
            ("page.html", FileFormat::Html),
            ("code.py", FileFormat::PlainText),
        ] {
            for hint in ["text/plain", "text/plain; charset=utf-8", "text/x-python"] {
                assert_eq!(detect(name, Some(hint)), Some(expected), "{name} {hint}");
            }
        }
        // Without a known extension the hint still decides.
        assert_eq!(
            detect("download", Some("text/plain")),
            Some(FileFormat::PlainText)
        );
    }

    #[test]
    fn describe_file_type_mentions_extension_and_mime() {
        assert_eq!(describe_file_type(Path::new("a/old.PPT"), None), ".ppt");
        assert_eq!(
            describe_file_type(Path::new("clip"), Some("Video/MP4; codecs=x")),
            "no extension (video/mp4)"
        );
    }
}
