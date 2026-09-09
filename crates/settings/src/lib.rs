//! Gritty settings (T120, ADR-031): the schema registry, the `settings.json`
//! document, and the store that reads and writes it.
//!
//! This crate knows nothing about the core's types — it takes and returns
//! plain JSON values, the way `gritty-persistence` takes and returns plain
//! rows — so the failure policy can be tested without constructing a core or
//! an event bus. It never depends on `gritty-core` or Tauri.
//!
//! The decision it implements: user configuration is not application metadata.
//! ADR-004's SQLite rule governs what the application keeps about *itself*;
//! settings are authored by the user, belong to the user, and must be
//! repairable when the application will not start. That is why they are a file,
//! and it is why the one rule this crate will not bend is that a file which
//! failed to parse is never rewritten.

pub mod document;
pub mod error;
pub mod file;
pub mod schema;
#[cfg(test)]
mod testing;

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde_json::Value;

pub use document::{Diagnostic, SettingsDocument};
pub use error::SettingsError;
pub use file::{FileState, MAX_SETTINGS_BYTES, SETTINGS_FILE_NAME, SettingsFile};
pub use schema::{
    Applies, Choice, DefaultValue, SECTIONS, SETTINGS, Scope, Section, SettingKind, SettingSpec,
};

/// Everything a caller needs to render the settings surface and to know how
/// much to trust it.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Every setting's effective value: the file overlaid on the defaults.
    pub values: BTreeMap<String, Value>,
    /// Keys the file sets and this build accepted.
    pub explicit: Vec<String>,
    /// Keys whose effective value differs from the default.
    pub modified: Vec<String>,
    /// Keys the file sets and this build rejected — a problem, never a
    /// modification.
    pub invalid: Vec<String>,
    /// Keys this build does not declare, preserved on write.
    pub unknown: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
    pub file_state: FileState,
    /// For display only. `None` when there is nowhere to store settings.
    pub path: Option<String>,
}

/// What a write did.
#[derive(Debug, Clone)]
pub struct Applied {
    pub snapshot: Snapshot,
    /// Keys whose effective value actually moved. Empty means no event is
    /// worth raising.
    pub changed: Vec<String>,
}

#[derive(Debug)]
struct State {
    document: SettingsDocument,
    file_state: FileState,
    stat: Option<(u64, Option<u64>)>,
}

/// The settings store: one document, one lock, and a file underneath it.
///
/// The lock is held across a whole read-modify-write so that a concurrent
/// external edit is merged rather than clobbered, and so that a focus re-read
/// cannot land a stale snapshot on top of a fresh write.
#[derive(Debug)]
pub struct SettingsStore {
    /// `None` when there is no application data directory. Values still apply
    /// for the session; nothing can be saved.
    file: Option<SettingsFile>,
    state: Mutex<State>,
}

impl SettingsStore {
    /// Open the store over `data_dir`, reading the file if it is there.
    pub fn open(data_dir: impl AsRef<std::path::Path>) -> Self {
        let file = SettingsFile::in_dir(data_dir);
        let loaded = file.load();
        Self {
            file: Some(file),
            state: Mutex::new(State {
                document: loaded.document,
                file_state: loaded.state,
                stat: loaded.stat,
            }),
        }
    }

    /// A store with nowhere to write. Every setting reads its default and
    /// every write is refused — the editor still works, which is the point.
    pub fn unavailable() -> Self {
        Self {
            file: None,
            state: Mutex::new(State {
                document: SettingsDocument::defaults(),
                file_state: FileState::Missing,
                stat: None,
            }),
        }
    }

    pub fn path(&self) -> Option<&std::path::Path> {
        self.file.as_ref().map(SettingsFile::path)
    }

    pub fn snapshot(&self) -> Snapshot {
        let state = self.lock();
        self.snapshot_from(&state)
    }

    /// Apply a patch. Either every key in it lands or none does: a partial
    /// write would leave the user unable to say what happened.
    pub fn set(&self, values: &BTreeMap<String, Value>) -> Result<Applied, SettingsError> {
        self.write(|document| {
            // Validate the whole patch before touching anything, so a refusal
            // leaves the document exactly as it was.
            for (key, value) in values {
                let spec =
                    schema::spec(key).ok_or_else(|| SettingsError::UnknownKey(key.clone()))?;
                document::validate(spec, value)?;
            }
            let mut changed = Vec::new();
            for (key, value) in values {
                if document.set(key, value)? {
                    changed.push(key.clone());
                }
            }
            Ok(changed)
        })
    }

    /// Return keys to their defaults. `None` resets every *known* key; unknown
    /// keys are never ours to reset.
    pub fn reset(&self, keys: Option<&[String]>) -> Result<Applied, SettingsError> {
        self.write(|document| match keys {
            None => Ok(document.reset_all()),
            Some(keys) => {
                for key in keys {
                    if schema::spec(key).is_none() {
                        return Err(SettingsError::UnknownKey(key.clone()));
                    }
                }
                let mut changed = Vec::new();
                for key in keys {
                    if document.reset(key)? {
                        changed.push(key.clone());
                    }
                }
                Ok(changed)
            }
        })
    }

    /// Re-read the file if it looks different, and report what moved.
    ///
    /// This is the focus-gain path, not a watcher: the data directory holds
    /// `gritty.sqlite-wal`, which is written every one to two seconds while any
    /// buffer is dirty, so watching it would be a self-inflicted storm
    /// (ADR-031 §6). The `(size, mtime)` guard makes the common case one
    /// `stat`.
    pub fn reload_if_changed(&self) -> Option<Applied> {
        let file = self.file.as_ref()?;
        let mut state = self.lock();
        let stat = file.stat();
        if stat == state.stat {
            return None;
        }

        let before = state.document.effective();
        let loaded = file.load();
        state.document = loaded.document;
        state.file_state = loaded.state;
        state.stat = loaded.stat;

        let changed = difference(&before, &state.document.effective());
        let snapshot = self.snapshot_from(&state);
        Some(Applied { snapshot, changed })
    }

    fn write(
        &self,
        edit: impl FnOnce(&mut SettingsDocument) -> Result<Vec<String>, SettingsError>,
    ) -> Result<Applied, SettingsError> {
        let mut state = self.lock();
        let Some(file) = self.file.as_ref() else {
            return Err(SettingsError::Unavailable);
        };

        // Re-read first: another editor may have changed the file since the
        // last load, and a write must merge with it rather than replace it.
        let loaded = file.load();
        state.document = loaded.document;
        state.file_state = loaded.state;
        state.stat = loaded.stat;

        if !state.file_state.is_writable() {
            return Err(SettingsError::Unparseable(file.path().to_path_buf()));
        }

        let changed = edit(&mut state.document)?;
        file.store(&state.document)?;
        state.file_state = FileState::Ok;
        state.stat = file.stat();

        let snapshot = self.snapshot_from(&state);
        Ok(Applied { snapshot, changed })
    }

    fn snapshot_from(&self, state: &State) -> Snapshot {
        Snapshot {
            values: state.document.effective(),
            explicit: state.document.explicit(),
            modified: state.document.modified(),
            invalid: state.document.invalid().to_vec(),
            unknown: state.document.unknown_keys(),
            diagnostics: state.document.diagnostics().to_vec(),
            file_state: state.file_state,
            path: self.path().map(|path| path.display().to_string()),
        }
    }

    /// A poisoned lock means a previous caller panicked mid-edit. The document
    /// behind it is still a valid document, so recovering is strictly better
    /// than propagating the panic into every later read.
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Keys whose value differs between two effective maps.
fn difference(before: &BTreeMap<String, Value>, after: &BTreeMap<String, Value>) -> Vec<String> {
    after
        .iter()
        .filter(|(key, value)| before.get(*key) != Some(value))
        .map(|(key, _)| key.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;
    use serde_json::json;

    fn open_store(dir: &TempDir) -> SettingsStore {
        SettingsStore::open(dir.path())
    }

    #[test]
    fn a_fresh_store_is_all_defaults_and_has_written_nothing() {
        let dir = TempDir::new("fresh");
        let store = open_store(&dir);
        let snapshot = store.snapshot();
        assert_eq!(snapshot.file_state, FileState::Missing);
        assert_eq!(snapshot.values["editor.fontSize"], json!(13));
        assert!(snapshot.modified.is_empty());
        assert!(!dir.join(SETTINGS_FILE_NAME).exists());
    }

    #[test]
    fn a_set_persists_and_reports_only_what_moved() {
        let dir = TempDir::new("set");
        let store = open_store(&dir);

        let applied = store
            .set(&BTreeMap::from([
                ("editor.fontSize".to_owned(), json!(20)),
                ("editor.minimap".to_owned(), json!(true)),
            ]))
            .unwrap();
        assert_eq!(
            applied.changed,
            vec!["editor.fontSize"],
            "minimap was already true"
        );
        assert_eq!(applied.snapshot.file_state, FileState::Ok);

        let reopened = open_store(&dir);
        assert_eq!(reopened.snapshot().values["editor.fontSize"], json!(20));
    }

    #[test]
    fn a_patch_is_all_or_nothing() {
        // A partial write would leave the user unable to say what happened:
        // some of what they clicked saved, some did not, one error.
        let dir = TempDir::new("atomic-patch");
        let store = open_store(&dir);
        store
            .set(&BTreeMap::from([("editor.fontSize".to_owned(), json!(20))]))
            .unwrap();

        let error = store
            .set(&BTreeMap::from([
                ("editor.fontSize".to_owned(), json!(24)),
                ("editor.minimap".to_owned(), json!("not a bool")),
            ]))
            .unwrap_err();
        assert!(
            matches!(error, SettingsError::InvalidValue { .. }),
            "{error:?}"
        );
        assert_eq!(
            store.snapshot().values["editor.fontSize"],
            json!(20),
            "the valid half of a refused patch must not land"
        );
    }

    #[test]
    fn an_unknown_key_is_refused_by_name_rather_than_stored() {
        let dir = TempDir::new("unknown-set");
        let store = open_store(&dir);
        let error = store
            .set(&BTreeMap::from([("editor.invented".to_owned(), json!(1))]))
            .unwrap_err();
        assert_eq!(error.to_string(), "unknown setting `editor.invented`");
    }

    #[test]
    fn a_write_merges_with_an_external_edit_instead_of_clobbering_it() {
        let dir = TempDir::new("merge");
        let store = open_store(&dir);
        store
            .set(&BTreeMap::from([("editor.fontSize".to_owned(), json!(20))]))
            .unwrap();

        // Someone edits the file in another editor while Gritty is running.
        std::fs::write(
            dir.join(SETTINGS_FILE_NAME),
            r#"{"editor.fontSize": 20, "workbench.theme": "dark"}"#,
        )
        .unwrap();

        store
            .set(&BTreeMap::from([(
                "editor.minimap".to_owned(),
                json!(false),
            )]))
            .unwrap();

        let snapshot = store.snapshot();
        assert_eq!(
            snapshot.values["workbench.theme"],
            json!("dark"),
            "their edit survived"
        );
        assert_eq!(
            snapshot.values["editor.minimap"],
            json!(false),
            "ours landed"
        );
    }

    #[test]
    fn a_write_is_refused_while_the_file_is_unparseable_and_the_bytes_are_kept() {
        let dir = TempDir::new("refuse");
        let store = open_store(&dir);
        let broken = "{ not json at all";
        std::fs::write(dir.join(SETTINGS_FILE_NAME), broken).unwrap();

        let error = store
            .set(&BTreeMap::from([("editor.fontSize".to_owned(), json!(20))]))
            .unwrap_err();
        assert!(matches!(error, SettingsError::Unparseable(_)), "{error:?}");
        assert_eq!(
            std::fs::read_to_string(dir.join(SETTINGS_FILE_NAME)).unwrap(),
            broken,
            "refusing a write must not repair the file behind the user's back"
        );
    }

    #[test]
    fn reset_returns_keys_to_their_defaults_and_leaves_unknown_ones_alone() {
        let dir = TempDir::new("reset");
        std::fs::write(
            dir.join(SETTINGS_FILE_NAME),
            r#"{"editor.fontSize": 20, "workbench.theme": "dark", "someone.elses": 1}"#,
        )
        .unwrap();
        let store = open_store(&dir);

        let applied = store.reset(Some(&["editor.fontSize".to_owned()])).unwrap();
        assert_eq!(applied.changed, vec!["editor.fontSize"]);
        assert_eq!(applied.snapshot.values["workbench.theme"], json!("dark"));

        let applied = store.reset(None).unwrap();
        assert_eq!(applied.changed, vec!["workbench.theme"]);
        assert_eq!(applied.snapshot.unknown, vec!["someone.elses"]);

        let text = std::fs::read_to_string(dir.join(SETTINGS_FILE_NAME)).unwrap();
        assert!(text.contains("someone.elses"), "not ours to reset: {text}");
    }

    #[test]
    fn reload_if_changed_costs_a_stat_when_nothing_moved() {
        let dir = TempDir::new("reload-noop");
        let store = open_store(&dir);
        store
            .set(&BTreeMap::from([("editor.fontSize".to_owned(), json!(20))]))
            .unwrap();
        assert!(store.reload_if_changed().is_none(), "no change, no event");
    }

    #[test]
    fn reload_if_changed_picks_up_an_external_edit() {
        let dir = TempDir::new("reload");
        let store = open_store(&dir);
        store
            .set(&BTreeMap::from([("editor.fontSize".to_owned(), json!(20))]))
            .unwrap();

        std::fs::write(
            dir.join(SETTINGS_FILE_NAME),
            "{\"editor.fontSize\": 20,\n \"workbench.theme\": \"light\"}\n",
        )
        .unwrap();

        let applied = store.reload_if_changed().expect("the file changed");
        assert_eq!(applied.changed, vec!["workbench.theme"]);
        assert_eq!(applied.snapshot.values["workbench.theme"], json!("light"));
    }

    #[test]
    fn an_unavailable_store_still_answers_and_refuses_to_pretend() {
        // No data directory: the editor works on defaults, and a write says so
        // rather than appearing to succeed.
        let store = SettingsStore::unavailable();
        assert_eq!(store.snapshot().values["editor.fontSize"], json!(13));
        assert!(store.path().is_none());
        let error = store
            .set(&BTreeMap::from([("editor.fontSize".to_owned(), json!(20))]))
            .unwrap_err();
        assert!(matches!(error, SettingsError::Unavailable), "{error:?}");
    }

    #[test]
    fn an_invalid_value_in_the_file_is_reported_apart_from_a_modification() {
        // A value that is not in force must not be drawn as "you changed
        // this" — it is a problem the user has to fix.
        let dir = TempDir::new("invalid");
        std::fs::write(
            dir.join(SETTINGS_FILE_NAME),
            r#"{"editor.fontSize": 900, "workbench.theme": "dark"}"#,
        )
        .unwrap();
        let snapshot = open_store(&dir).snapshot();

        assert_eq!(snapshot.invalid, vec!["editor.fontSize"]);
        assert_eq!(snapshot.modified, vec!["workbench.theme"]);
        assert!(!snapshot.explicit.contains(&"editor.fontSize".to_owned()));
        assert_eq!(snapshot.values["editor.fontSize"], json!(13));
    }
}
