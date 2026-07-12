//! File-backed [`SaveStore`]: one JSON file per slot under `saves/`
//! (gitignored), replacing the TS original's `localStorage`. Every I/O
//! failure is logged before falling back to the trait's "empty slot"
//! semantics — never silently swallowed.

use bevy::prelude::*;
use delve_core::save_system::SaveStore;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

/// `saves/` resolved the same way `crate::assets_dir()` resolves `assets/`:
/// relative to the current directory when running from the repo root,
/// falling back to a path anchored at the crate's own location otherwise.
/// Reuses `assets_dir()`'s already-correct repo-root detection rather than
/// re-implementing it.
pub(crate) fn saves_dir() -> PathBuf {
    crate::assets_dir()
        .parent()
        .map(|repo_root| repo_root.join("saves"))
        .unwrap_or_else(|| PathBuf::from("saves"))
}

#[derive(Resource)]
pub struct FileSaveStore {
    dir: PathBuf,
}

impl FileSaveStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn path_for(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.json"))
    }
}

impl SaveStore for FileSaveStore {
    fn get_item(&self, key: &str) -> Option<String> {
        match fs::read_to_string(self.path_for(key)) {
            Ok(contents) => Some(contents),
            // No save in this slot yet — not an error, matches
            // `localStorage.getItem` returning `null` for a missing key.
            Err(error) if error.kind() == ErrorKind::NotFound => None,
            Err(error) => {
                warn!(
                    "failed to read save slot {key} ({}): {error}",
                    self.path_for(key).display()
                );
                None
            }
        }
    }

    fn set_item(&mut self, key: &str, value: String) -> Result<(), String> {
        if let Err(error) = fs::create_dir_all(&self.dir) {
            let message = format!(
                "failed to create saves directory {}: {error}",
                self.dir.display()
            );
            warn!("{message}");
            return Err(message);
        }
        fs::write(self.path_for(key), value).map_err(|error| {
            let message = format!("failed to write save slot {key}: {error}");
            warn!("{message}");
            message
        })
    }

    fn remove_item(&mut self, key: &str) {
        if let Err(error) = fs::remove_file(self.path_for(key))
            && error.kind() != ErrorKind::NotFound
        {
            warn!("failed to delete save slot {key}: {error}");
        }
    }
}
