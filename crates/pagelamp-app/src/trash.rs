//! Moving downloaded course files to the system Trash (calendar design §8.3, D44).
//!
//! - macOS: `NSFileManager.trashItem` only (the `trash` crate's `NsFileManager` method), never
//!   the Finder/AppleScript method, which asks for the Automation permission and plays a sound.
//! - Windows: the Recycle Bin; Linux: the freedesktop trash.
//! - A failure (no Trash on that volume, a network drive, a permission error) leaves the file
//!   where it is and is reported. Nothing here ever deletes permanently: that only happens on
//!   the student's explicit "Delete permanently" (`purge_removed_courses`).

use std::path::Path;
use std::sync::{Arc, RwLock};

/// Where removed files go.
pub trait FileTrash: Send + Sync {
    /// Move `path` (a file or a directory) to the Trash; `Err` with a short reason otherwise.
    fn trash(&self, path: &Path) -> Result<(), String>;
}

/// The system Trash.
pub struct SystemTrash;

impl FileTrash for SystemTrash {
    fn trash(&self, path: &Path) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        let result = {
            use trash::macos::{DeleteMethod, TrashContextExtMacos};
            let mut context = trash::TrashContext::default();
            context.set_delete_method(DeleteMethod::NsFileManager);
            context.delete(path)
        };
        #[cfg(not(target_os = "macos"))]
        let result = trash::delete(path);
        result.map_err(|err| match err {
            trash::Error::CouldNotAccess { .. } => "couldn't be reached".to_string(),
            trash::Error::TargetedRoot => "is a drive's root".to_string(),
            _ => "couldn't be moved to the Trash".to_string(),
        })
    }
}

/// The app's Trash (the system one; tests put in their own).
pub(crate) struct TrashSlot(RwLock<Arc<dyn FileTrash>>);

impl Default for TrashSlot {
    fn default() -> Self {
        TrashSlot(RwLock::new(Arc::new(SystemTrash)))
    }
}

impl TrashSlot {
    pub(crate) fn get(&self) -> Arc<dyn FileTrash> {
        Arc::clone(&self.0.read().unwrap_or_else(|e| e.into_inner()))
    }

    pub(crate) fn set(&self, trash: Arc<dyn FileTrash>) {
        *self.0.write().unwrap_or_else(|e| e.into_inner()) = trash;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Touches the real Trash of this computer: run by hand.
    #[test]
    #[ignore = "moves a temporary file to this computer's real Trash"]
    fn a_temporary_file_goes_to_the_real_trash() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("pagelamp-trash-test.txt");
        std::fs::write(&file, b"demo").unwrap();
        SystemTrash.trash(&file).unwrap();
        assert!(!file.exists());
    }
}
