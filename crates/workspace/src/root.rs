//! The workspace root: an absolute directory, canonicalized once at open
//! time, against which every [`WorkspacePath`] is resolved.
//!
//! Resolution is lexical first (the wire path cannot contain `..`, so the
//! joined path is inside the root by construction) and then physical: the
//! joined path is canonicalized and must still start with the root, which
//! rejects symlinks that point outside. Case sensitivity follows the
//! filesystem (see `docs/architecture/security.md`).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::error::WorkspaceError;
use crate::path::WorkspacePath;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRoot {
    path: PathBuf,
}

/// A wire path resolved against a root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPath {
    /// `root` joined with the wire segments; the path to use for listing so
    /// entries are addressed the way the user sees them.
    native: PathBuf,
    /// Symlinks resolved; proven to be inside the root.
    canonical: PathBuf,
}

impl ResolvedPath {
    pub fn native(&self) -> &Path {
        &self.native
    }

    pub fn canonical(&self) -> &Path {
        &self.canonical
    }
}

impl WorkspaceRoot {
    /// Canonicalize `candidate` (symlinks resolved) and require an existing,
    /// readable directory. Relative paths are rejected: what they are relative
    /// to differs between the picker, the shell and tests.
    pub fn open(candidate: &Path) -> Result<Self, WorkspaceError> {
        if candidate.as_os_str().is_empty() {
            return Err(WorkspaceError::invalid_path("path", "must not be empty"));
        }
        if !candidate.is_absolute() {
            return Err(WorkspaceError::invalid_path("path", "must be absolute"));
        }
        let shown = candidate.display().to_string();
        let path =
            canonicalize(candidate).map_err(|error| WorkspaceError::from_io(error, &shown))?;
        let metadata =
            fs::metadata(&path).map_err(|error| WorkspaceError::from_io(error, &shown))?;
        if !metadata.is_dir() {
            return Err(WorkspaceError::NotADirectory(format!(
                "{shown} is not a directory"
            )));
        }
        // Reading the root is the first thing every listing does; refuse now
        // rather than after the workspace has been announced as open.
        fs::read_dir(&path).map_err(|error| WorkspaceError::from_io(error, &shown))?;
        Ok(Self { path })
    }

    /// The canonical root path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Display name: the last component, or the whole path for a drive root.
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.to_string_lossy().into_owned())
    }

    /// Whether the root still exists as a directory (it may be deleted or
    /// unmounted while open).
    pub fn is_present(&self) -> bool {
        fs::metadata(&self.path).is_ok_and(|metadata| metadata.is_dir())
    }

    /// Resolve a wire path: it must exist, and after resolving symlinks it must
    /// still be inside the root. Escapes are `PermissionDenied`, missing paths
    /// `NotFound`.
    pub fn resolve(&self, path: &WorkspacePath) -> Result<ResolvedPath, WorkspaceError> {
        let native = path.to_native(&self.path);
        let canonical = canonicalize(&native)
            .map_err(|error| WorkspaceError::from_io(error, path.to_string()))?;
        if !self.contains(&canonical) {
            return Err(WorkspaceError::PermissionDenied(format!(
                "{path} resolves outside the workspace root"
            )));
        }
        Ok(ResolvedPath { native, canonical })
    }

    /// Resolve a wire path for creating or replacing a file (T030). An existing
    /// target is resolved like [`Self::resolve`] (symlinks followed, must stay
    /// inside the root); a missing target is allowed when its parent resolves
    /// inside the root, and the final name is appended lexically. The root
    /// itself is rejected.
    pub fn resolve_for_write(&self, path: &WorkspacePath) -> Result<ResolvedPath, WorkspaceError> {
        let Some(name) = path.name() else {
            return Err(WorkspaceError::invalid_path("path", "must name a file"));
        };
        match self.resolve(path) {
            Ok(resolved) => Ok(resolved),
            Err(WorkspaceError::NotFound(_)) => {
                let parent = path.parent().expect("a named path has a parent");
                let resolved_parent = self.resolve(&parent)?;
                Ok(ResolvedPath {
                    native: path.to_native(&self.path),
                    canonical: resolved_parent.canonical().join(name),
                })
            }
            Err(error) => Err(error),
        }
    }

    /// Whether an already canonical path lies inside the root (component-wise,
    /// so `/ws2` is not inside `/ws`).
    pub fn contains(&self, canonical: &Path) -> bool {
        canonical.starts_with(&self.path)
    }

    /// The wire path for an already canonical native path inside this root, or
    /// `None` when it lies outside. The inverse of [`WorkspacePath::to_native`],
    /// for the cases where a path arrives from the platform rather than from
    /// the protocol — a file dropped onto the window, for one.
    ///
    /// The root itself maps to [`WorkspacePath::root`]. A component that is not
    /// valid UTF-8 yields `None` rather than a lossy path: the protocol carries
    /// strings, and a lossy name would not round-trip back to this file.
    ///
    /// The check itself lives in [`WorkspacePath::from_native`], which the
    /// watcher (T070) needs with its error rather than as an `Option`; this is
    /// the same answer with the reason dropped.
    pub fn relativize(&self, canonical: &Path) -> Option<WorkspacePath> {
        WorkspacePath::from_native(&self.path, canonical).ok()
    }
}

/// `fs::canonicalize` with the Windows verbatim prefix (`\\?\`) removed so
/// paths compare and display like the ones users type. Every path that is
/// compared against a root goes through this function.
pub fn canonicalize(path: &Path) -> io::Result<PathBuf> {
    let canonical = fs::canonicalize(path)?;
    Ok(strip_verbatim(canonical))
}

#[cfg(windows)]
fn strip_verbatim(path: PathBuf) -> PathBuf {
    use std::path::{Component, Prefix};

    let mut components = path.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return path;
    };
    let head = match prefix.kind() {
        Prefix::VerbatimDisk(disk) => format!("{}:\\", char::from(disk)),
        Prefix::VerbatimUNC(server, share) => format!(
            "\\\\{}\\{}\\",
            server.to_string_lossy(),
            share.to_string_lossy()
        ),
        _ => return path,
    };
    let mut rebuilt = PathBuf::from(head);
    for component in components {
        if !matches!(component, Component::RootDir) {
            rebuilt.push(component.as_os_str());
        }
    }
    rebuilt
}

#[cfg(not(windows))]
fn strip_verbatim(path: PathBuf) -> PathBuf {
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relativize_is_the_inverse_of_to_native() {
        let dir = std::env::temp_dir();
        let root = WorkspaceRoot::open(&dir).expect("temp dir is a root");
        for wire in ["", "src", "src/lib.rs", "a/b/c.txt"] {
            let path = WorkspacePath::parse(wire).expect("wire path");
            let native = path.to_native(root.path());
            assert_eq!(root.relativize(&native), Some(path), "{wire}");
        }
    }

    #[test]
    fn relativize_refuses_paths_outside_the_root() {
        let dir = std::env::temp_dir().join("gritty-relativize");
        std::fs::create_dir_all(dir.join("inside")).expect("fixture");
        let root = WorkspaceRoot::open(&dir).expect("root");
        // A sibling that merely shares a prefix as a string is not inside.
        let sibling = dir.with_file_name("gritty-relativize-other");
        assert_eq!(root.relativize(&sibling), None);
        assert_eq!(root.relativize(Path::new("/")), None);
        // The root itself is the root wire path, not `None`.
        assert_eq!(root.relativize(root.path()), Some(WorkspacePath::root()));
    }

    #[test]
    fn open_rejects_relative_and_empty_paths() {
        assert!(matches!(
            WorkspaceRoot::open(Path::new("")),
            Err(WorkspaceError::InvalidPath { field: "path", .. })
        ));
        assert!(matches!(
            WorkspaceRoot::open(Path::new("relative/dir")),
            Err(WorkspaceError::InvalidPath { field: "path", .. })
        ));
    }

    #[test]
    fn open_reports_missing_roots_and_files() {
        let missing = std::env::temp_dir().join("gritty-root-test-does-not-exist");
        assert!(matches!(
            WorkspaceRoot::open(&missing),
            Err(WorkspaceError::NotFound(_))
        ));
        let file = std::env::temp_dir().join("gritty-root-test-file.txt");
        fs::write(&file, "x").unwrap();
        let result = WorkspaceRoot::open(&file);
        fs::remove_file(&file).unwrap();
        assert!(matches!(result, Err(WorkspaceError::NotADirectory(_))));
    }

    #[test]
    fn open_canonicalizes_and_names_the_root() {
        let temp = std::env::temp_dir();
        let root = WorkspaceRoot::open(&temp).unwrap();
        assert!(root.path().is_absolute());
        assert!(root.is_present());
        assert!(!root.name().is_empty());
        assert!(root.contains(root.path()));
        assert!(!root.contains(Path::new("/definitely/elsewhere")));
        #[cfg(windows)]
        assert!(!root.path().to_string_lossy().starts_with("\\\\?\\"));
    }

    #[cfg(windows)]
    #[test]
    fn verbatim_prefixes_are_stripped() {
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\C:\Users\x")),
            PathBuf::from(r"C:\Users\x")
        );
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\UNC\server\share\dir")),
            PathBuf::from(r"\\server\share\dir")
        );
        assert_eq!(
            strip_verbatim(PathBuf::from(r"C:\plain")),
            PathBuf::from(r"C:\plain")
        );
    }
}
