//! Wire types of the `settings.*` commands (T120-C, ADR-031).
//!
//! The domain — the registry, the document, the failure policy — lives in
//! `gritty-settings`, the way `RepositoryStatus` lives in `gritty-git`. What
//! this module adds is the shape the UI sees.
//!
//! Two things about that shape are load-bearing rather than incidental.
//!
//! **The schema crosses the wire, and the form is generated from it.** A
//! setting is declared once, in one Rust table, which both refuses a bad value
//! here and tells the renderer which control to draw (ADR-031 §3). The
//! alternative — a hand-written form beside the registry — means maintaining
//! validation, search and reset-to-default twice, and the two drift. It also
//! means the four bounds the UI currently hardcodes (`MAX_CUSTOM_SHELLS`,
//! the font-size ranges, the tab sizes) stop being duplicates of a Rust
//! constant and start being reads of it.
//!
//! **A snapshot separates four kinds of "this key is interesting".** `explicit`
//! is what the file sets and this build accepted; `modified` is what differs
//! from its default; `invalid` is what the file sets and this build *rejected*;
//! `unknown` is what this build does not declare and preserves anyway. They are
//! not the same list, and collapsing them would make the page lie: an invalid
//! value is not in force, so drawing it as "you changed this" tells the user
//! their setting is applied when it is not.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use ts_rs::TS;

/// `settings.describe`: every setting this build declares, and how to draw it.
///
/// A build constant — it never varies by workspace or by what the file holds,
/// so the renderer fetches it once.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSchema {
    /// The groups the page renders, in order.
    pub sections: Vec<SettingSection>,
    /// Every setting, in registry order. Each names its section.
    pub settings: Vec<SettingSpecWire>,
}

/// One group of settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SettingSection {
    pub id: String,
    pub label: String,
}

/// One setting, described well enough to render a control without a second
/// lookup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SettingSpecWire {
    /// The dotted key. Stable forever — it is what a hand-edited file names.
    pub key: String,
    /// Which [`SettingSection`] this belongs to.
    pub section: String,
    pub label: String,
    pub description: String,
    /// The control to draw, and the bounds that make a value valid.
    pub kind: SettingKindWire,
    /// The value in force when the file says nothing.
    pub default: JsonValue,
    pub scope: SettingScope,
    pub applies: SettingApplies,
}

/// Where a setting may be written.
///
/// One variant today. It rides on the wire from the first version so that
/// workspace scope is an added variant rather than a changed shape — but it is
/// deliberately not honoured yet: a repo-local `.gritty/` directory is
/// forbidden until its ownership and Git behaviour are defined, and a
/// workspace-scoped setting that can name an executable is arbitrary code
/// execution on clone (ADR-031 §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SettingScope {
    /// Global to the user, in `settings.json`.
    User,
}

/// When a change takes effect.
///
/// Rendered as a hint beside the control. Saying "this reaches the next
/// terminal" is cheaper and more honest than pretending every setting is live,
/// and it is the difference between a user thinking the setting is broken and
/// knowing what to do about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SettingApplies {
    /// Immediately, on the surfaces already open.
    Live,
    /// To terminals started after the change; running ones keep their shell.
    NewTerminals,
}

/// The control a setting needs, and the bounds the core will enforce.
///
/// The bounds ride with the kind rather than being agreed separately, because
/// a hand-edited file never passes through a form: the core has to be able to
/// refuse a value the UI never saw, and the UI has to be able to stop the user
/// producing one the core would refuse. One declaration serves both.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SettingKindWire {
    Bool,
    /// A whole number in an inclusive range.
    Integer {
        min: i64,
        max: i64,
    },
    /// One of a fixed set of strings.
    Enum {
        choices: Vec<SettingChoice>,
    },
    /// Free text, or `null` for "unset, use the built-in".
    NullableString {
        max_len: u32,
    },
    /// An absolute path, or `null` for "the system default".
    NullablePath,
    /// A bounded, deduplicated list of absolute paths.
    PathList {
        max_len: u32,
    },
}

/// One option of a [`SettingKindWire::Enum`], with the label the UI shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SettingChoice {
    pub value: String,
    pub label: String,
}

/// `settings.get`, and the answer to every write: what is in force, and how
/// far it can be trusted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSnapshot {
    /// Every setting's effective value: the file overlaid on the defaults.
    pub values: BTreeMap<String, JsonValue>,
    /// Keys the file sets and this build accepted.
    pub explicit: Vec<String>,
    /// Keys whose effective value differs from the default — what the page
    /// marks as modified, and what "reset all" would change.
    pub modified: Vec<String>,
    /// Keys the file sets and this build **rejected**. Their default is in
    /// force instead. Not a modification: drawing one as changed would tell
    /// the user a value is applied when it is not.
    pub invalid: Vec<String>,
    /// Keys this build does not declare, preserved verbatim across a write so
    /// a downgrade never silently deletes a newer build's setting.
    /// Information, not an error.
    pub unknown: Vec<String>,
    /// What the user should know about their file, worst first.
    pub diagnostics: Vec<SettingDiagnostic>,
    /// Whether settings can be saved at all, and why not when they cannot.
    pub storage: SettingsStorage,
}

/// Whether settings can be written, and where they live.
///
/// The same shape `GitWriteAvailability` uses, for the same reason: a
/// capability the UI must render as **disabled with the reason** rather than
/// absent. A settings page that silently shows defaults it cannot save is the
/// worst version of that screen — the user changes something, it appears to
/// take, and it is gone next launch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SettingsStorage {
    /// Whether a write would be accepted right now.
    pub writable: bool,
    /// Absolute path of `settings.json`, for display and for the user to go
    /// and edit. Absent only when there is no application data directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub path: Option<String>,
    /// Why not, in words meant for the user, present exactly when `writable`
    /// is false. It must say **what to do**, not only what is wrong: the two
    /// cases that produce it — an unparseable file, and no data directory —
    /// are ones a user can act on, and "settings.json could not be parsed" on
    /// its own leaves them nowhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reason: Option<String>,
}

/// Something the user should know about their settings file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SettingDiagnostic {
    /// The setting this concerns, or absent for the file as a whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub key: Option<String>,
    pub message: String,
    /// 1-based position of a parse failure, when there is one. Carried so the
    /// user can go straight to the line rather than hunting for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub column: Option<u32>,
}

/// `settings.set`: a patch, never a whole document.
///
/// A whole-document write from a renderer holding a snapshot taken before an
/// external edit would silently revert that edit. A patch also says exactly
/// what the user touched, which is what makes preserving unknown keys
/// obviously correct rather than merely intended.
///
/// Several keys in one request commit as **one** atomic file write, so a page
/// resetting a section does not produce five writes and five events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SetSettingsRequest {
    pub values: BTreeMap<String, JsonValue>,
}

/// `settings.reset`: remove keys from the file so their defaults apply again.
///
/// Removing rather than writing the default value is deliberate: a default
/// that changes in a later build then actually reaches the user, instead of
/// being frozen into their file the first time they clicked Reset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResetSettingsRequest {
    /// The keys to reset. Absent resets every **known** key; keys this build
    /// does not declare are never ours to reset, so they survive either way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub keys: Option<Vec<String>>,
}

/// `SettingsChanged`: what is in force now, and which keys moved.
///
/// It carries the **whole** snapshot, not a delta, and that is not a hard rule
/// 8 violation — that rule is about volume and frequency. This fires at human
/// frequency with a payload bounded by a static registry, and every other
/// converging event in this codebase carries its whole state for the same
/// stated reason: a consumer that missed one event still converges, where a
/// delta-only stream would leave it permanently wrong.
///
/// `changed` rides alongside so a consumer can ignore an event that does not
/// concern it — the theme applier only cares about one key. The event is
/// emitted **only when the effective values actually differ**, which is also
/// what makes an applier that reads it unable to loop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SettingsChangedPayload {
    pub snapshot: SettingsSnapshot,
    /// The keys whose effective value moved. Never empty: an event with
    /// nothing in it is not emitted.
    pub changed: Vec<String>,
}
