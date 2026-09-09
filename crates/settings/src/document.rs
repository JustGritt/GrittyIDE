//! The settings document: what the file says, overlaid on the defaults.
//!
//! Three rules from ADR-031 §5 live here, and each one is a test below:
//!
//! - a **known key with an invalid value** falls back to its own default alone
//!   and earns a diagnostic; its neighbours still apply;
//! - **unknown keys are preserved verbatim** across a write, so an older build
//!   never eats a key a newer one added;
//! - a document parsed from **unparseable text is poisoned** and refuses to be
//!   serialized at all, so a typo cannot cost the user the file they were
//!   editing.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::error::SettingsError;
use crate::schema::{SETTINGS, SettingKind, SettingSpec, spec};

/// Something the user should know about their settings file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// The setting this concerns, or `None` for the file as a whole.
    pub key: Option<String>,
    pub message: String,
    /// 1-based position of a parse failure, when there is one.
    pub line: Option<usize>,
    pub column: Option<usize>,
}

/// The values a file holds, plus what could not be understood about it.
#[derive(Debug, Clone, Default)]
pub struct SettingsDocument {
    /// Known keys the file set, already validated.
    known: BTreeMap<String, Value>,
    /// Keys this build does not declare, kept exactly as they were read.
    unknown: Map<String, Value>,
    /// Keys present in the file but rejected. Tracked apart from `known`
    /// because a value that is not in force must not be drawn as "you changed
    /// this" — it is a problem, not a modification.
    invalid: Vec<String>,
    diagnostics: Vec<Diagnostic>,
    /// The text did not parse. Everything still works on defaults, and the
    /// file is never written back.
    poisoned: bool,
}

impl SettingsDocument {
    /// A document with nothing set — every setting at its default.
    pub fn defaults() -> Self {
        Self::default()
    }

    /// Read a document from the file's text. Never fails: unparseable text
    /// yields a poisoned document on defaults, because refusing to start is a
    /// worse answer than starting with a diagnostic.
    pub fn parse(text: &str) -> Self {
        let parsed: Value = match serde_json::from_str(text) {
            Ok(value) => value,
            Err(error) => {
                return Self {
                    diagnostics: vec![Diagnostic {
                        key: None,
                        message: format!("settings.json could not be parsed: {error}"),
                        line: Some(error.line()),
                        column: Some(error.column()),
                    }],
                    poisoned: true,
                    ..Self::default()
                };
            }
        };

        let Value::Object(object) = parsed else {
            return Self {
                diagnostics: vec![Diagnostic {
                    key: None,
                    message: "settings.json must contain a JSON object.".to_owned(),
                    line: None,
                    column: None,
                }],
                poisoned: true,
                ..Self::default()
            };
        };

        let mut document = Self::default();
        for (key, value) in object {
            let Some(spec) = spec(&key) else {
                // Not ours. Keep it byte-for-byte so a downgrade, or a build
                // whose feature was removed, does not silently delete it.
                document.unknown.insert(key, value);
                continue;
            };
            match validate(spec, &value) {
                Ok(normalized) => {
                    document.known.insert(key, normalized);
                }
                Err(error) => {
                    document.diagnostics.push(Diagnostic {
                        key: Some(key.clone()),
                        message: format!("{error}; the default is being used instead."),
                        line: None,
                        column: None,
                    });
                    document.invalid.push(key);
                }
            }
        }
        document
    }

    /// Whether the file this came from failed to parse.
    pub fn is_poisoned(&self) -> bool {
        self.poisoned
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Record something about the file as a whole — used by the reader for a
    /// failure that happens before any parsing.
    pub fn push_file_diagnostic(&mut self, message: String) {
        self.diagnostics.push(Diagnostic {
            key: None,
            message,
            line: None,
            column: None,
        });
    }

    /// Keys the file sets and this build accepted.
    pub fn explicit(&self) -> Vec<String> {
        self.known.keys().cloned().collect()
    }

    /// Keys the file sets and this build rejected.
    pub fn invalid(&self) -> &[String] {
        &self.invalid
    }

    /// Keys the file sets that this build does not declare. Preserved on
    /// write, and information rather than an error — a downgrade must be
    /// quiet, not noisy.
    pub fn unknown_keys(&self) -> Vec<String> {
        self.unknown.keys().cloned().collect()
    }

    /// The effective value of `key`: what the file says, else the default.
    pub fn get(&self, key: &str) -> Option<Value> {
        let spec = spec(key)?;
        Some(
            self.known
                .get(key)
                .cloned()
                .unwrap_or_else(|| spec.default.to_value()),
        )
    }

    /// Every setting's effective value.
    pub fn effective(&self) -> BTreeMap<String, Value> {
        SETTINGS
            .iter()
            .map(|spec| {
                let value = self
                    .known
                    .get(spec.key)
                    .cloned()
                    .unwrap_or_else(|| spec.default.to_value());
                (spec.key.to_owned(), value)
            })
            .collect()
    }

    /// Keys whose effective value differs from the default — what the UI marks
    /// as modified, and what a "reset all" would change.
    pub fn modified(&self) -> Vec<String> {
        SETTINGS
            .iter()
            .filter(|spec| {
                self.known
                    .get(spec.key)
                    .is_some_and(|value| *value != spec.default.to_value())
            })
            .map(|spec| spec.key.to_owned())
            .collect()
    }

    /// Set one key. Returns whether the effective value actually changed, so a
    /// no-op write raises no event.
    pub fn set(&mut self, key: &str, value: &Value) -> Result<bool, SettingsError> {
        let spec = spec(key).ok_or_else(|| SettingsError::UnknownKey(key.to_owned()))?;
        let normalized = validate(spec, value)?;
        let previous = self.get(key);
        let changed = previous.as_ref() != Some(&normalized);
        self.known.insert(key.to_owned(), normalized);
        Ok(changed)
    }

    /// Drop one key, returning it to its default. Returns whether anything
    /// changed.
    pub fn reset(&mut self, key: &str) -> Result<bool, SettingsError> {
        let spec = spec(key).ok_or_else(|| SettingsError::UnknownKey(key.to_owned()))?;
        match self.known.remove(key) {
            Some(previous) => Ok(previous != spec.default.to_value()),
            None => Ok(false),
        }
    }

    /// Return every setting to its default. Unknown keys are **not** touched —
    /// they are not ours to reset.
    pub fn reset_all(&mut self) -> Vec<String> {
        let changed = self.modified();
        self.known.clear();
        changed
    }

    /// The document as it should be written: known keys and preserved unknown
    /// ones, sorted, with a trailing newline.
    ///
    /// Refuses on a poisoned document — that is the rule that keeps a typo
    /// from costing the user their file.
    pub fn to_json(&self, path: &std::path::Path) -> Result<String, SettingsError> {
        if self.poisoned {
            return Err(SettingsError::Unparseable(path.to_path_buf()));
        }
        let mut object = Map::new();
        for (key, value) in &self.known {
            object.insert(key.clone(), value.clone());
        }
        for (key, value) in &self.unknown {
            object.insert(key.clone(), value.clone());
        }
        let mut text = serde_json::to_string_pretty(&Value::Object(object))
            .expect("a map of JSON values always serializes");
        text.push('\n');
        Ok(text)
    }
}

/// Check `value` against `spec`, returning the value to store.
///
/// Normalizes as well as checks: a path list is trimmed, deduplicated and
/// bounded here rather than in the UI, because a hand-edited file never passes
/// through a form.
pub fn validate(spec: &SettingSpec, value: &Value) -> Result<Value, SettingsError> {
    let invalid = |expected: &str| SettingsError::InvalidValue {
        key: spec.key.to_owned(),
        expected: expected.to_owned(),
    };

    match spec.kind {
        SettingKind::Bool => value
            .as_bool()
            .map(Value::Bool)
            .ok_or_else(|| invalid("true or false")),

        SettingKind::Integer { min, max } => {
            let number = value.as_i64().ok_or_else(|| invalid("a whole number"))?;
            if number < min || number > max {
                return Err(invalid(&format!("a whole number between {min} and {max}")));
            }
            Ok(Value::Number(number.into()))
        }

        SettingKind::Enum { choices } => {
            let text = value
                .as_str()
                .ok_or_else(|| invalid("one of its choices"))?;
            if choices.iter().any(|choice| choice.value == text) {
                Ok(Value::String(text.to_owned()))
            } else {
                let names: Vec<&str> = choices.iter().map(|choice| choice.value).collect();
                Err(invalid(&format!("one of: {}", names.join(", "))))
            }
        }

        SettingKind::NullableString { max_len } => {
            if value.is_null() {
                return Ok(Value::Null);
            }
            let text = value.as_str().ok_or_else(|| invalid("text, or null"))?;
            if text.chars().count() > max_len {
                return Err(invalid(&format!("at most {max_len} characters, or null")));
            }
            Ok(Value::String(text.to_owned()))
        }

        SettingKind::NullablePath => {
            if value.is_null() {
                return Ok(Value::Null);
            }
            let text = value
                .as_str()
                .ok_or_else(|| invalid("an absolute path, or null"))?;
            let trimmed = text.trim();
            if !is_absolute(trimmed) {
                return Err(invalid("an absolute path, or null"));
            }
            Ok(Value::String(trimmed.to_owned()))
        }

        SettingKind::PathList { max_len } => {
            let items = value
                .as_array()
                .ok_or_else(|| invalid("a list of absolute paths"))?;
            let mut paths: Vec<String> = Vec::new();
            for item in items {
                let text = item
                    .as_str()
                    .ok_or_else(|| invalid("a list of absolute paths"))?
                    .trim();
                if !is_absolute(text) {
                    return Err(invalid("a list of absolute paths"));
                }
                let owned = text.to_owned();
                if !paths.contains(&owned) {
                    paths.push(owned);
                }
            }
            if paths.len() > max_len {
                return Err(invalid(&format!("at most {max_len} paths")));
            }
            Ok(Value::Array(paths.into_iter().map(Value::String).collect()))
        }
    }
}

/// Absolute in the platform's own terms. Deliberately a string check and not a
/// filesystem one: a shell that is not installed on *this* machine is still a
/// legitimate entry in a settings file shared between machines, and refusing it
/// here would make a dotfile non-portable.
fn is_absolute(path: &str) -> bool {
    !path.is_empty() && std::path::Path::new(path).is_absolute()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_empty_document_is_every_default() {
        let document = SettingsDocument::defaults();
        assert_eq!(document.get("editor.fontSize"), Some(json!(13)));
        assert_eq!(document.get("workbench.theme"), Some(json!("system")));
        assert!(document.modified().is_empty());
    }

    #[test]
    fn a_missing_key_falls_back_without_a_diagnostic() {
        let document = SettingsDocument::parse(r#"{"editor.fontSize": 20}"#);
        assert_eq!(document.get("editor.fontSize"), Some(json!(20)));
        assert_eq!(document.get("editor.minimap"), Some(json!(true)));
        assert!(document.diagnostics().is_empty());
    }

    #[test]
    fn an_invalid_value_falls_back_alone_and_does_not_poison_its_neighbours() {
        // The rule from ADR-031 §5 that matters most in practice: one bad line
        // in a hand-edited file must not cost the user every other line.
        let document = SettingsDocument::parse(
            r#"{"editor.fontSize": 900, "editor.minimap": false, "workbench.theme": "dark"}"#,
        );
        assert_eq!(
            document.get("editor.fontSize"),
            Some(json!(13)),
            "fell back"
        );
        assert_eq!(
            document.get("editor.minimap"),
            Some(json!(false)),
            "neighbour survived"
        );
        assert_eq!(
            document.get("workbench.theme"),
            Some(json!("dark")),
            "neighbour survived"
        );

        let diagnostics = document.diagnostics();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].key.as_deref(), Some("editor.fontSize"));
        assert!(
            diagnostics[0].message.contains("between 8 and 32"),
            "the diagnostic should say what would have been valid: {}",
            diagnostics[0].message
        );
    }

    #[test]
    fn unknown_keys_survive_a_round_trip() {
        // A key a newer build added, opened by an older one. Losing it would
        // mean downgrading once silently deletes configuration.
        let document = SettingsDocument::parse(
            r#"{"editor.fontSize": 20, "editor.fromTheFuture": {"nested": [1, 2]}}"#,
        );
        let written = document
            .to_json(std::path::Path::new("settings.json"))
            .unwrap();
        assert!(written.contains("editor.fromTheFuture"), "{written}");
        assert!(written.contains("nested"), "{written}");

        let reread = SettingsDocument::parse(&written);
        assert_eq!(reread.get("editor.fontSize"), Some(json!(20)));
    }

    #[test]
    fn a_poisoned_document_refuses_to_be_written() {
        // The strict rule: a file that failed to parse is almost always one a
        // human was editing, and overwriting it destroys their work.
        let document = SettingsDocument::parse("{ this is not json");
        assert!(document.is_poisoned());
        assert_eq!(
            document.get("editor.fontSize"),
            Some(json!(13)),
            "defaults still apply"
        );

        let error = document
            .to_json(std::path::Path::new("/tmp/settings.json"))
            .unwrap_err();
        assert!(matches!(error, SettingsError::Unparseable(_)), "{error:?}");

        let diagnostics = document.diagnostics();
        assert_eq!(diagnostics.len(), 1);
        assert!(
            diagnostics[0].line.is_some(),
            "a parse failure carries its position"
        );
    }

    #[test]
    fn a_json_array_is_not_a_settings_file() {
        let document = SettingsDocument::parse("[1, 2, 3]");
        assert!(document.is_poisoned());
        assert!(document.diagnostics()[0].message.contains("object"));
    }

    #[test]
    fn set_reports_whether_anything_changed() {
        let mut document = SettingsDocument::defaults();
        assert!(document.set("editor.fontSize", &json!(20)).unwrap());
        assert!(
            !document.set("editor.fontSize", &json!(20)).unwrap(),
            "a no-op raises no event"
        );
        assert!(document.set("editor.fontSize", &json!(21)).unwrap());
    }

    #[test]
    fn set_refuses_an_unknown_key_by_name() {
        let mut document = SettingsDocument::defaults();
        let error = document.set("editor.nope", &json!(1)).unwrap_err();
        assert!(matches!(&error, SettingsError::UnknownKey(key) if key == "editor.nope"));
    }

    #[test]
    fn set_refuses_a_wrong_type_and_says_what_was_expected() {
        let mut document = SettingsDocument::defaults();
        let error = document.set("editor.minimap", &json!("yes")).unwrap_err();
        assert_eq!(error.to_string(), "`editor.minimap` expects true or false");
    }

    #[test]
    fn reset_returns_a_key_to_its_default_and_reset_all_keeps_unknown_keys() {
        let mut document =
            SettingsDocument::parse(r#"{"editor.fontSize": 20, "someone.elses": true}"#);
        assert_eq!(document.modified(), vec!["editor.fontSize"]);

        assert!(document.reset("editor.fontSize").unwrap());
        assert_eq!(document.get("editor.fontSize"), Some(json!(13)));
        assert!(
            !document.reset("editor.fontSize").unwrap(),
            "already default"
        );

        document.set("editor.minimap", &json!(false)).unwrap();
        assert_eq!(document.reset_all(), vec!["editor.minimap"]);

        let written = document
            .to_json(std::path::Path::new("settings.json"))
            .unwrap();
        assert!(
            written.contains("someone.elses"),
            "reset all is not ours to apply to: {written}"
        );
    }

    #[test]
    fn a_path_list_is_deduplicated_trimmed_and_bounded() {
        let mut document = SettingsDocument::defaults();
        document
            .set(
                "terminal.customShells",
                &json!(["/bin/zsh", " /bin/zsh ", "/bin/bash"]),
            )
            .unwrap();
        assert_eq!(
            document.get("terminal.customShells"),
            Some(json!(["/bin/zsh", "/bin/bash"]))
        );

        let too_many: Vec<String> = (0..20).map(|n| format!("/bin/sh{n}")).collect();
        let error = document
            .set("terminal.customShells", &json!(too_many))
            .unwrap_err();
        assert!(error.to_string().contains("at most 16 paths"), "{error}");
    }

    #[test]
    fn a_relative_shell_path_is_refused() {
        // The shell path is the one setting that names something the core will
        // execute, so it does not get to be relative to a working directory
        // nobody has pinned down.
        let mut document = SettingsDocument::defaults();
        assert!(document.set("terminal.shell", &json!("zsh")).is_err());
        assert!(document.set("terminal.shell", &json!("")).is_err());
        assert!(document.set("terminal.shell", &json!("/bin/zsh")).is_ok());
        assert!(
            document.set("terminal.shell", &Value::Null).is_ok(),
            "null is 'system default'"
        );
    }

    #[test]
    fn an_uninstalled_shell_is_still_a_valid_setting() {
        // Validation is a string check, not a filesystem one, so a settings
        // file stays portable between machines.
        let mut document = SettingsDocument::defaults();
        assert!(
            document
                .set("terminal.shell", &json!("/opt/nowhere/fish"))
                .is_ok()
        );
    }

    #[test]
    fn the_written_document_is_stable_and_ends_with_a_newline() {
        let mut document = SettingsDocument::defaults();
        document.set("workbench.theme", &json!("dark")).unwrap();
        document.set("editor.fontSize", &json!(15)).unwrap();
        let once = document
            .to_json(std::path::Path::new("settings.json"))
            .unwrap();
        let twice = SettingsDocument::parse(&once)
            .to_json(std::path::Path::new("settings.json"))
            .unwrap();
        assert_eq!(once, twice, "a round trip must not churn the file");
        assert!(once.ends_with('\n'));
    }
}
