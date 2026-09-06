//! The wire representation of a path inside a workspace (ADR-010).
//!
//! A [`WorkspacePath`] is relative to the workspace root, uses `/` as the only
//! separator, has no leading or trailing slash, and contains no `.` or `..`
//! segments. The empty string is the root. Absolute paths (including Windows
//! drive and UNC forms) and backslashes are rejected on every platform so the
//! same string means the same thing everywhere. Names are passed through as
//! the filesystem reports them: no Unicode normalization, no case folding.

use std::fmt;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use ts_rs::TS;

use crate::error::WorkspaceError;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, TS)]
pub struct WorkspacePath(#[ts(type = "string")] String);

impl WorkspacePath {
    /// The workspace root itself.
    pub fn root() -> Self {
        Self(String::new())
    }

    /// Validate a wire string.
    pub fn parse(raw: &str) -> Result<Self, WorkspaceError> {
        Self::parse_field(raw, "path")
    }

    /// Validate a wire string; `field` names the argument in the error.
    pub fn parse_field(raw: &str, field: &'static str) -> Result<Self, WorkspaceError> {
        if raw.is_empty() {
            return Ok(Self::root());
        }
        if raw.contains('\u{0}') {
            return Err(WorkspaceError::invalid_path(field, "must not contain NUL"));
        }
        if raw.contains('\\') {
            return Err(WorkspaceError::invalid_path(
                field,
                "must use '/' as the separator (backslashes are not allowed)",
            ));
        }
        if raw.starts_with('/') || looks_absolute(raw) {
            return Err(WorkspaceError::invalid_path(
                field,
                "must be relative to the workspace root",
            ));
        }
        for segment in raw.split('/') {
            match segment {
                "" => {
                    return Err(WorkspaceError::invalid_path(
                        field,
                        "must not contain empty segments or end with '/'",
                    ));
                }
                "." | ".." => {
                    return Err(WorkspaceError::invalid_path(
                        field,
                        "must not contain '.' or '..' segments",
                    ));
                }
                _ => {}
            }
        }
        Ok(Self(raw.to_owned()))
    }

    /// Append one name segment (validated like any other segment).
    pub fn join(&self, name: &str) -> Result<Self, WorkspaceError> {
        let child = Self::parse_field(name, "name")?;
        if child.is_root() {
            return Err(WorkspaceError::invalid_path("name", "must not be empty"));
        }
        if child.0.contains('/') {
            return Err(WorkspaceError::invalid_path(
                "name",
                "must be a single segment",
            ));
        }
        Ok(if self.is_root() {
            child
        } else {
            Self(format!("{}/{}", self.0, child.0))
        })
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn segments(&self) -> impl Iterator<Item = &str> {
        self.0.split('/').filter(|segment| !segment.is_empty())
    }

    /// Last segment; `None` for the root.
    pub fn name(&self) -> Option<&str> {
        self.segments().last()
    }

    /// Containing directory; `None` for the root.
    pub fn parent(&self) -> Option<Self> {
        if self.is_root() {
            return None;
        }
        Some(match self.0.rfind('/') {
            Some(index) => Self(self.0[..index].to_owned()),
            None => Self::root(),
        })
    }

    /// The wire path for a native path that is already known to be inside
    /// `root` — the inverse of [`Self::to_native`], for paths that arrive from
    /// the platform rather than from the protocol: a watch backend's event
    /// (T070), a file dropped onto the window (T045).
    ///
    /// `root` must be the canonicalized root and `native` a path resolved
    /// against it. The prefix is stripped component-wise, not as a string, so
    /// `/ws2/x` is not accepted as being inside `/ws`; anything that still
    /// resolves outside is `PermissionDenied` rather than trusted
    /// (`docs/architecture/security.md`). A component that is not valid UTF-8
    /// is refused too: the protocol carries strings, and a lossy name would
    /// not round-trip back to this file.
    pub fn from_native(root: &Path, native: &Path) -> Result<Self, WorkspaceError> {
        let shown = native.display().to_string();
        let relative = native.strip_prefix(root).map_err(|_| {
            WorkspaceError::PermissionDenied(format!("{shown} resolves outside the workspace root"))
        })?;
        let mut path = Self::root();
        for component in relative.components() {
            let Component::Normal(name) = component else {
                // Stripping a canonical root off a canonical path leaves only
                // normal components; anything else means the caller passed
                // something it had not resolved.
                return Err(WorkspaceError::PermissionDenied(format!(
                    "{shown} is not a resolved path inside the workspace root"
                )));
            };
            let name = name.to_str().ok_or_else(|| {
                WorkspaceError::invalid_path("path", "must be valid UTF-8 in every component")
            })?;
            path = path.join(name)?;
        }
        Ok(path)
    }

    /// The path as the operating system sees it, joined to `root`. Purely
    /// lexical: no symlink is resolved and nothing is checked on disk.
    pub fn to_native(&self, root: &Path) -> PathBuf {
        let mut native = root.to_path_buf();
        for segment in self.segments() {
            native.push(segment);
        }
        native
    }
}

/// Platform-specific absolute forms that do not start with `/`: Windows
/// drive-letter (`C:/x`, `C:x`) and UNC prefixes. On Unix these are ordinary
/// names and `std` reports no prefix, which is the intended behavior.
fn looks_absolute(raw: &str) -> bool {
    Path::new(raw)
        .components()
        .any(|component| matches!(component, Component::Prefix(_) | Component::RootDir))
}

impl fmt::Display for WorkspacePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_root() {
            f.write_str("<root>")
        } else {
            f.write_str(&self.0)
        }
    }
}

impl Serialize for WorkspacePath {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for WorkspacePath {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reason(result: Result<WorkspacePath, WorkspaceError>) -> String {
        match result {
            Err(WorkspaceError::InvalidPath { field, reason }) => format!("{field}: {reason}"),
            other => panic!("expected InvalidPath, got {other:?}"),
        }
    }

    #[test]
    fn accepts_relative_forward_slash_paths() {
        for raw in [
            "",
            "src",
            "src/main.rs",
            "a/b/c/d",
            "file with spaces.txt",
            "héllo wörld/日本語.md",
            "emoji 🎉/x",
            ".hidden/.also-hidden",
            "..foo/bar..", // dots are only special as whole segments
            "-",
            "~",
        ] {
            let path = WorkspacePath::parse(raw).unwrap_or_else(|e| panic!("{raw:?}: {e}"));
            assert_eq!(path.as_str(), raw);
        }
    }

    #[test]
    fn rejects_traversal_absolute_and_malformed_paths() {
        let cases: &[(&str, &str)] = &[
            ("..", "'.' or '..'"),
            ("../etc/passwd", "'.' or '..'"),
            ("src/../../etc", "'.' or '..'"),
            ("src/..", "'.' or '..'"),
            (".", "'.' or '..'"),
            ("./src", "'.' or '..'"),
            ("/etc/passwd", "relative"),
            ("//server/share", "relative"),
            ("src\\main.rs", "separator"),
            ("..\\..\\x", "separator"),
            ("src//main.rs", "empty segments"),
            ("src/", "empty segments"),
            ("a\u{0}b", "NUL"),
        ];
        for (raw, expected) in cases {
            let message = reason(WorkspacePath::parse(raw));
            assert!(message.contains(expected), "{raw:?} -> {message}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn rejects_windows_drive_and_unc_forms() {
        for raw in ["C:/Windows", "C:", "c:foo", "//?/C:/x"] {
            let message = reason(WorkspacePath::parse(raw));
            assert!(message.contains("relative"), "{raw:?} -> {message}");
        }
    }

    #[test]
    fn join_parent_and_name_are_consistent() {
        let root = WorkspacePath::root();
        assert!(root.is_root());
        assert_eq!(root.name(), None);
        assert_eq!(root.parent(), None);
        let src = root.join("src").unwrap();
        assert_eq!(src.as_str(), "src");
        assert_eq!(src.parent(), Some(root.clone()));
        let main = src.join("main.rs").unwrap();
        assert_eq!(main.as_str(), "src/main.rs");
        assert_eq!(main.name(), Some("main.rs"));
        assert_eq!(main.parent(), Some(src.clone()));
        assert_eq!(main.segments().collect::<Vec<_>>(), ["src", "main.rs"]);
        for bad in ["", "..", "a/b", "x\\y", "/x"] {
            assert!(src.join(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn from_native_is_the_inverse_of_to_native() {
        let root = Path::new("/ws");
        for wire in ["", "src", "src/main.rs", "a/b c/d", "héllo/日本語.md"] {
            let path = WorkspacePath::parse(wire).expect(wire);
            let native = path.to_native(root);
            let back = WorkspacePath::from_native(root, &native).expect(wire);
            assert_eq!(back, path, "{wire}");
        }
    }

    #[test]
    fn from_native_refuses_paths_outside_the_root() {
        let root = Path::new("/ws");
        for outside in [
            "/etc/passwd",
            // A sibling that merely shares the prefix as a string.
            "/ws2/src",
            "/",
            // Unresolved traversal: the caller was supposed to canonicalize.
            "/ws/../etc",
        ] {
            let error = WorkspacePath::from_native(root, Path::new(outside)).expect_err(outside);
            assert!(
                matches!(error, WorkspaceError::PermissionDenied(_)),
                "{outside} -> {error:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn from_native_refuses_a_component_that_is_not_utf8() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let root = Path::new("/ws");
        let native = root.join(OsStr::from_bytes(&[0x62, 0x61, 0x64, 0xff]));
        assert!(matches!(
            WorkspacePath::from_native(root, &native),
            Err(WorkspaceError::InvalidPath { .. })
        ));
    }

    #[test]
    fn to_native_joins_segments_lexically() {
        let root = Path::new("/ws");
        assert_eq!(WorkspacePath::root().to_native(root), root);
        assert_eq!(
            WorkspacePath::parse("a/b c/d").unwrap().to_native(root),
            root.join("a").join("b c").join("d")
        );
    }

    #[test]
    fn serde_uses_the_wire_string_and_validates_on_input() {
        let path = WorkspacePath::parse("src/lib").unwrap();
        assert_eq!(serde_json::to_string(&path).unwrap(), "\"src/lib\"");
        let back: WorkspacePath = serde_json::from_str("\"src/lib\"").unwrap();
        assert_eq!(back, path);
        let root: WorkspacePath = serde_json::from_str("\"\"").unwrap();
        assert!(root.is_root());
        let error = serde_json::from_str::<WorkspacePath>("\"../x\"").unwrap_err();
        assert!(error.to_string().contains("'..'"), "{error}");
    }

    #[test]
    fn display_names_the_root() {
        assert_eq!(WorkspacePath::root().to_string(), "<root>");
        assert_eq!(WorkspacePath::parse("a/b").unwrap().to_string(), "a/b");
    }
}
