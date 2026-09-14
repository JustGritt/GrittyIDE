//! Changing the tree (T042, ADR-028): create, rename/move, copy, remove and
//! trash.
//!
//! Every entry point resolves through [`WorkspaceRoot`] before it touches the
//! disk, with the same lexical-then-physical rule the rest of the crate obeys
//! (ADR-010): a path that resolves outside the root is `PermissionDenied`, and
//! it is refused *before* anything is written, not after.
//!
//! Two resolution rules differ from [`WorkspaceRoot::resolve`] and exist here:
//!
//! - **An existing entry is addressed as itself, never as what it points at.**
//!   The parent is canonicalized and proven inside the root, and the final
//!   component is appended by name. So a broken symlink and a symlink pointing
//!   outside the root — both of which the tree shows, and neither of which
//!   `resolve` will return — can still be renamed or removed. Nothing here ever
//!   follows a final link, which is why a rename moves the link rather than its
//!   target.
//! - **A new entry needs only its deepest *existing* ancestor to resolve.**
//!   That ancestor is canonicalized and checked, and every segment below it is a
//!   plain name this module creates itself, so it cannot escape. This is what
//!   lets one call create `a/b/c.ts` when `a/b` does not exist yet.
//!
//! Nothing here is atomic across more than one entry. A recursive copy that
//! fails or is cancelled part-way leaves what it already wrote, and says how
//! much that was ([`CopyOutcome`]); the alternative is a rollback that can fail
//! in the same ways as the copy, which would be a recovery claim rather than a
//! recovery (hard rule 12).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::error::WorkspaceError;
use crate::file::is_temp_file_name;
use crate::path::WorkspacePath;
use crate::root::WorkspaceRoot;

/// What [`create`] should make.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Directory,
}

/// What a [`copy`] managed before it stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CopyOutcome {
    /// Entries written — files, directories and symlinks alike.
    pub copied: usize,
    /// The caller cancelled. What `copied` counts is on the disk and stays
    /// there; nothing is rolled back.
    pub cancelled: bool,
}

/// Gritty's own atomic-save temporaries are not entries a user operates on:
/// they exist for the milliseconds between a write and its rename, and acting
/// on one races `documents.save` for the same bytes.
fn refuse_temp_name(path: &WorkspacePath, field: &'static str) -> Result<(), WorkspaceError> {
    if path.name().is_some_and(is_temp_file_name) {
        return Err(WorkspaceError::invalid_path(
            field,
            "is a temporary file Gritty is writing",
        ));
    }
    Ok(())
}

/// The native path of an entry that must already exist, addressed as itself.
///
/// The parent is resolved (canonicalized, proven inside the root, required to
/// be a directory) and the name appended, so the final component is never
/// followed. Existence is checked with `symlink_metadata` for the same reason.
fn resolve_existing(
    root: &WorkspaceRoot,
    path: &WorkspacePath,
    field: &'static str,
) -> Result<PathBuf, WorkspaceError> {
    let Some(name) = path.name() else {
        return Err(WorkspaceError::invalid_path(field, "must name an entry"));
    };
    refuse_temp_name(path, field)?;
    let parent = path.parent().expect("a named path has a parent");
    let resolved = root.resolve(&parent)?;
    if !resolved.canonical().is_dir() {
        return Err(WorkspaceError::NotADirectory(format!(
            "{parent} is not a directory"
        )));
    }
    let native = resolved.canonical().join(name);
    fs::symlink_metadata(&native)
        .map_err(|error| WorkspaceError::from_io(error, path.to_string()))?;
    Ok(native)
}

/// The native path to create at. The target must not exist; its deepest
/// existing ancestor must resolve inside the root and be a directory.
fn resolve_new(
    root: &WorkspaceRoot,
    path: &WorkspacePath,
    field: &'static str,
) -> Result<PathBuf, WorkspaceError> {
    if path.is_root() {
        return Err(WorkspaceError::invalid_path(field, "must name an entry"));
    }
    refuse_temp_name(path, field)?;
    let native = path.to_native(root.path());
    if fs::symlink_metadata(&native).is_ok() {
        return Err(WorkspaceError::AlreadyExists(format!(
            "{path} already exists"
        )));
    }
    // Walk up to the deepest ancestor that exists. `resolve` canonicalizes it
    // and proves it is inside the root; every segment below it is a plain name
    // (the path policy forbids `.` and `..`), so appending them cannot escape.
    let mut ancestor = path.parent().expect("a named path has a parent");
    loop {
        match root.resolve(&ancestor) {
            Ok(resolved) => {
                if !resolved.canonical().is_dir() {
                    return Err(WorkspaceError::NotADirectory(format!(
                        "{ancestor} is not a directory"
                    )));
                }
                return Ok(native);
            }
            Err(WorkspaceError::NotFound(_)) => {
                ancestor = ancestor
                    .parent()
                    .expect("the workspace root always resolves");
            }
            Err(error) => return Err(error),
        }
    }
}

/// Whether `candidate` is `ancestor` itself or lies beneath it, by wire path.
/// Purely lexical, which is all it needs to be: both sides are normalized
/// `WorkspacePath`s with no `.` or `..` segments.
fn is_at_or_below(ancestor: &WorkspacePath, candidate: &WorkspacePath) -> bool {
    if ancestor.is_root() {
        return true;
    }
    if candidate == ancestor {
        return true;
    }
    candidate
        .as_str()
        .strip_prefix(ancestor.as_str())
        .is_some_and(|rest| rest.starts_with('/'))
}

/// Whether an entry exists at `path`, without following a final symlink.
///
/// A broken link "exists" — it occupies its name, which is the question a
/// caller choosing a free destination name is actually asking. A parent that
/// does not resolve is not an error here either: nothing is there.
pub fn exists(root: &WorkspaceRoot, path: &WorkspacePath) -> bool {
    let Some(name) = path.name() else {
        return true; // the root
    };
    let Some(parent) = path.parent() else {
        return true;
    };
    root.resolve(&parent)
        .is_ok_and(|resolved| fs::symlink_metadata(resolved.canonical().join(name)).is_ok())
}

/// Create a file or a directory, and any missing directories above it.
///
/// A file is created exclusively, so an existing target is
/// [`WorkspaceError::AlreadyExists`] rather than a silent truncate.
pub fn create(
    root: &WorkspaceRoot,
    path: &WorkspacePath,
    kind: EntryKind,
) -> Result<(), WorkspaceError> {
    let native = resolve_new(root, path, "path")?;
    let parent = native.parent().expect("a resolved new path has a parent");
    fs::create_dir_all(parent)
        .map_err(|error| WorkspaceError::from_io(error, format!("{path}'s parent directory")))?;
    match kind {
        EntryKind::Directory => fs::create_dir(&native),
        EntryKind::File => fs::File::create_new(&native).map(drop),
    }
    .map_err(|error| match error.kind() {
        io::ErrorKind::AlreadyExists => {
            WorkspaceError::AlreadyExists(format!("{path} already exists"))
        }
        _ => WorkspaceError::from_io(error, path.to_string()),
    })
}

/// Rename or move `from` to `to` — the same syscall either way.
///
/// Falls back to copy-then-remove when the two are on different filesystems,
/// which a mount point inside the workspace root can arrange.
pub fn rename(
    root: &WorkspaceRoot,
    from: &WorkspacePath,
    to: &WorkspacePath,
) -> Result<(), WorkspaceError> {
    let source = resolve_existing(root, from, "path")?;
    if is_at_or_below(from, to) {
        return Err(WorkspaceError::invalid_path(
            "to",
            format!("{to} is inside {from}"),
        ));
    }
    let destination = match resolve_new(root, to, "to") {
        Ok(destination) => destination,
        // A case-only rename on a case-insensitive filesystem: `to` "exists"
        // because it *is* `from`. APFS and NTFS both do this, and refusing it
        // would make `README.md` → `readme.md` impossible on the platform
        // Gritty actually runs on.
        Err(WorkspaceError::AlreadyExists(message)) => {
            let existing = resolve_existing(root, to, "to")?;
            if same_entry(&source, &existing) {
                existing
            } else {
                return Err(WorkspaceError::AlreadyExists(message));
            }
        }
        Err(error) => return Err(error),
    };
    rename_native(&source, &destination, from, to, &|source, destination| {
        fs::rename(source, destination)
    })
}

/// The syscall half of [`rename`], with the syscall passed in so the
/// cross-device branch is testable without a second mount (FILES-PARTIAL).
fn rename_native(
    source: &Path,
    destination: &Path,
    from: &WorkspacePath,
    to: &WorkspacePath,
    syscall: &dyn Fn(&Path, &Path) -> io::Result<()>,
) -> Result<(), WorkspaceError> {
    match syscall(source, destination) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::CrossesDevices => {
            let mut copied = 0;
            copy_tree(source, destination, &|| false, &mut copied)?;
            // The copy is complete, so what is left to fail is removing the
            // *source*: name it, not the destination that now exists.
            remove_native(source, from)
        }
        Err(error) => Err(WorkspaceError::from_io(error, format!("{from} → {to}"))),
    }
}

/// Whether two native paths name the same entry once symlinks are resolved.
/// Used only to recognise a case-only rename, so a failure to canonicalize
/// (a broken link) is "not the same entry" rather than an error.
fn same_entry(left: &Path, right: &Path) -> bool {
    match (
        crate::root::canonicalize(left),
        crate::root::canonicalize(right),
    ) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

/// Copy `from` to `to`, recursively for a directory.
///
/// `cancelled` is polled once per entry. Cancellation is not an error: it
/// returns what had already been written, which stays on the disk (ADR-028).
pub fn copy(
    root: &WorkspaceRoot,
    from: &WorkspacePath,
    to: &WorkspacePath,
    cancelled: &dyn Fn() -> bool,
) -> Result<CopyOutcome, WorkspaceError> {
    let source = resolve_existing(root, from, "path")?;
    if is_at_or_below(from, to) {
        return Err(WorkspaceError::invalid_path(
            "to",
            format!("{to} is inside {from}"),
        ));
    }
    let destination = resolve_new(root, to, "to")?;
    let parent = destination
        .parent()
        .expect("a resolved new path has a parent");
    fs::create_dir_all(parent)
        .map_err(|error| WorkspaceError::from_io(error, format!("{to}'s parent directory")))?;
    let mut copied = 0;
    let completed = copy_tree(&source, &destination, cancelled, &mut copied)?;
    Ok(CopyOutcome {
        copied,
        cancelled: !completed,
    })
}

/// Returns `false` when the caller cancelled part-way.
fn copy_tree(
    source: &Path,
    destination: &Path,
    cancelled: &dyn Fn() -> bool,
    copied: &mut usize,
) -> Result<bool, WorkspaceError> {
    if cancelled() {
        return Ok(false);
    }
    let metadata = fs::symlink_metadata(source)
        .map_err(|error| WorkspaceError::from_io(error, source.display().to_string()))?;
    if metadata.is_symlink() {
        copy_symlink(source, destination)?;
        *copied += 1;
        return Ok(true);
    }
    if metadata.is_file() {
        fs::copy(source, destination)
            .map_err(|error| WorkspaceError::from_io(error, source.display().to_string()))?;
        *copied += 1;
        return Ok(true);
    }
    if !metadata.is_dir() {
        // A socket, a fifo, a device node. `list_directory` shows these as
        // `other`; copying one would either block or produce something that is
        // not what it names.
        return Err(WorkspaceError::invalid_path(
            "path",
            format!("{} is not a regular file or directory", source.display()),
        ));
    }
    fs::create_dir_all(destination)
        .map_err(|error| WorkspaceError::from_io(error, destination.display().to_string()))?;
    *copied += 1;
    let entries = fs::read_dir(source)
        .map_err(|error| WorkspaceError::from_io(error, source.display().to_string()))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| WorkspaceError::from_io(error, source.display().to_string()))?;
        let name = entry.file_name();
        if !copy_tree(&entry.path(), &destination.join(&name), cancelled, copied)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Copy a symlink as a symlink, so a copied link keeps pointing where the
/// original did instead of silently becoming a copy of the target.
#[cfg(unix)]
fn copy_symlink(source: &Path, destination: &Path) -> Result<(), WorkspaceError> {
    let target = fs::read_link(source)
        .map_err(|error| WorkspaceError::from_io(error, source.display().to_string()))?;
    std::os::unix::fs::symlink(target, destination)
        .map_err(|error| WorkspaceError::from_io(error, destination.display().to_string()))
}

#[cfg(not(unix))]
fn copy_symlink(source: &Path, _destination: &Path) -> Result<(), WorkspaceError> {
    // Windows needs a privilege or developer mode to create one, and choosing
    // between a file and a directory link is a decision this task has no way to
    // test. Refused by name rather than silently dereferenced.
    Err(WorkspaceError::invalid_path(
        "path",
        format!(
            "{} is a symlink, which cannot be copied here",
            source.display()
        ),
    ))
}

/// Delete permanently. A directory goes with everything under it.
pub fn remove(root: &WorkspaceRoot, path: &WorkspacePath) -> Result<(), WorkspaceError> {
    let native = resolve_existing(root, path, "path")?;
    remove_native(&native, path)
}

fn remove_native(native: &Path, path: &WorkspacePath) -> Result<(), WorkspaceError> {
    let metadata = fs::symlink_metadata(native)
        .map_err(|error| WorkspaceError::from_io(error, path.to_string()))?;
    // A symlink to a directory is removed as a link, never followed into.
    let result = if metadata.is_dir() {
        fs::remove_dir_all(native)
    } else {
        fs::remove_file(native)
    };
    result.map_err(|error| WorkspaceError::from_io(error, path.to_string()))
}

/// Move to the OS trash — Finder's Trash, freedesktop's, the Recycle Bin.
///
/// The one call into the `trash` crate, so the rest of Gritty never sees it
/// (hard rule 11). A volume with no trash (many network and removable mounts)
/// fails here rather than silently deleting: the caller offers a permanent
/// delete instead, and the user chooses.
pub fn trash(root: &WorkspaceRoot, path: &WorkspacePath) -> Result<(), WorkspaceError> {
    let native = resolve_existing(root, path, "path")?;
    trash::delete(&native).map_err(|error| WorkspaceError::Io {
        context: format!("{path} could not be moved to the trash"),
        source: io::Error::other(error.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A scratch directory that removes itself. There is no `tempfile` crate in
    /// the workspace; every test module rolls this, and this one matches.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("gritty-mutate-{}-{label}-{n}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("create scratch dir");
            Self(crate::root::canonicalize(&path).expect("canonicalize scratch dir"))
        }

        fn root(&self) -> WorkspaceRoot {
            WorkspaceRoot::open(&self.0).expect("open scratch root")
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn path(raw: &str) -> WorkspacePath {
        WorkspacePath::parse(raw).expect("valid wire path")
    }

    fn never() -> bool {
        false
    }

    #[test]
    fn create_makes_intermediate_directories_and_refuses_an_existing_target() {
        let temp = TempDir::new("create");
        let root = temp.root();

        create(&root, &path("a/b/c.ts"), EntryKind::File).expect("create nested file");
        assert!(temp.0.join("a/b/c.ts").is_file());

        create(&root, &path("a/b/d"), EntryKind::Directory).expect("create nested dir");
        assert!(temp.0.join("a/b/d").is_dir());

        let again = create(&root, &path("a/b/c.ts"), EntryKind::File).unwrap_err();
        assert!(matches!(again, WorkspaceError::AlreadyExists(_)));
        // The existing file was not truncated.
        assert!(temp.0.join("a/b/c.ts").is_file());
    }

    #[test]
    fn create_refuses_the_root_a_temp_name_and_a_file_as_a_parent() {
        let temp = TempDir::new("create-refuse");
        let root = temp.root();
        create(&root, &path("f.txt"), EntryKind::File).expect("create file");

        let at_root = create(&root, &WorkspacePath::root(), EntryKind::Directory).unwrap_err();
        assert!(matches!(at_root, WorkspaceError::InvalidPath { .. }));

        let temp_name = format!(".x.gritty-{}-1.tmp", std::process::id());
        let refused = create(&root, &path(&temp_name), EntryKind::File).unwrap_err();
        assert!(matches!(refused, WorkspaceError::InvalidPath { .. }));

        let under_file = create(&root, &path("f.txt/nested"), EntryKind::File).unwrap_err();
        assert!(matches!(under_file, WorkspaceError::NotADirectory(_)));
    }

    #[test]
    fn rename_moves_and_refuses_an_occupied_name() {
        let temp = TempDir::new("rename");
        let root = temp.root();
        create(&root, &path("src/a.ts"), EntryKind::File).expect("create");
        create(&root, &path("dst"), EntryKind::Directory).expect("create dir");
        create(&root, &path("dst/taken.ts"), EntryKind::File).expect("create");

        rename(&root, &path("src/a.ts"), &path("dst/b.ts")).expect("move");
        assert!(!temp.0.join("src/a.ts").exists());
        assert!(temp.0.join("dst/b.ts").is_file());

        create(&root, &path("src/c.ts"), EntryKind::File).expect("create");
        let occupied = rename(&root, &path("src/c.ts"), &path("dst/taken.ts")).unwrap_err();
        assert!(matches!(occupied, WorkspaceError::AlreadyExists(_)));
        // Refused, not half-done.
        assert!(temp.0.join("src/c.ts").is_file());
    }

    #[test]
    fn a_directory_cannot_be_moved_or_copied_into_itself() {
        let temp = TempDir::new("descendant");
        let root = temp.root();
        create(&root, &path("a/b/c.ts"), EntryKind::File).expect("create");

        let into_self = rename(&root, &path("a"), &path("a/b/a")).unwrap_err();
        assert!(matches!(
            into_self,
            WorkspaceError::InvalidPath { field: "to", .. }
        ));

        let copy_into_self = copy(&root, &path("a"), &path("a/copy"), &never).unwrap_err();
        assert!(matches!(
            copy_into_self,
            WorkspaceError::InvalidPath { field: "to", .. }
        ));

        let onto_self = rename(&root, &path("a"), &path("a")).unwrap_err();
        assert!(matches!(onto_self, WorkspaceError::InvalidPath { .. }));
    }

    #[test]
    fn copy_is_recursive_and_reports_what_a_cancellation_left() {
        let temp = TempDir::new("copy");
        let root = temp.root();
        for n in 0..4 {
            create(&root, &path(&format!("src/f{n}.txt")), EntryKind::File).expect("create");
        }
        create(&root, &path("src/deep/g.txt"), EntryKind::File).expect("create");

        let all = copy(&root, &path("src"), &path("dst"), &never).expect("copy");
        assert!(!all.cancelled);
        // 1 dir + 4 files + 1 dir + 1 file
        assert_eq!(all.copied, 7);
        assert!(temp.0.join("dst/deep/g.txt").is_file());

        // Stop after the first two entries; what was written stays.
        let seen = std::cell::Cell::new(0);
        let stop_after_two = || {
            let n = seen.get();
            seen.set(n + 1);
            n >= 2
        };
        let partial = copy(&root, &path("src"), &path("part"), &stop_after_two).expect("copy");
        assert!(partial.cancelled);
        assert!(partial.copied < all.copied);
        assert!(temp.0.join("part").is_dir());
    }

    #[test]
    fn a_symlink_is_moved_copied_and_removed_as_itself() {
        let temp = TempDir::new("symlink");
        let root = temp.root();
        create(&root, &path("target.txt"), EntryKind::File).expect("create");
        std::os::unix::fs::symlink("target.txt", temp.0.join("link")).expect("symlink");
        std::os::unix::fs::symlink("nowhere", temp.0.join("broken")).expect("broken symlink");

        // A broken link is an entry the tree shows, so it can be renamed even
        // though `resolve` would refuse to return it.
        rename(&root, &path("broken"), &path("still-broken")).expect("rename a broken link");
        assert!(fs::symlink_metadata(temp.0.join("still-broken")).is_ok());

        copy(&root, &path("link"), &path("link-copy"), &never).expect("copy a link");
        assert!(
            fs::symlink_metadata(temp.0.join("link-copy"))
                .expect("copied link exists")
                .is_symlink(),
            "a copied symlink stays a symlink"
        );

        remove(&root, &path("link")).expect("remove a link");
        assert!(!temp.0.join("link").exists());
        // Removing the link did not follow it.
        assert!(temp.0.join("target.txt").is_file());
    }

    #[test]
    fn a_path_escaping_the_root_through_a_symlink_is_refused() {
        let outside = TempDir::new("outside");
        let temp = TempDir::new("escape");
        let root = temp.root();
        fs::write(outside.0.join("secret.txt"), b"x").expect("write outside");
        std::os::unix::fs::symlink(&outside.0, temp.0.join("escape")).expect("symlink out");

        let created = create(&root, &path("escape/new.txt"), EntryKind::File).unwrap_err();
        assert!(matches!(created, WorkspaceError::PermissionDenied(_)));

        let removed = remove(&root, &path("escape/secret.txt")).unwrap_err();
        assert!(matches!(removed, WorkspaceError::PermissionDenied(_)));
        assert!(
            outside.0.join("secret.txt").is_file(),
            "nothing outside was touched"
        );
    }

    #[test]
    fn remove_takes_a_directory_with_everything_under_it() {
        let temp = TempDir::new("remove");
        let root = temp.root();
        create(&root, &path("tree/deep/f.txt"), EntryKind::File).expect("create");

        remove(&root, &path("tree")).expect("remove tree");
        assert!(!temp.0.join("tree").exists());

        let missing = remove(&root, &path("tree")).unwrap_err();
        assert!(matches!(missing, WorkspaceError::NotFound(_)));
    }

    /// What `fs::rename` answers when the two paths are on different
    /// filesystems. Injected, because no fixture has a mount inside a root.
    fn across_devices(_: &Path, _: &Path) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::CrossesDevices))
    }

    #[test]
    fn a_rename_across_devices_copies_then_removes_the_source() {
        let temp = TempDir::new("exdev");
        create(&temp.root(), &path("from/deep/f.txt"), EntryKind::File).expect("create");
        fs::write(temp.0.join("from/deep/f.txt"), b"kept").expect("write");
        fs::create_dir_all(temp.0.join("into")).expect("target");

        rename_native(
            &temp.0.join("from"),
            &temp.0.join("into/from"),
            &path("from"),
            &path("into/from"),
            &across_devices,
        )
        .expect("the fallback moves the tree");

        assert!(!temp.0.join("from").exists(), "the source is gone");
        assert_eq!(
            fs::read(temp.0.join("into/from/deep/f.txt")).expect("copied"),
            b"kept"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_remove_after_a_cross_device_copy_names_the_source() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new("exdev-locked");
        create(&temp.root(), &path("locked/a.txt"), EntryKind::File).expect("create");
        fs::create_dir_all(temp.0.join("into")).expect("target");
        // The file cannot be unlinked from a directory that is not writable.
        fs::set_permissions(temp.0.join("locked"), fs::Permissions::from_mode(0o555))
            .expect("lock");

        let result = rename_native(
            &temp.0.join("locked/a.txt"),
            &temp.0.join("into/a.txt"),
            &path("locked/a.txt"),
            &path("into/a.txt"),
            &across_devices,
        );
        let source_survived = temp.0.join("locked/a.txt").exists();
        fs::set_permissions(temp.0.join("locked"), fs::Permissions::from_mode(0o755))
            .expect("unlock, so the scratch dir can be removed");

        // Root can unlink from a 0o555 directory; there is nothing to assert.
        if !source_survived {
            return;
        }
        let message = result.expect_err("removing the source failed").to_string();
        assert!(
            message.contains("locked/a.txt") && !message.contains("into/a.txt"),
            "the error names the source it could not remove, not the copy: {message}"
        );
    }
}
