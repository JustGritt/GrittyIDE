//! The schema registry: every setting Gritty has, declared exactly once.
//!
//! ADR-031 §3 — this table both validates writes here and is what
//! `settings.describe` serializes for the UI, so a setting is declared once and
//! its control is generated. The alternative, hand-written forms, means
//! maintaining validation, search and reset-to-default twice, and they drift.
//!
//! Adding a setting is adding a row. Nothing else in this crate changes.

use serde_json::Value;

/// Where a setting may be written. Only [`Scope::User`] exists today.
///
/// The field is here from the first commit so workspace scope is additive
/// rather than a migration (ADR-031 §4). It is deliberately not honoured yet: a
/// repo-local `.gritty/` directory is forbidden by Architecture Refinements §14
/// until its ownership and git behaviour are defined, and a workspace-scoped
/// setting that can name an executable is arbitrary code execution on clone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Global to the user, stored in `settings.json`.
    User,
}

/// When a change takes effect. Rendered as a hint, because saying so is
/// cheaper and more honest than pretending every setting is live.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applies {
    /// Immediately, on the surfaces already open.
    Live,
    /// To terminals started after the change; running ones keep their shell.
    NewTerminals,
    // Deliberately no `Restart`: nothing in v1 needs one, and a variant with no
    // member is the speculative surface hard rule 9 refuses. Adding it later is
    // additive.
}

/// One choice in an [`SettingKind::Enum`], with the label the UI shows.
#[derive(Debug, Clone, Copy)]
pub struct Choice {
    pub value: &'static str,
    pub label: &'static str,
}

/// The type of a setting, and the bounds that make a value valid.
///
/// Every variant carries its own limits rather than leaving them to the UI:
/// the core is the one that must refuse a bad value, because a hand-edited
/// file never passes through a form.
#[derive(Debug, Clone, Copy)]
pub enum SettingKind {
    Bool,
    /// A whole number within an inclusive range.
    Integer {
        min: i64,
        max: i64,
    },
    /// One of a fixed set of strings.
    Enum {
        choices: &'static [Choice],
    },
    /// Free text, or `null` for "unset, use the built-in".
    NullableString {
        max_len: usize,
    },
    /// An absolute path, or `null` for "whatever the system default is".
    NullablePath,
    /// A list of absolute paths, deduplicated and bounded.
    PathList {
        max_len: usize,
    },
}

/// A setting's default, in a form that can be written in a `const`.
#[derive(Debug, Clone, Copy)]
pub enum DefaultValue {
    Bool(bool),
    Integer(i64),
    Str(&'static str),
    /// "Unset" — the surface falls through to its own built-in.
    Null,
    EmptyList,
}

impl DefaultValue {
    /// The default as the JSON value an effective-settings map holds.
    pub fn to_value(self) -> Value {
        match self {
            Self::Bool(value) => Value::Bool(value),
            Self::Integer(value) => Value::Number(value.into()),
            Self::Str(value) => Value::String(value.to_owned()),
            Self::Null => Value::Null,
            Self::EmptyList => Value::Array(Vec::new()),
        }
    }
}

/// One setting.
#[derive(Debug, Clone, Copy)]
pub struct SettingSpec {
    /// The dotted key, unique across the registry and stable forever — it is
    /// what a hand-edited file names.
    pub key: &'static str,
    /// The section it renders under, matched against [`SECTIONS`].
    pub section: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub kind: SettingKind,
    pub default: DefaultValue,
    pub scope: Scope,
    pub applies: Applies,
}

/// A group of settings, in the order the page shows them.
#[derive(Debug, Clone, Copy)]
pub struct Section {
    pub id: &'static str,
    pub label: &'static str,
}

pub const SECTIONS: &[Section] = &[
    Section {
        id: "workbench",
        label: "Workbench",
    },
    Section {
        id: "editor",
        label: "Editor",
    },
    Section {
        id: "terminal",
        label: "Terminal",
    },
    Section {
        id: "agents",
        label: "Agents",
    },
];

const THEME_CHOICES: &[Choice] = &[
    Choice {
        value: "system",
        label: "Follow the system",
    },
    Choice {
        value: "light",
        label: "Light",
    },
    Choice {
        value: "dark",
        label: "Dark",
    },
];

const TAB_SIZE_CHOICES: &[Choice] = &[
    Choice {
        value: "2",
        label: "2 spaces",
    },
    Choice {
        value: "4",
        label: "4 spaces",
    },
    Choice {
        value: "8",
        label: "8 spaces",
    },
];

/// The most custom shells the picker will hold. Matches the UI's existing
/// `MAX_CUSTOM_SHELLS`, which this setting takes over from the layout blob.
pub const MAX_CUSTOM_SHELLS: usize = 16;

/// Every setting, in registry order.
///
/// The v1 set is deliberately small: ADR-031 §7's migrated values plus
/// `workbench.theme`. Turning any other hardcoded value into a setting is a
/// follow-up, not a quiet addition here (AGENTS.md rule 14).
pub const SETTINGS: &[SettingSpec] = &[
    SettingSpec {
        key: "workbench.theme",
        section: "workbench",
        label: "Theme",
        description: "Which colour theme to use. \"Follow the system\" tracks the operating system's light or dark appearance.",
        kind: SettingKind::Enum {
            choices: THEME_CHOICES,
        },
        default: DefaultValue::Str("system"),
        scope: Scope::User,
        applies: Applies::Live,
    },
    SettingSpec {
        key: "editor.fontSize",
        section: "editor",
        label: "Font size",
        description: "Font size in the editor, in pixels.",
        kind: SettingKind::Integer { min: 8, max: 32 },
        default: DefaultValue::Integer(13),
        scope: Scope::User,
        applies: Applies::Live,
    },
    SettingSpec {
        key: "editor.tabSize",
        section: "editor",
        label: "Tab size",
        description: "How many spaces a tab counts for.",
        kind: SettingKind::Enum {
            choices: TAB_SIZE_CHOICES,
        },
        default: DefaultValue::Str("4"),
        scope: Scope::User,
        applies: Applies::Live,
    },
    SettingSpec {
        key: "editor.wordWrap",
        section: "editor",
        label: "Word wrap",
        description: "Wrap long lines at the width of the editor instead of scrolling sideways.",
        kind: SettingKind::Bool,
        default: DefaultValue::Bool(false),
        scope: Scope::User,
        applies: Applies::Live,
    },
    SettingSpec {
        key: "editor.minimap",
        section: "editor",
        label: "Minimap",
        description: "Show the document overview down the right-hand edge of the editor.",
        kind: SettingKind::Bool,
        default: DefaultValue::Bool(true),
        scope: Scope::User,
        applies: Applies::Live,
    },
    SettingSpec {
        key: "terminal.shell",
        section: "terminal",
        label: "Shell",
        description: "Absolute path of the shell new terminals start. Unset means the system default.",
        kind: SettingKind::NullablePath,
        default: DefaultValue::Null,
        scope: Scope::User,
        applies: Applies::NewTerminals,
    },
    SettingSpec {
        key: "terminal.customShells",
        section: "terminal",
        label: "Custom shells",
        description: "Shells added by hand. Discovery only reports what $SHELL and /etc/shells name, so this is how a shell the system does not register reaches the picker.",
        kind: SettingKind::PathList {
            max_len: MAX_CUSTOM_SHELLS,
        },
        default: DefaultValue::EmptyList,
        scope: Scope::User,
        applies: Applies::Live,
    },
    SettingSpec {
        key: "terminal.fontFamily",
        section: "terminal",
        label: "Font family",
        description: "Font stack for the terminal. Unset uses the built-in stack.",
        kind: SettingKind::NullableString { max_len: 200 },
        default: DefaultValue::Null,
        scope: Scope::User,
        applies: Applies::Live,
    },
    SettingSpec {
        key: "terminal.fontSize",
        section: "terminal",
        label: "Font size",
        description: "Font size in the terminal, in pixels.",
        kind: SettingKind::Integer { min: 8, max: 32 },
        default: DefaultValue::Integer(13),
        scope: Scope::User,
        applies: Applies::Live,
    },
    SettingSpec {
        key: "agents.notifications",
        section: "agents",
        label: "Notify when an agent needs you",
        description: "Raise a system notification when an agent session becomes blocked or starts waiting, while Gritty is in the background.",
        kind: SettingKind::Bool,
        default: DefaultValue::Bool(false),
        scope: Scope::User,
        applies: Applies::Live,
    },
];

/// The spec for `key`, or `None` if this build does not know it.
pub fn spec(key: &str) -> Option<&'static SettingSpec> {
    SETTINGS.iter().find(|spec| spec.key == key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn keys_are_unique() {
        let mut seen = BTreeSet::new();
        for spec in SETTINGS {
            assert!(seen.insert(spec.key), "duplicate key {}", spec.key);
        }
    }

    #[test]
    fn every_setting_names_a_section_that_exists() {
        for spec in SETTINGS {
            assert!(
                SECTIONS.iter().any(|section| section.id == spec.section),
                "{} names section {}, which is not in SECTIONS",
                spec.key,
                spec.section
            );
        }
    }

    #[test]
    fn every_default_is_valid_against_its_own_kind() {
        // The failure this catches is a default nobody could have typed: a
        // registry row whose default is out of its own range, or missing from
        // its own enum. It would only surface as a value the UI refuses to
        // save back.
        for spec in SETTINGS {
            let value = spec.default.to_value();
            assert!(
                crate::document::validate(spec, &value).is_ok(),
                "{} has a default its own kind rejects: {value}",
                spec.key
            );
        }
    }

    #[test]
    fn keys_are_dotted_and_start_with_their_section() {
        // Not cosmetic: the key is what a hand-edited file names, and grouping
        // by prefix is what makes a file readable without the schema.
        for spec in SETTINGS {
            assert!(
                spec.key.starts_with(&format!("{}.", spec.section)),
                "{} does not start with its section {}",
                spec.key,
                spec.section
            );
        }
    }

    #[test]
    fn descriptions_are_present_and_are_sentences() {
        // `settings.describe` is the only documentation a setting gets.
        for spec in SETTINGS {
            assert!(
                !spec.description.is_empty(),
                "{} has no description",
                spec.key
            );
            assert!(
                spec.description.ends_with('.'),
                "{} description is not a sentence",
                spec.key
            );
        }
    }
}
