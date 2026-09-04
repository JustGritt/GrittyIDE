//! File I/O for documents (T030): whole-file reads with a size bound, a disk
//! probe for external-modification detection, content hashing and atomic
//! replacement of a file.
//!
//! Atomic write (addendum §4.3, plan §11): the new content goes to a temporary
//! file in the destination's directory, is flushed with `fsync`, receives the
//! destination's permissions and is renamed over the destination. A failure at
//! any step leaves the destination untouched and removes the temporary file;
//! a process death between the write and the rename leaves the destination
//! untouched and a `.<name>.gritty-<pid>-<n>.tmp` file behind (see
//! [`is_temp_file_name`]).

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ts_rs::TS;

use crate::error::WorkspaceError;
use crate::path::WorkspacePath;
use crate::root::WorkspaceRoot;

/// SHA-256 of a file's bytes, as `sha256:<64 lower-case hex digits>`. Equal
/// hashes mean equal content for every purpose of the document model (dirty
/// state, external modification, recovery reconciliation).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub struct ContentHash(#[ts(type = "string")] String);

impl ContentHash {
    pub fn of(bytes: &[u8]) -> Self {
        Self::from_digest(Sha256::digest(bytes))
    }

    /// Hash of the bytes [`encode_text`](crate::detect::encode_text) would
    /// produce, without materializing them.
    pub fn of_text(text: &str, encoding: crate::detect::DocumentEncoding) -> Self {
        let mut hasher = Sha256::new();
        if encoding.has_bom() {
            hasher.update(crate::detect::UTF8_BOM);
        }
        hasher.update(text.as_bytes());
        Self::from_digest(hasher.finalize())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// A hash read back from storage; `None` unless it has the exact
    /// `sha256:<64 hex>` shape this type produces.
    pub fn parse(raw: &str) -> Option<Self> {
        let hex = raw.strip_prefix("sha256:")?;
        (hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()))
        .then(|| Self(raw.to_owned()))
    }

    fn from_digest(digest: impl AsRef<[u8]>) -> Self {
        let mut hex = String::with_capacity(7 + 64);
        hex.push_str("sha256:");
        for byte in digest.as_ref() {
            use std::fmt::Write as _;
            // Writing to a String cannot fail.
            let _ = write!(hex, "{byte:02x}");
        }
        Self(hex)
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What `stat` says about a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStat {
    pub size: u64,
    /// Last modification, milliseconds since the Unix epoch, when known.
    pub modified_at: Option<u64>,
    /// No write permission bit is set (`Permissions::readonly`). Ownership is
    /// not consulted: a file writable by its owner but owned by someone else
    /// reports `false` and fails at save time instead.
    pub read_only: bool,
}

impl FileStat {
    fn from_metadata(metadata: &fs::Metadata) -> Self {
        Self {
            size: metadata.len(),
            modified_at: metadata.modified().ok().and_then(to_millis),
            read_only: metadata.permissions().readonly(),
        }
    }
}

/// A file read in full.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSnapshot {
    pub bytes: Vec<u8>,
    pub hash: ContentHash,
    pub stat: FileStat,
}

/// What a probe found on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiskProbe {
    /// The path does not exist (or a component of it disappeared).
    Missing,
    /// The file exists. `hash` is `None` when the file has grown past the
    /// read bound and could not be hashed; it then necessarily differs from
    /// anything the document model holds.
    Present {
        stat: FileStat,
        hash: Option<ContentHash>,
    },
}

/// Read the file at `path` (symlinks inside the root are followed). Fails
/// with `NotAFile` for directories and special files, `TooLarge` above
/// `max_bytes`, and the usual `NotFound` / `PermissionDenied` otherwise.
pub fn read_file(
    root: &WorkspaceRoot,
    path: &WorkspacePath,
    max_bytes: u64,
) -> Result<FileSnapshot, WorkspaceError> {
    let resolved = root.resolve(path)?;
    let metadata = fs::metadata(resolved.canonical())
        .map_err(|error| WorkspaceError::from_io(error, path.to_string()))?;
    if !metadata.is_file() {
        return Err(WorkspaceError::NotAFile(format!("{path} is not a file")));
    }
    if metadata.len() > max_bytes {
        return Err(WorkspaceError::TooLarge {
            path: path.to_string(),
            size: metadata.len(),
            limit: max_bytes,
        });
    }
    let bytes = fs::read(resolved.canonical())
        .map_err(|error| WorkspaceError::from_io(error, path.to_string()))?;
    Ok(FileSnapshot {
        hash: ContentHash::of(&bytes),
        // Sizes from the read, not the earlier stat: the file may have changed
        // in between and the hash describes what was actually read.
        stat: FileStat {
            size: bytes.len() as u64,
            ..FileStat::from_metadata(&metadata)
        },
        bytes,
    })
}

/// Look at the file behind `path` without keeping its content: exists or not,
/// and if so its stat and hash. Never returns `NotFound`.
pub fn probe_file(
    root: &WorkspaceRoot,
    path: &WorkspacePath,
    max_bytes: u64,
) -> Result<DiskProbe, WorkspaceError> {
    let resolved = match root.resolve(path) {
        Ok(resolved) => resolved,
        Err(WorkspaceError::NotFound(_)) => return Ok(DiskProbe::Missing),
        Err(error) => return Err(error),
    };
    let metadata = match fs::metadata(resolved.canonical()) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(DiskProbe::Missing),
        Err(error) => return Err(WorkspaceError::from_io(error, path.to_string())),
    };
    if !metadata.is_file() {
        return Err(WorkspaceError::NotAFile(format!("{path} is not a file")));
    }
    let stat = FileStat::from_metadata(&metadata);
    if stat.size > max_bytes {
        return Ok(DiskProbe::Present { stat, hash: None });
    }
    let bytes = match fs::read(resolved.canonical()) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(DiskProbe::Missing),
        Err(error) => return Err(WorkspaceError::from_io(error, path.to_string())),
    };
    Ok(DiskProbe::Present {
        stat: FileStat {
            size: bytes.len() as u64,
            ..stat
        },
        hash: Some(ContentHash::of(&bytes)),
    })
}

/// Test hook: where an atomic write should go wrong. Production callers pass
/// [`AtomicWriteFault::None`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AtomicWriteFault {
    #[default]
    None,
    /// Fail after the temporary file was written and flushed, before the
    /// rename; the failure path must clean up.
    FailBeforeRename,
    /// Simulate the process dying after the temporary file was written and
    /// flushed: return an error *without* cleaning up, leaving the temporary
    /// file behind the way a real crash would.
    CrashBeforeRename,
}

/// Replace `target` (an absolute native path, already resolved against a
/// root) with `bytes` atomically. Returns the stat of the written file.
pub fn write_file_atomic(
    target: &Path,
    bytes: &[u8],
    fault: AtomicWriteFault,
) -> Result<FileStat, WorkspaceError> {
    let shown = target.display().to_string();
    let parent = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    let Some(parent) = parent else {
        return Err(WorkspaceError::invalid_path("path", "must name a file"));
    };
    let existing = match fs::metadata(target) {
        Ok(metadata) if metadata.is_dir() => {
            return Err(WorkspaceError::NotAFile(format!("{shown} is a directory")));
        }
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(WorkspaceError::from_io(error, shown)),
    };

    let (temp_path, mut temp) = create_temp_file(parent, target)
        .map_err(|error| WorkspaceError::from_io(error, format!("temporary file for {shown}")))?;
    let cleanup = TempGuard(Some(temp_path.clone()));

    let mut write = || -> io::Result<()> {
        temp.write_all(bytes)?;
        temp.sync_all()?;
        Ok(())
    };
    write().map_err(|error| WorkspaceError::from_io(error, format!("writing {shown}")))?;
    drop(temp);
    if let Some(existing) = &existing
        && let Err(error) = fs::set_permissions(&temp_path, existing.permissions())
    {
        // Not fatal: the content is what matters, and a filesystem that refuses
        // permission changes (some network shares) still gets the file.
        tracing::warn!(path = %shown, %error, "could not carry permissions over to the new file");
    }
    match fault {
        AtomicWriteFault::None => {}
        AtomicWriteFault::FailBeforeRename => {
            return Err(WorkspaceError::Io {
                context: format!("writing {shown}"),
                source: io::Error::other("injected failure before rename"),
            });
        }
        AtomicWriteFault::CrashBeforeRename => {
            std::mem::forget(cleanup);
            return Err(WorkspaceError::Io {
                context: format!("writing {shown}"),
                source: io::Error::other("injected crash before rename"),
            });
        }
    }
    fs::rename(&temp_path, target)
        .map_err(|error| WorkspaceError::from_io(error, format!("replacing {shown}")))?;
    // The rename is durable only once the directory entry is flushed.
    std::mem::forget(cleanup);
    sync_directory(parent);
    let metadata = fs::metadata(target).map_err(|error| WorkspaceError::from_io(error, shown))?;
    Ok(FileStat::from_metadata(&metadata))
}

/// Whether a directory entry is a temporary file left by an interrupted
/// atomic write (`.<name>.gritty-<pid>-<n>.tmp`).
pub fn is_temp_file_name(name: &str) -> bool {
    name.starts_with('.') && name.ends_with(TEMP_SUFFIX) && name.contains(TEMP_INFIX)
}

const TEMP_INFIX: &str = ".gritty-";
const TEMP_SUFFIX: &str = ".tmp";

fn create_temp_file(parent: &Path, target: &Path) -> io::Result<(PathBuf, File)> {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_owned());
    let pid = std::process::id();
    for _attempt in 0..16 {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(".{name}{TEMP_INFIX}{pid}-{n}{TEMP_SUFFIX}"));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => return Ok((candidate, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not find a free temporary file name",
    ))
}

/// Removes the temporary file unless the write completed (`forget`).
struct TempGuard(Option<PathBuf>);

impl Drop for TempGuard {
    fn drop(&mut self) {
        if let Some(path) = self.0.take()
            && let Err(error) = fs::remove_file(&path)
            && error.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!(path = %path.display(), %error, "could not remove temporary file");
        }
    }
}

#[cfg(unix)]
fn sync_directory(dir: &Path) {
    // Best effort: a directory that cannot be opened or synced (some
    // filesystems refuse) does not make the write fail.
    if let Ok(handle) = File::open(dir)
        && let Err(error) = handle.sync_all()
    {
        tracing::debug!(dir = %dir.display(), %error, "directory fsync failed");
    }
}

#[cfg(not(unix))]
fn sync_directory(_dir: &Path) {}

fn to_millis(time: SystemTime) -> Option<u64> {
    let elapsed = time.duration_since(UNIX_EPOCH).ok()?;
    u64::try_from(elapsed.as_millis()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_prefixed_lowercase_hex_and_deterministic() {
        let hash = ContentHash::of(b"hello");
        assert_eq!(
            hash.as_str(),
            "sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        assert_eq!(hash, ContentHash::of(b"hello"));
        assert_ne!(hash, ContentHash::of(b"hello\n"));
        assert_eq!(serde_json::to_string(&hash).unwrap(), format!("\"{hash}\""));
        let empty = ContentHash::of(b"");
        assert_eq!(empty.as_str().len(), "sha256:".len() + 64);
    }

    #[test]
    fn of_text_matches_the_hash_of_the_encoded_bytes() {
        use crate::detect::{DocumentEncoding, encode_text};
        for encoding in [DocumentEncoding::Utf8, DocumentEncoding::Utf8Bom] {
            for text in ["", "héllo\r\n", "plain"] {
                assert_eq!(
                    ContentHash::of_text(text, encoding),
                    ContentHash::of(&encode_text(text, encoding)),
                    "{encoding} {text:?}"
                );
            }
        }
        assert_ne!(
            ContentHash::of_text("x", DocumentEncoding::Utf8),
            ContentHash::of_text("x", DocumentEncoding::Utf8Bom)
        );
    }

    #[test]
    fn temp_file_names_are_recognizable_and_hidden() {
        let dir = std::env::temp_dir();
        let (path, file) = create_temp_file(&dir, Path::new("/x/notes.md")).unwrap();
        drop(file);
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        fs::remove_file(&path).unwrap();
        assert!(name.starts_with(".notes.md.gritty-"), "{name}");
        assert!(is_temp_file_name(&name), "{name}");
        assert!(!is_temp_file_name("notes.md"));
        assert!(!is_temp_file_name(".hidden.tmp"));
        assert!(!is_temp_file_name(".gritty-1.txt"));
    }
}
