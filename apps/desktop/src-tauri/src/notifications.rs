//! The platform end of attention notifications (T223).
//!
//! The core decides *whether* to notify — the edge-triggering, the throttle,
//! the cap and the focus suppression all live in
//! `gritty_core::agent_session::attention`, with no Tauri in them. This is the
//! adapter that hands the result to the operating system, and it is the only
//! file in the tree that knows a notification plugin exists.
//!
//! It is raised from **Rust**, from the same place that already observes the
//! roster diff, so the renderer neither decides nor sends one and the WebView
//! gains no new capability: `capabilities/default.json` grants the plugin's
//! `notify`, `requestPermission` and `isPermissionGranted` commands to nobody.
//!
//! Two things the desktop plugin does not give us, recorded here rather than
//! implied away (rule 12):
//!
//! - **Permission is not really ours to ask.** On desktop the plugin's
//!   `request_permission` and `permission_state` both return `Granted`
//!   unconditionally; the actual prompt is the operating system's, raised the
//!   first time a notification is shown. So enabling the setting is still the
//!   only thing that can trigger that prompt — which is what T223 §2 asks for —
//!   but Gritty cannot report a refusal on macOS, and this reports what the
//!   plugin says rather than inventing an answer.
//! - **No click handling.** The desktop implementation spawns
//!   `notify_rust::Notification::show()` and drops the handle, so there is no
//!   callback to focus the window from. T223 §6 is therefore not implemented,
//!   and no code here pretends otherwise.
//!
//! **macOS only.** The plugin is a macOS dependency and is registered on macOS
//! alone. Nobody has run a notification on any other platform, so carrying the
//! dependency everywhere would claim support that was never verified (rule 11),
//! and on Linux it pulls `notify-rust`, `zbus` and `dbus` into every test
//! binary — enough to make `rust-lld` die linking them. Elsewhere the notifier
//! answers `supported() == false`, which is exactly what T223's failure table
//! asks for: "the setting is absent rather than broken". Bringing notifications
//! to another platform is a task, not a build flag.

#[cfg(target_os = "macos")]
use tauri::plugin::PermissionState;
use tauri::{AppHandle, Runtime};

use gritty_core::agent_session::attention::{
    AgentNotificationPermission, AttentionAlert, AttentionNotifier,
};

/// Registers the notification plugin where there is one. Separate from
/// `configure` so the `cfg` lives beside the adapter it belongs to rather than
/// in the middle of the builder chain.
#[cfg(target_os = "macos")]
pub fn register<R: Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder.plugin(tauri_plugin_notification::init())
}

/// Registers nothing. The notifier reports itself unsupported and the setting
/// is absent.
#[cfg(not(target_os = "macos"))]
pub fn register<R: Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder
}

/// Raises notifications through `tauri-plugin-notification`.
///
/// The field is the app handle on macOS and nothing anywhere else, because
/// there is no plugin to look up: keeping an unread handle would be a field
/// that exists only to be ignored.
pub struct PluginNotifier<R: Runtime> {
    #[cfg(target_os = "macos")]
    app: AppHandle<R>,
    #[cfg(not(target_os = "macos"))]
    _runtime: std::marker::PhantomData<R>,
}

impl<R: Runtime> PluginNotifier<R> {
    #[cfg(target_os = "macos")]
    pub fn new(app: AppHandle<R>) -> Self {
        Self { app }
    }

    #[cfg(not(target_os = "macos"))]
    pub fn new(_app: AppHandle<R>) -> Self {
        Self {
            _runtime: std::marker::PhantomData,
        }
    }

    /// The plugin's state, or `None` when it was never registered — which is
    /// what makes the setting *absent* rather than broken on a platform that
    /// does not carry it.
    #[cfg(target_os = "macos")]
    fn plugin(&self) -> Option<&tauri_plugin_notification::Notification<R>> {
        use tauri::Manager;
        self.app
            .try_state::<tauri_plugin_notification::Notification<R>>()
            .map(|s| s.inner())
    }
}

#[cfg(target_os = "macos")]
impl<R: Runtime> AttentionNotifier for PluginNotifier<R> {
    fn supported(&self) -> bool {
        self.plugin().is_some()
    }

    fn permission(&self) -> AgentNotificationPermission {
        self.plugin()
            .and_then(|plugin| plugin.permission_state().ok())
            .map_or(AgentNotificationPermission::Denied, to_domain)
    }

    fn request_permission(&self) -> AgentNotificationPermission {
        let Some(plugin) = self.plugin() else {
            return AgentNotificationPermission::Denied;
        };
        match plugin.request_permission() {
            Ok(state) => to_domain(state),
            Err(error) => {
                tracing::warn!(%error, "notification permission could not be requested");
                AgentNotificationPermission::Denied
            }
        }
    }

    fn show(&self, alert: &AttentionAlert) {
        let Some(plugin) = self.plugin() else {
            return;
        };
        // Fire and forget by design: a notification that failed to appear must
        // not fail a roster scan, and the core has already recorded that this
        // session was notified about — retrying would be the loop the failure
        // table forbids.
        if let Err(error) = plugin
            .builder()
            .title(alert.title.clone())
            .body(alert.body.clone())
            .show()
        {
            // The session id only. A title, a prompt or a working directory
            // never reaches the log (rule 13).
            tracing::warn!(session = %alert.session_id, %error, "notification was not shown");
        }
    }
}

/// Everywhere else: unsupported, permanently and honestly. The core reads
/// `supported()` first and never offers the setting, so `show` is unreachable
/// rather than silently swallowing an alert.
#[cfg(not(target_os = "macos"))]
impl<R: Runtime> AttentionNotifier for PluginNotifier<R> {
    fn supported(&self) -> bool {
        false
    }

    fn permission(&self) -> AgentNotificationPermission {
        AgentNotificationPermission::Denied
    }

    fn request_permission(&self) -> AgentNotificationPermission {
        AgentNotificationPermission::Denied
    }

    fn show(&self, _alert: &AttentionAlert) {}
}

/// Tauri's permission state onto the wire's. `PromptWithRationale` is Android
/// only and reads as `PROMPT` here rather than as a fourth value nothing on
/// desktop can produce.
#[cfg(target_os = "macos")]
fn to_domain(state: PermissionState) -> AgentNotificationPermission {
    match state {
        PermissionState::Granted => AgentNotificationPermission::Granted,
        PermissionState::Denied => AgentNotificationPermission::Denied,
        _ => AgentNotificationPermission::Prompt,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn permission_states_map_onto_the_wire_without_inventing_a_fourth() {
        assert_eq!(
            to_domain(PermissionState::Granted),
            AgentNotificationPermission::Granted
        );
        assert_eq!(
            to_domain(PermissionState::Denied),
            AgentNotificationPermission::Denied
        );
        assert_eq!(
            to_domain(PermissionState::Prompt),
            AgentNotificationPermission::Prompt
        );
        assert_eq!(
            to_domain(PermissionState::PromptWithRationale),
            AgentNotificationPermission::Prompt
        );
    }

    /// A build whose plugin was never registered answers "not supported" and
    /// raises nothing, instead of panicking inside `NotificationExt::state`.
    /// Runs on every platform: on macOS it proves the absent-plugin branch, and
    /// elsewhere it is the only branch there is.
    #[test]
    fn without_the_plugin_nothing_is_supported_and_nothing_is_shown() {
        let app = tauri::test::mock_app();
        let notifier = PluginNotifier::new(app.handle().clone());
        assert!(!notifier.supported());
        assert_eq!(notifier.permission(), AgentNotificationPermission::Denied);
        assert_eq!(
            notifier.request_permission(),
            AgentNotificationPermission::Denied
        );
        notifier.show(&AttentionAlert {
            session_id: "s-1".to_owned(),
            title: "t".to_owned(),
            body: "b".to_owned(),
        });
    }
}
