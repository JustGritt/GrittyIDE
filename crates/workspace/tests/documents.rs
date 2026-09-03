//! Integration tests for the T030 file primitives against a staged copy of
//! `fixtures/weird-files` (under `target/fixtures/weird-files`) and scratch
//! trees for everything that writes.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use gritty_workspace::fixtures::{
    WEIRD_DENIED, WEIRD_FILES_DIR, WEIRD_LARGE_TEXT, WEIRD_LARGE_TEXT_BYTES, WEIRD_LINKS,
    stage_weird_files,
};
use gritty_workspace::{
    AtomicWriteFault, ContentHash, ContentKind, DirEntryKind, DiskProbe, DocumentEncoding,
    LARGE_FILE_THRESHOLD, ListOptions, MAX_DOCUMENT_BYTES, WorkspaceError, WorkspacePath,
    WorkspaceRoot, decode_text, detect_content, is_large_file, is_temp_file_name, list_directory,
    probe_file, read_file, write_file_atomic,
};

fn repo_relative(parts: &[&str]) -> PathBuf {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    let mut path = gritty_workspace::canonicalize(&repo).expect("repository root exists");
    path.extend(parts);
    path
}

/// The staged fixture, created once per test binary.
fn staged() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let committed = repo_relative(&["fixtures", WEIRD_FILES_DIR]);
        let dest = repo_relative(&["target", "fixtures", WEIRD_FILES_DIR]);
        let summary = stage_weird_files(&committed, &dest).expect("stage weird-files");
        assert!(summary.copied_files >= 20, "{summary:?}");
        assert_eq!(
            summary.large_text_bytes,
            fs::metadata(dest.join(WEIRD_LARGE_TEXT)).unwrap().len()
        );
        dest
    })
}

fn weird() -> WorkspaceRoot {
    WorkspaceRoot::open(staged()).expect("open staged weird-files")
}

fn wp(raw: &str) -> WorkspacePath {
    WorkspacePath::parse(raw).unwrap_or_else(|e| panic!("{raw:?}: {e}"))
}

fn read(root: &WorkspaceRoot, raw: &str) -> Result<gritty_workspace::FileSnapshot, WorkspaceError> {
    read_file(root, &wp(raw), MAX_DOCUMENT_BYTES)
}

/// `true` when this process bypasses permission bits (root), which makes the
/// denied-file assertions meaningless.
fn running_privileged() -> bool {
    fs::read(staged().join(WEIRD_DENIED)).is_ok()
}

struct TempTree {
    path: PathBuf,
}

impl TempTree {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path =
            std::env::temp_dir().join(format!("gritty-doc-{label}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self {
            path: fs::canonicalize(&path).unwrap(),
        }
    }

    fn file(&self, relative: &str, content: &[u8]) -> PathBuf {
        let path = self.path.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, content).unwrap();
        path
    }

    fn root(&self) -> WorkspaceRoot {
        WorkspaceRoot::open(&self.path).expect("open temp root")
    }

    fn entries(&self, relative: &str) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.path.join(relative))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn temp_files(&self, relative: &str) -> Vec<String> {
        self.entries(relative)
            .into_iter()
            .filter(|name| is_temp_file_name(name))
            .collect()
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn staged_fixture_lists_every_category() {
    let root = weird();
    let page = list_directory(
        &root,
        &WorkspacePath::root(),
        &ListOptions::default(),
        &|| false,
    )
    .unwrap()
    .page;
    let names: Vec<&str> = page.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        [
            ".hidden-dir",
            "binary",
            "deep",
            "encoding",
            "generated",
            "links",
            "long-lines",
            "spaces",
            "unicode",
            ".hidden-file",
            "empty.txt",
            "no-trailing-newline.txt",
            "readme.md",
        ]
    );
    let links = list_directory(&root, &wp("links"), &ListOptions::default(), &|| false)
        .unwrap()
        .page;
    for entry in &links.entries {
        assert_eq!(entry.kind, DirEntryKind::Symlink, "{}", entry.name);
    }
    let broken = links.entries.iter().find(|e| e.name == "broken").unwrap();
    assert!(broken.symlink.as_ref().unwrap().broken);
    let to_unicode = links
        .entries
        .iter()
        .find(|e| e.name == "to-unicode")
        .unwrap();
    assert_eq!(
        to_unicode.symlink.as_ref().unwrap().target_kind,
        Some(DirEntryKind::Dir)
    );
    assert_eq!(links.entries.len(), WEIRD_LINKS.len());
}

#[test]
fn text_files_decode_with_their_encoding_and_line_endings_intact() {
    let root = weird();
    let cases: &[(&str, &str, DocumentEncoding)] = &[
        ("readme.md", "# weird-files", DocumentEncoding::Utf8),
        (
            "unicode/日本語.md",
            "こんにちは世界",
            DocumentEncoding::Utf8,
        ),
        (
            "unicode/héllo wörld.txt",
            "héllo wörld",
            DocumentEncoding::Utf8,
        ),
        ("unicode/emoji 🎉.txt", "🎉", DocumentEncoding::Utf8),
        (
            "spaces/file with spaces.txt",
            "spaces in the name",
            DocumentEncoding::Utf8,
        ),
        (
            "spaces/two  spaces.txt",
            "two consecutive",
            DocumentEncoding::Utf8,
        ),
        (".hidden-file", "hidden", DocumentEncoding::Utf8),
        (".hidden-dir/inside.txt", "inside", DocumentEncoding::Utf8),
        (
            "deep/a/b/c/d/e/f/g/h/i/j/leaf.txt",
            "deep leaf",
            DocumentEncoding::Utf8,
        ),
        (
            "encoding/utf8-bom.txt",
            "bom text",
            DocumentEncoding::Utf8Bom,
        ),
        (
            "encoding/crlf.txt",
            "line one\r\nline two\r\n",
            DocumentEncoding::Utf8,
        ),
        (
            "encoding/mixed-eol.txt",
            "a\nb\r\nc\rd\n",
            DocumentEncoding::Utf8,
        ),
        (
            "no-trailing-newline.txt",
            "last line without a newline",
            DocumentEncoding::Utf8,
        ),
    ];
    for (path, needle, encoding) in cases {
        let snapshot = read(&root, path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let (text, detected) =
            decode_text(&snapshot.bytes).unwrap_or_else(|| panic!("{path} is text"));
        assert_eq!(detected, *encoding, "{path}");
        assert!(text.contains(needle), "{path}: {text:?}");
        assert!(
            !text.starts_with('\u{feff}'),
            "{path}: BOM leaked into the text"
        );
        assert_eq!(snapshot.hash, ContentHash::of(&snapshot.bytes));
        assert_eq!(snapshot.stat.size, snapshot.bytes.len() as u64);
        assert!(!snapshot.stat.read_only);
    }
    let empty = read(&root, "empty.txt").unwrap();
    assert_eq!(
        decode_text(&empty.bytes),
        Some((String::new(), DocumentEncoding::Utf8))
    );
    let long = read(&root, "long-lines/one-line-100k.txt").unwrap();
    assert_eq!(long.bytes.len(), 100_000);
    assert!(!long.bytes.contains(&b'\n'));
}

#[test]
fn binary_and_invalid_utf8_never_produce_text() {
    let root = weird();
    for path in [
        "binary/pixel.png",
        "binary/nul.bin",
        "encoding/invalid-utf8.txt",
        "encoding/utf16le.txt",
    ] {
        let snapshot = read(&root, path).unwrap_or_else(|e| panic!("{path}: {e}"));
        assert!(
            matches!(detect_content(&snapshot.bytes), ContentKind::Binary { .. }),
            "{path}"
        );
        assert_eq!(decode_text(&snapshot.bytes), None, "{path}");
        assert_eq!(
            detect_content(&snapshot.bytes).encoding(),
            DocumentEncoding::Binary
        );
    }
    // Reading twice yields identical bytes: nothing normalizes them.
    let a = read(&root, "encoding/invalid-utf8.txt").unwrap();
    let b = read(&root, "encoding/invalid-utf8.txt").unwrap();
    assert_eq!(a.bytes, b.bytes);
    assert_eq!(a.bytes, b"caf\xe9 cr\xe8me br\xfbl\xe9e\n");
}

#[test]
fn large_text_is_flagged_and_the_read_bound_is_enforced() {
    let root = weird();
    let snapshot = read(&root, WEIRD_LARGE_TEXT).unwrap();
    assert!(snapshot.stat.size >= WEIRD_LARGE_TEXT_BYTES);
    assert!(is_large_file(snapshot.stat.size));
    assert!(snapshot.stat.size >= LARGE_FILE_THRESHOLD);
    assert!(matches!(
        detect_content(&snapshot.bytes),
        ContentKind::Text { bom: false }
    ));

    let error = read_file(&root, &wp(WEIRD_LARGE_TEXT), 1024).unwrap_err();
    assert!(
        matches!(error, WorkspaceError::TooLarge { limit: 1024, .. }),
        "{error}"
    );
    // A probe never fails on size: it reports the file as present but unhashed.
    let probe = probe_file(&root, &wp(WEIRD_LARGE_TEXT), 1024).unwrap();
    assert!(
        matches!(probe, DiskProbe::Present { hash: None, .. }),
        "{probe:?}"
    );
}

#[cfg(unix)]
#[test]
fn permission_denied_file_fails_cleanly() {
    let root = weird();
    if running_privileged() {
        eprintln!("running with elevated permissions; skipping the denied-file check");
        return;
    }
    let error = read(&root, WEIRD_DENIED).unwrap_err();
    assert!(
        matches!(error, WorkspaceError::PermissionDenied(_)),
        "{error}"
    );
    let error = probe_file(&root, &wp(WEIRD_DENIED), MAX_DOCUMENT_BYTES).unwrap_err();
    assert!(
        matches!(error, WorkspaceError::PermissionDenied(_)),
        "{error}"
    );
    let snapshot = fs::metadata(staged().join(WEIRD_DENIED)).unwrap();
    assert!(snapshot.permissions().readonly());
}

#[test]
fn directories_missing_paths_and_the_root_are_rejected() {
    let root = weird();
    let error = read(&root, "unicode").unwrap_err();
    assert!(matches!(error, WorkspaceError::NotAFile(_)), "{error}");
    let error = read(&root, "").unwrap_err();
    assert!(matches!(error, WorkspaceError::NotAFile(_)), "{error}");
    let error = read(&root, "does/not/exist.txt").unwrap_err();
    assert!(matches!(error, WorkspaceError::NotFound(_)), "{error}");
    assert_eq!(
        probe_file(&root, &wp("does/not/exist.txt"), MAX_DOCUMENT_BYTES).unwrap(),
        DiskProbe::Missing
    );
    let error = probe_file(&root, &wp("unicode"), MAX_DOCUMENT_BYTES).unwrap_err();
    assert!(matches!(error, WorkspaceError::NotAFile(_)), "{error}");
}

#[test]
fn symlinks_inside_the_root_are_followed_and_others_are_not() {
    let root = weird();
    if !staged()
        .join("links/to-readme")
        .symlink_metadata()
        .unwrap()
        .file_type()
        .is_symlink()
    {
        eprintln!("no symlinks on this checkout; skipping");
        return;
    }
    let through_link = read(&root, "links/to-readme").unwrap();
    let direct = read(&root, "readme.md").unwrap();
    assert_eq!(through_link.bytes, direct.bytes);
    assert_eq!(through_link.hash, direct.hash);
    let via_dir_link = read(&root, "links/to-unicode/日本語.md").unwrap();
    assert!(
        decode_text(&via_dir_link.bytes)
            .unwrap()
            .0
            .contains("日本語")
    );

    let error = read(&root, "links/broken").unwrap_err();
    assert!(matches!(error, WorkspaceError::NotFound(_)), "{error}");
    assert_eq!(
        probe_file(&root, &wp("links/broken"), MAX_DOCUMENT_BYTES).unwrap(),
        DiskProbe::Missing
    );

    // A link that escapes the root is refused before anything is read.
    let tree = TempTree::new("escape");
    let outside = TempTree::new("outside");
    let secret = outside.file("secret.txt", b"secret");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&secret, tree.path.join("escape.txt")).unwrap();
    #[cfg(windows)]
    if std::os::windows::fs::symlink_file(&secret, tree.path.join("escape.txt")).is_err() {
        return;
    }
    let error = read(&tree.root(), "escape.txt").unwrap_err();
    assert!(
        matches!(error, WorkspaceError::PermissionDenied(_)),
        "{error}"
    );
    let error = probe_file(&tree.root(), &wp("escape.txt"), MAX_DOCUMENT_BYTES).unwrap_err();
    assert!(
        matches!(error, WorkspaceError::PermissionDenied(_)),
        "{error}"
    );
    let error = tree
        .root()
        .resolve_for_write(&wp("escape.txt"))
        .unwrap_err();
    assert!(
        matches!(error, WorkspaceError::PermissionDenied(_)),
        "{error}"
    );
}

#[test]
fn atomic_write_replaces_content_and_leaves_no_temp_file() {
    let tree = TempTree::new("write");
    let target = tree.file("notes.md", b"before\n");
    let stat = write_file_atomic(&target, b"after\r\n", AtomicWriteFault::None).unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"after\r\n");
    assert_eq!(stat.size, 7);
    assert!(!stat.read_only);
    assert_eq!(tree.entries(""), ["notes.md"]);
    assert!(tree.temp_files("").is_empty());

    let root = tree.root();
    let probe = probe_file(&root, &wp("notes.md"), MAX_DOCUMENT_BYTES).unwrap();
    assert_eq!(
        probe,
        DiskProbe::Present {
            stat,
            hash: Some(ContentHash::of(b"after\r\n"))
        }
    );
}

#[cfg(unix)]
#[test]
fn atomic_write_preserves_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let tree = TempTree::new("perms");
    let target = tree.file("script.sh", b"#!/bin/sh\n");
    fs::set_permissions(&target, fs::Permissions::from_mode(0o750)).unwrap();
    write_file_atomic(&target, b"#!/bin/sh\necho hi\n", AtomicWriteFault::None).unwrap();
    let mode = fs::metadata(&target).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o750);

    // A read-only file is still replaceable (directory permission decides) and
    // stays read-only afterwards.
    let target = tree.file("locked.txt", b"v1");
    fs::set_permissions(&target, fs::Permissions::from_mode(0o444)).unwrap();
    let stat = write_file_atomic(&target, b"v2", AtomicWriteFault::None).unwrap();
    assert!(stat.read_only);
    assert_eq!(fs::read(&target).unwrap(), b"v2");
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o444
    );
}

#[test]
fn injected_failure_before_rename_keeps_the_destination_and_cleans_up() {
    let tree = TempTree::new("fail");
    let target = tree.file("data.json", b"{\"v\":1}");
    let error =
        write_file_atomic(&target, b"{\"v\":2}", AtomicWriteFault::FailBeforeRename).unwrap_err();
    assert!(matches!(error, WorkspaceError::Io { .. }), "{error}");
    assert_eq!(fs::read(&target).unwrap(), b"{\"v\":1}");
    assert_eq!(
        tree.entries(""),
        ["data.json"],
        "temporary file was removed"
    );
}

#[test]
fn simulated_crash_before_rename_keeps_the_destination_intact() {
    let tree = TempTree::new("crash");
    let target = tree.file("data.json", b"{\"v\":1}");
    let error =
        write_file_atomic(&target, b"{\"v\":2}", AtomicWriteFault::CrashBeforeRename).unwrap_err();
    assert!(matches!(error, WorkspaceError::Io { .. }), "{error}");
    assert_eq!(
        fs::read(&target).unwrap(),
        b"{\"v\":1}",
        "destination untouched"
    );
    let leftovers = tree.temp_files("");
    assert_eq!(
        leftovers.len(),
        1,
        "a crash leaves exactly the temp file: {leftovers:?}"
    );
    assert!(leftovers[0].starts_with(".data.json.gritty-"));
    let leftover = tree.path.join(&leftovers[0]);
    assert_eq!(
        fs::read(&leftover).unwrap(),
        b"{\"v\":2}",
        "the flushed content is recoverable"
    );

    // The next save is unaffected by the leftover and succeeds.
    write_file_atomic(&target, b"{\"v\":3}", AtomicWriteFault::None).unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"{\"v\":3}");
    assert_eq!(tree.temp_files("").len(), 1);
}

#[test]
fn write_into_a_vanished_directory_fails_without_leftovers() {
    let tree = TempTree::new("vanished");
    let target = tree.path.join("gone").join("file.txt");
    let error = write_file_atomic(&target, b"x", AtomicWriteFault::None).unwrap_err();
    assert!(matches!(error, WorkspaceError::NotFound(_)), "{error}");
    assert!(tree.entries("").is_empty());

    // Writing over a directory is refused.
    fs::create_dir(tree.path.join("dir")).unwrap();
    let error =
        write_file_atomic(&tree.path.join("dir"), b"x", AtomicWriteFault::None).unwrap_err();
    assert!(matches!(error, WorkspaceError::NotAFile(_)), "{error}");
    assert!(tree.temp_files("").is_empty());
    let error = write_file_atomic(Path::new(""), b"x", AtomicWriteFault::None).unwrap_err();
    assert!(
        matches!(error, WorkspaceError::InvalidPath { .. }),
        "{error}"
    );
}

#[test]
fn resolve_for_write_creates_missing_files_inside_existing_directories_only() {
    let tree = TempTree::new("resolve");
    tree.file("src/lib.rs", b"");
    let root = tree.root();

    let existing = root.resolve_for_write(&wp("src/lib.rs")).unwrap();
    assert_eq!(existing.canonical(), tree.path.join("src/lib.rs"));

    let fresh = root.resolve_for_write(&wp("src/new.rs")).unwrap();
    assert_eq!(fresh.canonical(), tree.path.join("src/new.rs"));
    assert_eq!(fresh.native(), tree.path.join("src/new.rs"));
    write_file_atomic(
        fresh.canonical(),
        b"pub fn new() {}\n",
        AtomicWriteFault::None,
    )
    .unwrap();
    assert_eq!(
        read(&root, "src/new.rs").unwrap().bytes,
        b"pub fn new() {}\n"
    );

    let error = root
        .resolve_for_write(&wp("missing-dir/file.rs"))
        .unwrap_err();
    assert!(matches!(error, WorkspaceError::NotFound(_)), "{error}");
    let error = root.resolve_for_write(&WorkspacePath::root()).unwrap_err();
    assert!(
        matches!(error, WorkspaceError::InvalidPath { .. }),
        "{error}"
    );

    // Writing through a symlinked directory inside the root lands on the real
    // file, so the link survives.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(tree.path.join("src"), tree.path.join("link-src")).unwrap();
        let through = root.resolve_for_write(&wp("link-src/lib.rs")).unwrap();
        assert_eq!(through.canonical(), tree.path.join("src/lib.rs"));
        write_file_atomic(through.canonical(), b"via link\n", AtomicWriteFault::None).unwrap();
        assert_eq!(
            fs::read(tree.path.join("src/lib.rs")).unwrap(),
            b"via link\n"
        );
        assert!(
            tree.path
                .join("link-src")
                .symlink_metadata()
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }
}

#[test]
fn probe_reports_missing_present_and_changed_content() {
    let tree = TempTree::new("probe");
    let root = tree.root();
    let path = wp("a.txt");
    assert_eq!(
        probe_file(&root, &path, MAX_DOCUMENT_BYTES).unwrap(),
        DiskProbe::Missing
    );
    tree.file("a.txt", b"one");
    let DiskProbe::Present {
        hash: Some(first),
        stat,
    } = probe_file(&root, &path, MAX_DOCUMENT_BYTES).unwrap()
    else {
        panic!("expected present")
    };
    assert_eq!(first, ContentHash::of(b"one"));
    assert_eq!(stat.size, 3);
    // Same size, different bytes: only the hash tells (mtime granularity is
    // not relied on).
    fs::write(tree.path.join("a.txt"), b"two").unwrap();
    let DiskProbe::Present {
        hash: Some(second), ..
    } = probe_file(&root, &path, MAX_DOCUMENT_BYTES).unwrap()
    else {
        panic!("expected present")
    };
    assert_ne!(first, second);
    fs::remove_file(tree.path.join("a.txt")).unwrap();
    assert_eq!(
        probe_file(&root, &path, MAX_DOCUMENT_BYTES).unwrap(),
        DiskProbe::Missing
    );
}
