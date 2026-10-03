//! A material's local file, for a shell to open with the default app or reveal in Finder /
//! Explorer ("open the local file" on a quote card, calendar design §7.10). The facade only
//! hands over a checked path; each shell opens it itself (Tauri Rust-side through its opener
//! plugin, so the webview never gets a path; Swift through NSWorkspace).
//!
//! Checks, all of them every time:
//! - the material is a file (kind `file`) with a stored local path;
//! - the path, with symlinks resolved, lies inside its source's root: a folder source's folder,
//!   or PageLamp's download cache for Canvas files; and it is a regular file that exists (an
//!   `.app` bundle is a directory);
//! - to open it, it must be a document (`pagelamp_extract::opens_as_document`): files
//!   downloaded from Canvas carry no quarantine attribute, so an uploaded `.command`, `.app`
//!   or `.exe` would run without a Gatekeeper prompt. Revealing executes nothing and skips
//!   this check.
//!
//! The path contains the user's name: it never goes into MCP output, the diagnostic report or
//! logs.

use std::path::{Path, PathBuf};

use pagelamp_core::model::{MaterialKind, SourceKind};
use pagelamp_core::paths;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{App, AppError, AppErrorKind, Result};

/// What a shell will do with a material's local file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LocalFileUse {
    /// Open it with the default app (documents only).
    Open,
    /// Show it in Finder / Explorer.
    Reveal,
}

impl App {
    /// The checked local file of `material_id` for `purpose`, or `None` when there is none a
    /// shell may use (see the module docs). `NotFound` for an unknown material.
    pub fn material_local_file(
        &self,
        material_id: &str,
        purpose: LocalFileUse,
    ) -> Result<Option<String>> {
        let store = self.read_store()?;
        let material = store.get_material(material_id)?.ok_or_else(|| {
            AppError::new(
                AppErrorKind::NotFound,
                format!("There is no material {material_id}."),
            )
        })?;
        let Some(local_path) = material.local_path.as_deref() else {
            return Ok(None);
        };
        if material.kind != MaterialKind::File {
            return Ok(None);
        }
        let Some(course) = store.get_course(&material.course_id)? else {
            return Ok(None);
        };
        let Some(source) = store.get_source(&course.source_id)? else {
            return Ok(None);
        };
        let root: PathBuf = match source.kind {
            SourceKind::Folder => match source.config.get("path").and_then(|p| p.as_str()) {
                Some(path) => path.into(),
                None => return Ok(None),
            },
            SourceKind::Canvas => paths::files_dir_in(self.data_dir()),
            SourceKind::Ical => return Ok(None),
        };
        let Some(file) = checked_file(&root, Path::new(local_path)) else {
            return Ok(None);
        };
        if purpose == LocalFileUse::Open
            && !pagelamp_extract::opens_as_document(&file, material.mime.as_deref())
        {
            return Ok(None);
        }
        Ok(file.to_str().map(str::to_string))
    }
}

/// `path` with symlinks resolved, if it is a regular file inside `root` (also resolved).
fn checked_file(root: &Path, path: &Path) -> Option<PathBuf> {
    let root = std::fs::canonicalize(root).ok()?;
    let file = std::fs::canonicalize(path).ok()?;
    if !file.starts_with(&root) || file == root {
        return None;
    }
    // `canonicalize` followed every link, so this is the target's own type.
    std::fs::metadata(&file)
        .ok()
        .filter(|meta| meta.is_file())
        .map(|_| file)
}
