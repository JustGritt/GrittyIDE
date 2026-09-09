//! `settings.json` on disk: where it is, how it is read, and the one rule that
//! makes writing it safe.
//!
//! The write is `gritty_workspace::write_file_atomic`, not a second atomic
//! writer. It takes a plain absolute path, is already used for a file outside
//! any workspace by `gritty-agents`, and is already crash-tested through its
//! own fault hooks — AGENTS.md hard rule 2 says not to build a second one.

use std::path::{Path, PathBuf};

use gritty_workspace::{AtomicWriteFault, FileStat};

use crate::document::SettingsDocument;
use crate::error::SettingsError;

/// Beside `gritty.sqlite` in the application data directory.
pub const SETTINGS_FILE_NAME: &str = "settings.json";

/// Largest file we will parse. A settings file is a few hundred bytes; a
/// hundred-megabyte one is a mistake or an attack, and refusing it *without
/// parsing* is the cheap half of hard rule 13.
pub const MAX_SETTINGS_BYTES: u64 = 256 * 1024;

/// What reading the file found. Distinguished because the UI draws each one
/// differently, and because only some of them permit a write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileState {
    /// No file yet. Normal, not an error — it is how "everything is default"
    /// is spelled, and nothing is written to change that.
    Missing,
    /// Read and understood.
    Ok,
    /// Present but not valid JSON, or not an object. Defaults apply and the
    /// file is never rewritten.
    Unparseable,
    /// Present but unreadable — permissions, or larger than the cap.
    Unreadable,
}

impl FileState {
    /// Whether a write may proceed. Refusing on `Unparseable` is the rule that
    /// keeps a typo from costing the user their file.
    pub fn is_writable(self) -> bool {
        matches!(self, Self::Missing | Self::Ok)
    }
}

/// The result of a read.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub document: SettingsDocument,
    pub state: FileState,
    /// Size and modification time, for the cheap "did anything change" guard
    /// on the focus re-read. `None` when there is no file.
    pub stat: Option<(u64, Option<u64>)>,
}

/// The settings file at a known path.
#[derive(Debug, Clone)]
pub struct SettingsFile {
    path: PathBuf,
}

impl SettingsFile {
    /// The settings file inside `data_dir`.
    pub fn in_dir(data_dir: impl AsRef<Path>) -> Self {
        Self {
            path: data_dir.as_ref().join(SETTINGS_FILE_NAME),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Size and modification time without reading the contents. Used to skip
    /// the re-read when a window regains focus and nothing has changed.
    pub fn stat(&self) -> Option<(u64, Option<u64>)> {
        let metadata = std::fs::metadata(&self.path).ok()?;
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|since| since.as_millis() as u64);
        Some((metadata.len(), modified))
    }

    /// Read the file. Never fails: every failure mode becomes a state plus a
    /// diagnostic on a document that still answers with defaults, because an
    /// editor that will not start is a worse answer than one that starts and
    /// says what is wrong.
    pub fn load(&self) -> Loaded {
        let metadata = match std::fs::metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Loaded {
                    document: SettingsDocument::defaults(),
                    state: FileState::Missing,
                    stat: None,
                };
            }
            Err(error) => return self.unreadable(&format!("could not be read: {error}")),
        };

        if metadata.len() > MAX_SETTINGS_BYTES {
            return self.unreadable(&format!(
                "is {} bytes, larger than the {MAX_SETTINGS_BYTES}-byte limit, so it was not parsed",
                metadata.len()
            ));
        }

        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(error) => return self.unreadable(&format!("could not be read: {error}")),
        };

        let document = SettingsDocument::parse(&text);
        let state = if document.is_poisoned() {
            FileState::Unparseable
        } else {
            FileState::Ok
        };
        Loaded {
            document,
            state,
            stat: self.stat(),
        }
    }

    fn unreadable(&self, why: &str) -> Loaded {
        let mut document = SettingsDocument::defaults();
        document.push_file_diagnostic(format!("{} {why}.", self.path.display()));
        Loaded {
            document,
            state: FileState::Unreadable,
            stat: self.stat(),
        }
    }

    /// Write the document, atomically. The parent directory is created if it
    /// is missing, which is normal on a first run.
    pub fn store(&self, document: &SettingsDocument) -> Result<FileStat, SettingsError> {
        let text = document.to_json(&self.path)?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| SettingsError::io("create", parent, error))?;
        }
        gritty_workspace::write_file_atomic(&self.path, text.as_bytes(), AtomicWriteFault::None)
            .map_err(|error| SettingsError::Write {
                path: self.path.clone(),
                reason: error.to_string(),
            })
    }

    /// The same write, with a fault injected. Test-only: hard rule 12 says
    /// prove the recovery behaviour rather than reason about it.
    #[cfg(test)]
    pub fn store_with_fault(
        &self,
        document: &SettingsDocument,
        fault: AtomicWriteFault,
    ) -> Result<FileStat, SettingsError> {
        let text = document.to_json(&self.path)?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| SettingsError::io("create", parent, error))?;
        }
        gritty_workspace::write_file_atomic(&self.path, text.as_bytes(), fault).map_err(|error| {
            SettingsError::Write {
                path: self.path.clone(),
                reason: error.to_string(),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;
    use serde_json::json;

    fn file(dir: &TempDir) -> SettingsFile {
        SettingsFile::in_dir(dir.path())
    }

    #[test]
    fn a_missing_file_is_not_an_error_and_writes_nothing() {
        let dir = TempDir::new("missing");
        let file = file(&dir);
        let loaded = file.load();
        assert_eq!(loaded.state, FileState::Missing);
        assert!(
            loaded.document.diagnostics().is_empty(),
            "absence is not a complaint"
        );
        assert!(!file.path().exists(), "loading must not create the file");
    }

    #[test]
    fn a_written_file_reads_back_the_same() {
        let dir = TempDir::new("roundtrip");
        let file = file(&dir);
        let mut document = SettingsDocument::defaults();
        document.set("editor.fontSize", &json!(21)).unwrap();
        file.store(&document).unwrap();

        let loaded = file.load();
        assert_eq!(loaded.state, FileState::Ok);
        assert_eq!(loaded.document.get("editor.fontSize"), Some(json!(21)));
        assert!(loaded.stat.is_some());
    }

    #[test]
    fn an_unparseable_file_is_never_rewritten() {
        // ADR-031 §5's strict rule, and the reason a file was chosen over a
        // database: the bytes on disk are almost always something a human was
        // editing, and overwriting them destroys the work *and* the property
        // that made the format worth choosing.
        let dir = TempDir::new("poison");
        let file = file(&dir);
        let broken = "{ \"editor.fontSize\": 20,\n  oops\n}";
        std::fs::write(file.path(), broken).unwrap();

        let loaded = file.load();
        assert_eq!(loaded.state, FileState::Unparseable);
        assert!(!loaded.state.is_writable());
        assert_eq!(
            loaded.document.get("editor.fontSize"),
            Some(json!(13)),
            "defaults apply"
        );

        let error = file.store(&loaded.document).unwrap_err();
        assert!(matches!(error, SettingsError::Unparseable(_)), "{error:?}");
        assert_eq!(
            std::fs::read_to_string(file.path()).unwrap(),
            broken,
            "the bytes are untouched"
        );

        let diagnostic = &loaded.document.diagnostics()[0];
        assert!(diagnostic.line.is_some(), "the user needs to be told where");
    }

    #[test]
    fn a_file_over_the_cap_is_refused_without_being_parsed() {
        let dir = TempDir::new("huge");
        let file = file(&dir);
        let padding = "x".repeat(MAX_SETTINGS_BYTES as usize + 1);
        std::fs::write(file.path(), padding).unwrap();

        let loaded = file.load();
        assert_eq!(loaded.state, FileState::Unreadable);
        assert!(!loaded.state.is_writable());
        assert!(loaded.document.diagnostics()[0].message.contains("limit"));
    }

    #[test]
    fn a_write_that_fails_before_the_rename_leaves_the_previous_file_intact() {
        // Forced rather than reasoned about (hard rule 12). This is the whole
        // reason the write is atomic.
        let dir = TempDir::new("fault");
        let file = file(&dir);

        let mut good = SettingsDocument::defaults();
        good.set("editor.fontSize", &json!(15)).unwrap();
        file.store(&good).unwrap();
        let before = std::fs::read_to_string(file.path()).unwrap();

        let mut next = SettingsDocument::defaults();
        next.set("editor.fontSize", &json!(30)).unwrap();
        let error = file
            .store_with_fault(&next, AtomicWriteFault::FailBeforeRename)
            .unwrap_err();
        assert!(matches!(error, SettingsError::Write { .. }), "{error:?}");

        assert_eq!(
            std::fs::read_to_string(file.path()).unwrap(),
            before,
            "a failed write must not damage what was there"
        );
        assert_eq!(file.load().document.get("editor.fontSize"), Some(json!(15)));
    }

    #[test]
    fn the_parent_directory_is_created_on_a_first_run() {
        let dir = TempDir::new("firstrun");
        let nested = dir.join("not/created/yet");
        let file = SettingsFile::in_dir(&nested);
        let mut document = SettingsDocument::defaults();
        document.set("workbench.theme", &json!("dark")).unwrap();
        file.store(&document).unwrap();
        assert_eq!(
            file.load().document.get("workbench.theme"),
            Some(json!("dark"))
        );
    }
}
