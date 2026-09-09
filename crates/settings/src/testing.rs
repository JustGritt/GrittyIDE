//! A temporary directory that is removed on drop. Test support only.
//!
//! The same shape as `gritty-persistence`'s, and hand-rolled for the same
//! reason: it is twenty lines, and it keeps `tempfile` out of the dependency
//! list for something no production code needs.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

pub struct TempDir(PathBuf);

impl TempDir {
    /// A fresh, empty directory under the system temp dir.
    pub fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "gritty-settings-{label}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn join(&self, name: impl AsRef<Path>) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
