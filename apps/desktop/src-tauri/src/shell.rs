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
}
