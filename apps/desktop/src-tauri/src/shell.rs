//! The desktop implementation of the core's [`ExternalOpener`]: hand a
//! validated http(s) URL to the operating system's default handler. Rust side
//! only; the WebView has no shell or opener capability.

use gritty_core::CoreError;
use gritty_core::shell::ExternalOpener;

pub struct SystemOpener;

impl ExternalOpener for SystemOpener {
    fn open(&self, url: &str) -> Result<(), CoreError> {
        // Detached: returns once the handler was spawned, never waits for the browser.
        open::that_detached(url).map_err(CoreError::from)
    }

    /// Show an entry in the file manager, selected rather than opened.
    ///
    /// The one place in the desktop shell that branches on the platform, and it
    /// branches because "reveal" has no portable spelling: macOS and Windows
    /// each have a flag for it, and the free desktops have a D-Bus interface
    /// not every file manager implements. Only the macOS arm has been run
    /// (ADR-002, hard rule 11); the others compile and are not claimed.
    fn reveal(&self, path: &std::path::Path) -> Result<(), CoreError> {
        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("/usr/bin/open")
                .arg("-R")
                .arg(path)
                .spawn()
                .map(drop)
                .map_err(CoreError::from)
        }
        #[cfg(target_os = "windows")]
        {
            std::process::Command::new("explorer")
                .arg(format!("/select,{}", path.display()))
                .spawn()
                .map(drop)
                .map_err(CoreError::from)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            // No portable "select this entry", so open the containing folder —
            // the honest subset, rather than a selection that may not happen.
            let folder = path.parent().unwrap_or(path);
            open::that_detached(folder).map_err(CoreError::from)
        }
    }
}
