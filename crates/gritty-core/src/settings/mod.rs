//! The settings service (T120, ADR-031): the `settings.*` handlers, and the
//! bridge between `gritty-settings` and the wire.
//!
//! The domain is deliberately not here. `gritty-settings` owns the registry,
//! the document and the failure policy, and knows nothing about this crate —
//! the same division `gritty-persistence` has, and what lets the rules that
//! matter (a file that fails to parse is never rewritten; a bad value falls
//! back alone; unknown keys survive a write) be tested without an event bus.
//!
//! What this module adds is three things the domain cannot do for itself:
//! turning the registry into a schema the renderer can draw, turning a domain
//! refusal into the error envelope, and emitting `SettingsChanged` when — and
//! only when — the effective values actually moved.

pub mod wire;

use std::collections::BTreeMap;
use std::sync::Arc;

use gritty_settings::{
    Applies, SettingKind, SettingsError, SettingsStore, Snapshot as DomainSnapshot,
};
use serde_json::Value as JsonValue;
use tokio::runtime::Handle;

use crate::event::{CoreEvent, EventBus, EventOrigin};
use crate::protocol::actor::Actor;
use crate::protocol::error::{CoreError, CoreErrorCode};
use crate::protocol::ids::CorrelationId;
use crate::shell::ExternalOpener;

use wire::{
    ResetSettingsRequest, SetSettingsRequest, SettingApplies, SettingChoice, SettingDiagnostic,
    SettingKindWire, SettingScope, SettingSection, SettingSpecWire, SettingsChangedPayload,
    SettingsSchema, SettingsSnapshot, SettingsStorage,
};

/// The `settings.*` command surface.
pub struct SettingsService {
    handle: Handle,
    store: Arc<SettingsStore>,
    events: Arc<EventBus>,
    /// Shows `settings.json` in the file manager. The path handed to it is
    /// always core-derived — see [`SettingsService::reveal`].
    opener: Arc<dyn ExternalOpener>,
}

impl SettingsService {
    pub fn new(
        handle: Handle,
        store: Arc<SettingsStore>,
        events: Arc<EventBus>,
        opener: Arc<dyn ExternalOpener>,
    ) -> Self {
        Self {
            handle,
            store,
            events,
            opener,
        }
    }

    /// `settings.reveal`: show the settings file in the system file manager.
    ///
    /// Its own command rather than `shell.revealPath`, which resolves a
    /// `WorkspacePath` against the workspace root — `settings.json` lives
    /// outside every workspace, so that route refuses by construction. The
    /// path here comes from the store, never from the renderer, which is what
    /// keeps the boundary intact: the WebView names no path and cannot.
    pub async fn reveal(&self) -> Result<(), CoreError> {
        let Some(path) = self.store.path().map(std::path::Path::to_path_buf) else {
            return Err(CoreError::conflict(
                "Gritty has no application data directory, so there is no settings file to show.",
            ));
        };
        let opener = Arc::clone(&self.opener);
        self.blocking(move || opener.reveal(&path)).await?
    }

    /// One setting's effective value, read synchronously.
    ///
    /// For the composition root, which needs an answer before the async
    /// runtime is doing anything else — the store is already loaded by then, so
    /// this is a lock and a map lookup rather than I/O. Everything on the
    /// command path goes through [`Self::get`] and the blocking pool instead.
    pub fn snapshot_value(&self, key: &str) -> Option<serde_json::Value> {
        self.store.snapshot().values.get(key).cloned()
    }

    /// `settings.describe`. A build constant: it does not read the file, so it
    /// cannot fail and does not touch the blocking pool.
    pub fn describe(&self) -> SettingsSchema {
        schema()
    }

    /// `settings.get`.
    pub async fn get(&self) -> Result<SettingsSnapshot, CoreError> {
        let store = Arc::clone(&self.store);
        let snapshot = self.blocking(move || store.snapshot()).await?;
        Ok(to_wire(&snapshot))
    }

    /// `settings.set`: a patch, applied as one atomic file write.
    pub async fn set(
        &self,
        request: SetSettingsRequest,
        correlation: CorrelationId,
    ) -> Result<SettingsSnapshot, CoreError> {
        if request.values.is_empty() {
            return Err(CoreError::invalid_input(
                "values",
                "must name at least one setting",
            ));
        }
        let store = Arc::clone(&self.store);
        let values = request.values;
        let applied = self
            .blocking(move || store.set(&values))
            .await?
            .map_err(to_core_error)?;
        Ok(self.announce(applied, Actor::User, Some(correlation)))
    }

    /// `settings.reset`: remove keys so their defaults apply again.
    pub async fn reset(
        &self,
        request: ResetSettingsRequest,
        correlation: CorrelationId,
    ) -> Result<SettingsSnapshot, CoreError> {
        let store = Arc::clone(&self.store);
        let keys = request.keys;
        let applied = self
            .blocking(move || store.reset(keys.as_deref()))
            .await?
            .map_err(to_core_error)?;
        Ok(self.announce(applied, Actor::User, Some(correlation)))
    }

    /// Re-read the file, and announce it if anything moved.
    ///
    /// The focus-gain path, not a watcher: the application data directory holds
    /// `gritty.sqlite-wal`, which the snapshot writer touches every one to two
    /// seconds while any buffer is dirty, so watching that directory would
    /// reproduce exactly the noise ADR-025 exists to filter — plus an echo loop
    /// on Gritty's own writes (ADR-031 §6). The store's `(size, mtime)` guard
    /// makes the common case one `stat`, so this is cheap enough to run on
    /// every focus.
    pub async fn reload_on_focus(&self) -> Option<SettingsSnapshot> {
        let store = Arc::clone(&self.store);
        let applied = self
            .blocking(move || store.reload_if_changed())
            .await
            .ok()??;
        Some(self.announce(applied, Actor::System, None))
    }

    /// Emit `SettingsChanged` if anything actually moved, and return the
    /// snapshot either way.
    ///
    /// The "if" is load-bearing twice over: it keeps a no-op write off the bus,
    /// and it is what stops an applier that reads the event and writes a
    /// setting from looping.
    fn announce(
        &self,
        applied: gritty_settings::Applied,
        actor: Actor,
        correlation: Option<CorrelationId>,
    ) -> SettingsSnapshot {
        let snapshot = to_wire(&applied.snapshot);
        if !applied.changed.is_empty() {
            self.events.emit(
                &EventOrigin {
                    workspace_id: None,
                    correlation_id: correlation,
                    actor,
                },
                CoreEvent::SettingsChanged(SettingsChangedPayload {
                    snapshot: snapshot.clone(),
                    changed: applied.changed,
                }),
            );
        }
        snapshot
    }

    /// Every file touch runs on the blocking pool. A settings read is a `stat`
    /// and a small parse, but it is still I/O, and hard rule 10 does not have
    /// an exception for small I/O.
    async fn blocking<T, F>(&self, work: F) -> Result<T, CoreError>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        self.handle
            .spawn_blocking(work)
            .await
            .map_err(|error| CoreError::internal(format!("settings task failed: {error}")))
    }
}

/// The registry, as the renderer sees it.
fn schema() -> SettingsSchema {
    SettingsSchema {
        sections: gritty_settings::SECTIONS
            .iter()
            .map(|section| SettingSection {
                id: section.id.to_owned(),
                label: section.label.to_owned(),
            })
            .collect(),
        settings: gritty_settings::SETTINGS
            .iter()
            .map(|spec| SettingSpecWire {
                key: spec.key.to_owned(),
                section: spec.section.to_owned(),
                label: spec.label.to_owned(),
                description: spec.description.to_owned(),
                kind: to_wire_kind(&spec.kind),
                default: spec.default.to_value(),
                scope: SettingScope::User,
                applies: match spec.applies {
                    Applies::Live => SettingApplies::Live,
                    Applies::NewTerminals => SettingApplies::NewTerminals,
                },
            })
            .collect(),
    }
}

fn to_wire_kind(kind: &SettingKind) -> SettingKindWire {
    match *kind {
        SettingKind::Bool => SettingKindWire::Bool,
        SettingKind::Integer { min, max } => SettingKindWire::Integer { min, max },
        SettingKind::Enum { choices } => SettingKindWire::Enum {
            choices: choices
                .iter()
                .map(|choice| SettingChoice {
                    value: choice.value.to_owned(),
                    label: choice.label.to_owned(),
                })
                .collect(),
        },
        SettingKind::NullableString { max_len } => SettingKindWire::NullableString {
            max_len: max_len as u32,
        },
        SettingKind::NullablePath => SettingKindWire::NullablePath,
        SettingKind::PathList { max_len } => SettingKindWire::PathList {
            max_len: max_len as u32,
        },
    }
}

fn to_wire(snapshot: &DomainSnapshot) -> SettingsSnapshot {
    SettingsSnapshot {
        values: snapshot
            .values
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<BTreeMap<String, JsonValue>>(),
        explicit: snapshot.explicit.clone(),
        modified: snapshot.modified.clone(),
        invalid: snapshot.invalid.clone(),
        unknown: snapshot.unknown.clone(),
        diagnostics: snapshot
            .diagnostics
            .iter()
            .map(|diagnostic| SettingDiagnostic {
                key: diagnostic.key.clone(),
                message: diagnostic.message.clone(),
                line: diagnostic.line.map(|line| line as u32),
                column: diagnostic.column.map(|column| column as u32),
            })
            .collect(),
        storage: storage_of(snapshot),
    }
}

/// Whether settings can be saved, and — when they cannot — what the user
/// should do about it.
///
/// The `reason` is the part worth care. Both cases that produce one are
/// actionable, and a message that only says what is wrong leaves the user
/// nowhere: "settings.json could not be parsed" is a dead end, while naming the
/// file and the two ways out is not.
fn storage_of(snapshot: &DomainSnapshot) -> SettingsStorage {
    let path = snapshot.path.clone();
    let writable = snapshot.file_state.is_writable() && path.is_some();
    let reason = if writable {
        None
    } else if let Some(path) = path.as_deref() {
        Some(format!(
            "{path} could not be read, so Gritty is showing defaults and will not overwrite it. \
             Fix the file, or delete it to start from defaults."
        ))
    } else {
        Some(
            "Gritty has no application data directory, so settings apply to this session only \
             and cannot be saved."
                .to_owned(),
        )
    };
    SettingsStorage {
        writable,
        path,
        reason,
    }
}

/// A domain refusal, in the envelope's terms.
fn to_core_error(error: SettingsError) -> CoreError {
    match error {
        // A client asking for a key this build does not declare is a bug, not a
        // downgrade — unlike the same key found *in the file*, which is
        // preserved. Naming it is what makes that distinction debuggable.
        SettingsError::UnknownKey(key) => {
            CoreError::invalid_input("values", format!("unknown setting `{key}`"))
        }
        SettingsError::InvalidValue {
            ref key,
            ref expected,
        } => CoreError::invalid_input("values", format!("`{key}` expects {expected}")),
        // Not `Io`: the file is readable and the disk is fine. The state of the
        // world conflicts with the write, and the user resolves it by editing
        // the file — which is exactly what `Conflict` means here.
        SettingsError::Unparseable(ref path) => CoreError::conflict(format!(
            "{} could not be parsed, so Gritty will not overwrite it. \
             Fix the file, or delete it to start from defaults.",
            path.display()
        )),
        SettingsError::Unavailable => CoreError::conflict(
            "Gritty has no application data directory, so settings cannot be saved.",
        ),
        SettingsError::Write { .. } | SettingsError::Io { .. } => {
            CoreError::new(CoreErrorCode::Io, error.to_string())
        }
    }
}
