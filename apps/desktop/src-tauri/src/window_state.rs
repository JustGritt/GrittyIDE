//! Window geometry persistence (T060): capture `Resized` / `Moved` into the
//! session service and restore the main window before it is shown.
//!
//! Positions and sizes are physical pixels straight from the events. A
//! restored position is applied only when the window would still be visible
//! on a connected monitor; otherwise the size alone is restored and the
//! platform places the window. Verified on macOS; Wayland ignores
//! `set_position` and GTK's maximize-before-show is unreliable, so Linux is
//! recorded as untested in the platform matrix.

use std::sync::Mutex;

use gritty_core::session::WindowGeometry;
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, Runtime, Window, WindowEvent};

use crate::core::Core;

pub const MAIN_WINDOW: &str = "main";
/// Smallest size ever restored, in physical pixels.
const MIN_SIZE: u32 = 200;
/// How much of the window must lie on a monitor for its position to be reused.
const VISIBLE_MARGIN: i32 = 64;

/// Remembers the last geometry seen while *not* maximized so that a
/// maximized window persists its flag without overwriting the size and
/// position it will return to.
#[derive(Default)]
pub struct WindowTracker {
    last_normal: Mutex<Option<WindowGeometry>>,
}

impl WindowTracker {
    fn remember(&self, geometry: &WindowGeometry) {
        *self.last_normal.lock().unwrap_or_else(|e| e.into_inner()) = Some(geometry.clone());
    }

    fn last_normal(&self) -> Option<WindowGeometry> {
        self.last_normal
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

/// A monitor's area in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonitorRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// What to apply from a stored geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fit {
    pub width: u32,
    pub height: u32,
    /// `None`: leave placement to the platform.
    pub position: Option<(i32, i32)>,
}

/// Clamp the size and keep the position only if at least a `VISIBLE_MARGIN`
/// square of the window intersects some monitor. No monitors (headless, the
/// mock runtime) means size only.
pub fn fit_to_monitors(geometry: &WindowGeometry, monitors: &[MonitorRect]) -> Fit {
    let width = geometry.width.max(MIN_SIZE);
    let height = geometry.height.max(MIN_SIZE);
    let position = match (geometry.x, geometry.y) {
        (Some(x), Some(y)) => {
            let right = x.saturating_add(i32::try_from(width).unwrap_or(i32::MAX));
            let bottom = y.saturating_add(i32::try_from(height).unwrap_or(i32::MAX));
            let visible = monitors.iter().any(|monitor| {
                let monitor_right = monitor
                    .x
                    .saturating_add(i32::try_from(monitor.width).unwrap_or(i32::MAX));
                let monitor_bottom = monitor
                    .y
                    .saturating_add(i32::try_from(monitor.height).unwrap_or(i32::MAX));
                let overlap_w = right.min(monitor_right) - x.max(monitor.x);
                let overlap_h = bottom.min(monitor_bottom) - y.max(monitor.y);
                overlap_w >= VISIBLE_MARGIN && overlap_h >= VISIBLE_MARGIN
            });
            visible.then_some((x, y))
        }
        _ => None,
    };
    Fit {
        width,
        height,
        position,
    }
}

/// Registered with `Builder::on_window_event`: geometry changes go to the
/// session writer (debounced there). Nothing happens before the core is
/// attached (the mock runtime never attaches it through `setup`).
pub fn on_window_event<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
    match event {
        WindowEvent::Resized(_) | WindowEvent::Moved(_) => {}
        _ => return,
    }
    let app = window.app_handle();
    let (Some(core), Some(tracker)) = (app.try_state::<Core>(), app.try_state::<WindowTracker>())
    else {
        return;
    };
    let Some(current) = capture(window) else {
        return;
    };
    let geometry = if current.maximized {
        // Keep the geometry the window returns to; only the flag changes.
        match tracker.last_normal() {
            Some(normal) => WindowGeometry {
                maximized: true,
                ..normal
            },
            None => current,
        }
    } else {
        tracker.remember(&current);
        current
    };
    core.session.record_window(geometry);
}

fn capture<R: Runtime>(window: &Window<R>) -> Option<WindowGeometry> {
    let size = window.inner_size().ok()?;
    let position = window.outer_position().ok()?;
    Some(WindowGeometry {
        label: window.label().to_owned(),
        x: Some(position.x),
        y: Some(position.y),
        width: size.width,
        height: size.height,
        scale_factor: window.scale_factor().unwrap_or(1.0),
        maximized: window.is_maximized().unwrap_or(false),
    })
}

/// Apply the persisted geometry of the main window and show it. Always shows
/// the window, whatever happened before, so the app never starts invisible.
pub fn restore_main_window<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        tracing::warn!(
            label = MAIN_WINDOW,
            "main window not found; nothing to restore"
        );
        return;
    };
    if let Some(core) = app.try_state::<Core>()
        && let Some(stored) = core.session.window_state(MAIN_WINDOW)
    {
        let monitors: Vec<MonitorRect> = window
            .available_monitors()
            .unwrap_or_default()
            .iter()
            .map(|monitor| MonitorRect {
                x: monitor.position().x,
                y: monitor.position().y,
                width: monitor.size().width,
                height: monitor.size().height,
            })
            .collect();
        let fit = fit_to_monitors(&stored, &monitors);
        tracing::info!(
            width = fit.width,
            height = fit.height,
            position = ?fit.position,
            maximized = stored.maximized,
            monitors = monitors.len(),
            "restoring window geometry"
        );
        if let Err(error) = window.set_size(PhysicalSize::new(fit.width, fit.height)) {
            tracing::warn!(%error, "could not restore the window size");
        }
        if let Some((x, y)) = fit.position
            && let Err(error) = window.set_position(PhysicalPosition::new(x, y))
        {
            tracing::warn!(%error, "could not restore the window position");
        }
        if stored.maximized
            && let Err(error) = window.maximize()
        {
            tracing::warn!(%error, "could not maximize the window");
        }
        if let Some(tracker) = app.try_state::<WindowTracker>() {
            tracker.remember(&WindowGeometry {
                x: fit.position.map(|p| p.0),
                y: fit.position.map(|p| p.1),
                width: fit.width,
                height: fit.height,
                maximized: false,
                ..stored
            });
        }
    }
    if let Err(error) = window.show() {
        tracing::error!(%error, "could not show the main window");
    } else {
        crate::log_startup_milestone("window shown");
    }
    // Record where the window ended up, so a quit without any resize or move
    // still restores this geometry next time.
    if let (Some(core), Some(tracker)) = (app.try_state::<Core>(), app.try_state::<WindowTracker>())
        && let (Ok(size), Ok(position)) = (window.inner_size(), window.outer_position())
    {
        let geometry = WindowGeometry {
            label: window.label().to_owned(),
            x: Some(position.x),
            y: Some(position.y),
            width: size.width,
            height: size.height,
            scale_factor: window.scale_factor().unwrap_or(1.0),
            maximized: window.is_maximized().unwrap_or(false),
        };
        if !geometry.maximized {
            tracker.remember(&geometry);
        }
        core.session.record_window(geometry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry(x: Option<i32>, y: Option<i32>, width: u32, height: u32) -> WindowGeometry {
        WindowGeometry {
            label: MAIN_WINDOW.into(),
            x,
            y,
            width,
            height,
            scale_factor: 2.0,
            maximized: false,
        }
    }

    const PRIMARY: MonitorRect = MonitorRect {
        x: 0,
        y: 0,
        width: 2560,
        height: 1440,
    };
    const LEFT: MonitorRect = MonitorRect {
        x: -1920,
        y: 0,
        width: 1920,
        height: 1080,
    };

    #[test]
    fn a_position_on_a_connected_monitor_is_kept() {
        let fit = fit_to_monitors(&geometry(Some(100), Some(80), 1280, 800), &[PRIMARY]);
        assert_eq!(
            fit,
            Fit {
                width: 1280,
                height: 800,
                position: Some((100, 80))
            }
        );
        let on_left = fit_to_monitors(
            &geometry(Some(-1500), Some(10), 1000, 700),
            &[PRIMARY, LEFT],
        );
        assert_eq!(on_left.position, Some((-1500, 10)));
    }

    #[test]
    fn a_position_off_every_monitor_is_dropped_and_the_size_kept() {
        let gone = fit_to_monitors(&geometry(Some(-1500), Some(10), 1000, 700), &[PRIMARY]);
        assert_eq!(gone.position, None);
        assert_eq!((gone.width, gone.height), (1000, 700));
        let barely = fit_to_monitors(&geometry(Some(2560 - 63), Some(0), 800, 600), &[PRIMARY]);
        assert_eq!(
            barely.position, None,
            "less than the visible margin overlaps"
        );
        let enough = fit_to_monitors(&geometry(Some(2560 - 64), Some(0), 800, 600), &[PRIMARY]);
        assert_eq!(enough.position, Some((2560 - 64, 0)));
    }

    #[test]
    fn no_monitors_or_no_position_means_size_only_with_a_floor() {
        let headless = fit_to_monitors(&geometry(Some(10), Some(10), 50, 20), &[]);
        assert_eq!(
            headless,
            Fit {
                width: MIN_SIZE,
                height: MIN_SIZE,
                position: None
            }
        );
        let unknown = fit_to_monitors(&geometry(None, None, 1280, 800), &[PRIMARY]);
        assert_eq!(unknown.position, None);
        assert_eq!((unknown.width, unknown.height), (1280, 800));
    }

    #[test]
    fn extreme_coordinates_do_not_overflow() {
        let fit = fit_to_monitors(
            &geometry(Some(i32::MAX - 10), Some(i32::MIN), u32::MAX, u32::MAX),
            &[PRIMARY],
        );
        assert_eq!(fit.position, None);
    }
}
