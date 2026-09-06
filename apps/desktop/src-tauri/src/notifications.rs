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

use tauri::plugin::PermissionState;
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_notification::Notification;

use gritty_core::agent_session::attention::{
    AgentNotificationPermission, AttentionAlert, AttentionNotifier,
};

/// Raises notifications through `tauri-plugin-notification`.
pub struct PluginNotifier<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> PluginNotifier<R> {
    pub fn new(app: AppHandle<R>) -> Self {
        Self { app }
    }

    /// The plugin's state, or `None` when it was never registered — which is
    /// what makes the setting *absent* rather than broken on a platform that
    /// does not carry it.
    fn plugin(&self) -> Option<&Notification<R>> {
        self.app.try_state::<Notification<R>>().map(|s| s.inner())
    }
}

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

/// Tauri's permission state onto the wire's. `PromptWithRationale` is Android
/// only and reads as `PROMPT` here rather than as a fourth value nothing on
/// desktop can produce.
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
