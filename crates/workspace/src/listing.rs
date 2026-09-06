//! One-level directory listing: sorted (directories first, then by name),
//! paginated with an opaque cursor, and cancellable between entries.
//!
//! Nothing is filtered: hidden entries carry `hidden: true`, well-known heavy
//! directories carry `ignoredHint: true`, symlinks are reported as
//! `kind: "symlink"` with where they resolve. The lister reads exactly one
//! directory per call ([`ListStats::directories_read`] proves it) and never
//! follows a symlink to list its target.

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::WorkspaceError;
use crate::path::WorkspacePath;
use crate::root::{WorkspaceRoot, canonicalize};

/// Largest page a single call returns; larger directories are paginated.
pub const MAX_PAGE_SIZE: usize = 5_000;
pub const DEFAULT_PAGE_SIZE: usize = MAX_PAGE_SIZE;

/// Directory names that are usually generated or heavy. They are still
/// listed; the flag lets the UI de-emphasize or defer them.
pub const IGNORED_DIRECTORY_NAMES: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "target",
    "dist",
    "build",
    "out",
    "__pycache__",
    ".venv",
    ".cache",
];

/// Whether `path` lies in a generated or dependency subtree: any of its
/// segments is one of [`IGNORED_DIRECTORY_NAMES`].
///
/// One answer, shared by everything that has to decide whether to descend —
/// the watcher (T070) prunes its registrations with it, and a directory walk
/// (T080) calls it per entry — so "ignored" never means two different things
/// in two places. It is deliberately name-based and synchronous: nothing is
/// read from disk, and the ignored directory *itself* is not ignored by its
/// parent, only its contents are.
pub fn is_ignored(path: &WorkspacePath) -> bool {
    path.segments()
        .any(|segment| IGNORED_DIRECTORY_NAMES.contains(&segment))
}

/// How many entries are processed between two cancellation checks.
const CANCEL_CHECK_INTERVAL: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum DirEntryKind {
    File,
    Dir,
    /// A symbolic link (or Windows junction); see [`DirEntry::symlink`].
    Symlink,
    /// Sockets, devices, pipes and anything else.
    Other,
}

/// Where a symlink points, without following it for listing purposes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SymlinkInfo {
    /// The target does not exist (or cannot be resolved).
    pub broken: bool,
    /// The fully resolved target lies inside the workspace root. Targets
    /// outside are never followed by `workspace.list`.
    pub resolves_inside_root: bool,
    /// Kind of the resolved target; absent when broken.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub target_kind: Option<DirEntryKind>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DirEntry {
    /// The name as the filesystem reports it (lossily decoded when not UTF-8).
    pub name: String,
    pub kind: DirEntryKind,
    /// Byte size, files only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub size: Option<u64>,
    /// Last modification, milliseconds since the Unix epoch, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub modified_at: Option<u64>,
    /// Name starts with a dot.
    pub hidden: bool,
    /// Directory named like a generated or dependency directory
    /// ([`IGNORED_DIRECTORY_NAMES`]).
    pub ignored_hint: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub symlink: Option<SymlinkInfo>,
}

/// One page of a directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryPage {
    /// The directory that was listed (as requested).
    pub path: WorkspacePath,
    /// Directories first, then by name (Unicode code point order).
    pub entries: Vec<DirEntry>,
    /// Number of entries in the directory when it was read, across all pages.
    pub total: u64,
    /// More entries follow; pass `cursor` back to continue.
    pub truncated: bool,
    /// Opaque continuation token, present when `truncated`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ListOptions {
    /// Entries per page, clamped to `1..=MAX_PAGE_SIZE`.
    pub page_size: usize,
    /// Continue after the entry a previous page ended with.
    pub cursor: Option<String>,
}

impl Default for ListOptions {
    fn default() -> Self {
        Self {
            page_size: DEFAULT_PAGE_SIZE,
            cursor: None,
        }
    }
}

/// What a listing cost; logged by the core and asserted by tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListStats {
    /// Directories opened with `read_dir`. Always 1: listing is one level.
    pub directories_read: u32,
    /// Entries read from the directory before pagination.
    pub entries_seen: u64,
    pub elapsed: Duration,
}

#[derive(Debug, Clone)]
pub struct Listing {
    pub page: DirectoryPage,
    pub stats: ListStats,
}

/// List one level of `path` inside `root`. `cancelled` is polled every
/// [`CANCEL_CHECK_INTERVAL`] entries; when it returns `true` the listing stops
/// with [`WorkspaceError::Cancelled`] and nothing partial is returned.
pub fn list_directory(
    root: &WorkspaceRoot,
    path: &WorkspacePath,
    options: &ListOptions,
    cancelled: &dyn Fn() -> bool,
) -> Result<Listing, WorkspaceError> {
    let started = Instant::now();
    let page_size = options.page_size.clamp(1, MAX_PAGE_SIZE);
    let after = options.cursor.as_deref().map(Cursor::decode).transpose()?;

    let resolved = root.resolve(path)?;
    let metadata = fs::metadata(resolved.canonical())
        .map_err(|error| WorkspaceError::from_io(error, path.to_string()))?;
    if !metadata.is_dir() {
        return Err(WorkspaceError::NotADirectory(format!(
            "{path} is not a directory"
        )));
    }

    let read_dir = fs::read_dir(resolved.native())
        .map_err(|error| WorkspaceError::from_io(error, path.to_string()))?;
    let mut candidates: Vec<Candidate> = Vec::new();
    for (index, entry) in read_dir.enumerate() {
        if index % CANCEL_CHECK_INTERVAL == 0 && cancelled() {
            return Err(WorkspaceError::Cancelled);
        }
        let entry = entry.map_err(|error| WorkspaceError::from_io(error, path.to_string()))?;
        // `file_type` is free on every platform (no extra syscall); a failure
        // here means the entry vanished mid-read, which is not an error of the
        // listing as a whole.
        let file_type = entry.file_type().ok();
        let name = entry.file_name().to_string_lossy().into_owned();
        let rank = if file_type.is_some_and(|ft| ft.is_dir()) {
            Rank::Dir
        } else {
            Rank::Other
        };
        candidates.push(Candidate {
            rank,
            name,
            entry,
            file_type,
        });
    }
    let total = candidates.len();
    candidates.sort_unstable_by(|a, b| a.key().cmp(&b.key()));

    let start = match &after {
        Some(cursor) => candidates.partition_point(|candidate| candidate.key() <= cursor.key()),
        None => 0,
    };
    let remaining = &candidates[start..];
    let truncated = remaining.len() > page_size;
    let window = &remaining[..remaining.len().min(page_size)];

    let mut entries = Vec::with_capacity(window.len());
    for (index, candidate) in window.iter().enumerate() {
        if index % CANCEL_CHECK_INTERVAL == 0 && cancelled() {
            return Err(WorkspaceError::Cancelled);
        }
        entries.push(describe(root, candidate));
    }
    let cursor = if truncated {
        window.last().map(|last| Cursor::from(last).encode())
    } else {
        None
    };

    let stats = ListStats {
        directories_read: 1,
        entries_seen: total as u64,
        elapsed: started.elapsed(),
    };
    tracing::debug!(
        %path,
        total,
        page = entries.len(),
        truncated,
        elapsed_ms = stats.elapsed.as_millis() as u64,
        "directory listed"
    );
    Ok(Listing {
        page: DirectoryPage {
            path: path.clone(),
            entries,
            total: total as u64,
            truncated,
            cursor,
        },
        stats,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Rank {
    Dir = 0,
    Other = 1,
}

struct Candidate {
    rank: Rank,
    name: String,
    entry: fs::DirEntry,
    file_type: Option<fs::FileType>,
}

impl Candidate {
    fn key(&self) -> (Rank, &str) {
        (self.rank, self.name.as_str())
    }
}

/// Keyset cursor: the sort key of the last entry of a page. Stable when the
/// directory changes between pages (no entry is skipped or repeated because
/// of an insertion elsewhere), and opaque to the client.
struct Cursor {
    rank: Rank,
    name: String,
}

impl Cursor {
    fn key(&self) -> (Rank, &str) {
        (self.rank, self.name.as_str())
    }

    fn encode(&self) -> String {
        let tag = match self.rank {
            Rank::Dir => 'd',
            Rank::Other => 'f',
        };
        format!("{tag}{}", self.name)
    }

    fn decode(raw: &str) -> Result<Self, WorkspaceError> {
        let invalid = || WorkspaceError::invalid_path("cursor", "is not a listing cursor");
        let mut chars = raw.chars();
        let rank = match chars.next() {
            Some('d') => Rank::Dir,
            Some('f') => Rank::Other,
            _ => return Err(invalid()),
        };
        let name = chars.as_str();
        if name.is_empty() {
            return Err(invalid());
        }
        Ok(Self {
            rank,
            name: name.to_owned(),
        })
    }
}

impl From<&Candidate> for Cursor {
    fn from(candidate: &Candidate) -> Self {
        Self {
            rank: candidate.rank,
            name: candidate.name.clone(),
        }
    }
}

fn describe(root: &WorkspaceRoot, candidate: &Candidate) -> DirEntry {
    let kind = candidate.file_type.map_or(DirEntryKind::Other, kind_of);
    // `DirEntry::metadata` does not follow symlinks; it is free on Windows and
    // one `lstat` elsewhere.
    let metadata = candidate.entry.metadata().ok();
    let size = match kind {
        DirEntryKind::File => metadata.as_ref().map(fs::Metadata::len),
        _ => None,
    };
    let modified_at = metadata
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(to_millis);
    let symlink = match kind {
        DirEntryKind::Symlink => Some(describe_symlink(root, &candidate.entry.path())),
        _ => None,
    };
    DirEntry {
        hidden: candidate.name.starts_with('.'),
        ignored_hint: kind == DirEntryKind::Dir
            && IGNORED_DIRECTORY_NAMES.contains(&candidate.name.as_str()),
        name: candidate.name.clone(),
        kind,
        size,
        modified_at,
        symlink,
    }
}

fn describe_symlink(root: &WorkspaceRoot, link: &Path) -> SymlinkInfo {
    match canonicalize(link) {
        Err(_) => SymlinkInfo {
            broken: true,
            resolves_inside_root: false,
            target_kind: None,
        },
        Ok(target) => SymlinkInfo {
            broken: false,
            resolves_inside_root: root.contains(&target),
            target_kind: fs::metadata(&target).ok().map(|m| kind_of(m.file_type())),
        },
    }
}

fn kind_of(file_type: fs::FileType) -> DirEntryKind {
    if file_type.is_symlink() {
        DirEntryKind::Symlink
    } else if file_type.is_dir() {
        DirEntryKind::Dir
    } else if file_type.is_file() {
        DirEntryKind::File
    } else {
        DirEntryKind::Other
    }
}

fn to_millis(time: SystemTime) -> Option<u64> {
    let elapsed = time.duration_since(UNIX_EPOCH).ok()?;
    u64::try_from(elapsed.as_millis()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_round_trips_and_rejects_garbage() {
        for (rank, name) in [(Rank::Dir, "src"), (Rank::Other, "a:b/c d")] {
            let encoded = Cursor {
                rank,
                name: name.to_owned(),
            }
            .encode();
            let decoded = Cursor::decode(&encoded).unwrap();
            assert_eq!(decoded.rank, rank);
            assert_eq!(decoded.name, name);
        }
        for bad in ["", "d", "x", "zname"] {
            assert!(
                matches!(
                    Cursor::decode(bad),
                    Err(WorkspaceError::InvalidPath {
                        field: "cursor",
                        ..
                    })
                ),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn kinds_serialize_lowercase() {
        assert_eq!(
            serde_json::to_string(&DirEntryKind::Symlink).unwrap(),
            "\"symlink\""
        );
        let back: DirEntryKind = serde_json::from_str("\"dir\"").unwrap();
        assert_eq!(back, DirEntryKind::Dir);
    }

    #[test]
    fn entries_serialize_camel_case_and_omit_absent_fields() {
        let entry = DirEntry {
            name: ".git".into(),
            kind: DirEntryKind::Dir,
            size: None,
            modified_at: Some(5),
            hidden: true,
            ignored_hint: true,
            symlink: None,
        };
        assert_eq!(
            serde_json::to_value(&entry).unwrap(),
            serde_json::json!({
                "name": ".git", "kind": "dir", "modifiedAt": 5, "hidden": true, "ignoredHint": true
            })
        );
    }

    #[test]
    fn is_ignored_matches_any_segment_not_just_the_name() {
        for wire in [
            "node_modules",
            "node_modules/react",
            "node_modules/react/dist/index.js",
            "a/b/target",
            "a/b/target/debug/deps/x.rlib",
            ".git",
            "crates/x/.git/HEAD",
        ] {
            let path = WorkspacePath::parse(wire).expect(wire);
            assert!(is_ignored(&path), "{wire} should be ignored");
        }
        for wire in [
            "",
            "src",
            "src/main.rs",
            "my-node_modules",
            "targets",
            "a/targeting/b",
            ".gitignore",
        ] {
            let path = WorkspacePath::parse(wire).expect(wire);
            assert!(!is_ignored(&path), "{wire} should not be ignored");
        }
    }
}
