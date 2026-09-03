//! Integration tests: the committed `fixtures/small-repo`, a synthetic tree
//! with symlinks and unusual names, and the generated `large-repo`
//! (written once per test binary under `target/fixtures/large-repo`).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use gritty_workspace::fixtures::{
    BULK_DIRECTORY, BULK_ENTRIES, DEEP_DIRECTORY, DEEP_LEVELS, LARGE_REPO_ROOT_ENTRIES,
    generate_large_repo, large_repo_is_current,
};
use gritty_workspace::{
    DirEntryKind, DirectoryPage, ListOptions, MAX_PAGE_SIZE, WorkspaceError, WorkspacePath,
    WorkspaceRoot, list_directory,
};

fn never() -> bool {
    false
}

/// Absolute, `..`-free path under the repository root. On Windows a path with
/// `..` segments stays subject to the 260-character limit, and the deep
/// fixture chain exceeds it.
fn repo_relative(parts: &[&str]) -> PathBuf {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    let mut path = gritty_workspace::canonicalize(&repo).expect("repository root exists");
    path.extend(parts);
    path
}

fn small_repo() -> WorkspaceRoot {
    WorkspaceRoot::open(&repo_relative(&["fixtures", "small-repo"])).expect("open small-repo")
}

fn large_repo() -> WorkspaceRoot {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    let dir = DIR.get_or_init(|| {
        let dir = repo_relative(&["target", "fixtures", "large-repo"]);
        if !large_repo_is_current(&dir) {
            let summary = generate_large_repo(&dir).expect("generate large-repo");
            assert!(summary.files >= 10_000, "{summary:?}");
        }
        dir
    });
    WorkspaceRoot::open(dir).expect("open large-repo")
}

fn list(root: &WorkspaceRoot, path: &str) -> DirectoryPage {
    list_with(root, path, ListOptions::default()).unwrap_or_else(|e| panic!("list {path:?}: {e}"))
}

fn list_with(
    root: &WorkspaceRoot,
    path: &str,
    options: ListOptions,
) -> Result<DirectoryPage, WorkspaceError> {
    let path = WorkspacePath::parse(path)?;
    list_directory(root, &path, &options, &never).map(|listing| listing.page)
}

fn names(page: &DirectoryPage) -> Vec<&str> {
    page.entries.iter().map(|e| e.name.as_str()).collect()
}

/// A scratch directory removed on drop.
struct TempTree {
    path: PathBuf,
}

impl TempTree {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path =
            std::env::temp_dir().join(format!("gritty-ws-{label}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn file(&self, relative: &str) {
        let path = self.path.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, relative).unwrap();
    }

    fn dir(&self, relative: &str) {
        fs::create_dir_all(self.path.join(relative)).unwrap();
    }

    /// `false` when the platform refuses to create symlinks (Windows without
    /// Developer Mode or elevation); callers then skip symlink assertions.
    fn symlink(&self, link: &str, target: &Path, target_is_dir: bool) -> bool {
        let link = self.path.join(link);
        #[cfg(unix)]
        let result = {
            let _ = target_is_dir;
            std::os::unix::fs::symlink(target, &link)
        };
        #[cfg(windows)]
        let result = if target_is_dir {
            std::os::windows::fs::symlink_dir(target, &link)
        } else {
            std::os::windows::fs::symlink_file(target, &link)
        };
        match result {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                eprintln!("symlink creation not permitted here, skipping: {error}");
                false
            }
            Err(error) => panic!("symlink {}: {error}", link.display()),
        }
    }

    fn root(&self) -> WorkspaceRoot {
        WorkspaceRoot::open(&self.path).expect("open temp root")
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn small_repo_root_is_sorted_directories_first_then_by_name() {
    let root = small_repo();
    let page = list(&root, "");
    assert_eq!(
        names(&page),
        [
            ".hidden",
            "crates",
            "data",
            "docs",
            "src",
            ".gitignore",
            "Cargo.toml",
            "README.md",
            "file with spaces.txt",
            "link-to-src",
            "package.json",
            "tsconfig.json",
        ]
    );
    assert_eq!(page.total, 12);
    assert!(!page.truncated);
    assert!(page.cursor.is_none());
    for entry in &page.entries[..5] {
        assert_eq!(entry.kind, DirEntryKind::Dir, "{}", entry.name);
        assert!(entry.size.is_none());
    }
    for entry in &page.entries[5..] {
        // `link-to-src` is a symlink where git could create one and a plain
        // file holding the link text where it could not (core.symlinks=false).
        let expected = if entry.name == "link-to-src" {
            [DirEntryKind::Symlink, DirEntryKind::File]
        } else {
            [DirEntryKind::File, DirEntryKind::File]
        };
        assert!(expected.contains(&entry.kind), "{entry:?}");
        if entry.kind == DirEntryKind::File {
            assert!(entry.size.is_some(), "{entry:?}");
        }
        assert!(entry.modified_at.is_some(), "{entry:?}");
        assert!(!entry.ignored_hint);
    }
    let hidden: Vec<&str> = page
        .entries
        .iter()
        .filter(|e| e.hidden)
        .map(|e| e.name.as_str())
        .collect();
    assert_eq!(hidden, [".hidden", ".gitignore"]);
}

#[test]
fn small_repo_lists_exactly_one_level_per_call() {
    let root = small_repo();
    assert_eq!(names(&list(&root, "src")), ["lib", "index.ts"]);
    assert_eq!(names(&list(&root, "src/lib")), ["nested", "util.ts"]);
    assert_eq!(names(&list(&root, "src/lib/nested/deep")), ["leaf.ts"]);
    assert_eq!(names(&list(&root, ".hidden")), ["notes.md"]);
    assert_eq!(names(&list(&root, "crates/demo")), ["src", "Cargo.toml"]);
    assert!(matches!(
        list_with(&root, "src/index.ts", ListOptions::default()),
        Err(WorkspaceError::NotADirectory(_))
    ));
    assert!(matches!(
        list_with(&root, "does-not-exist", ListOptions::default()),
        Err(WorkspaceError::NotFound(_))
    ));
    assert!(matches!(
        list_with(&root, "src/missing/deeper", ListOptions::default()),
        Err(WorkspaceError::NotFound(_))
    ));
}

#[test]
fn ignored_hint_marks_well_known_heavy_directories_without_filtering() {
    let tree = TempTree::new("ignored");
    for dir in [".git", "node_modules", "target", "dist", "src", "build"] {
        tree.dir(dir);
    }
    tree.file("node_modules.txt");
    tree.file("target.md");
    let page = list(&tree.root(), "");
    let hinted: Vec<&str> = page
        .entries
        .iter()
        .filter(|e| e.ignored_hint)
        .map(|e| e.name.as_str())
        .collect();
    assert_eq!(hinted, [".git", "build", "dist", "node_modules", "target"]);
    assert_eq!(page.total, 8, "nothing is filtered out");
    let src = page.entries.iter().find(|e| e.name == "src").unwrap();
    assert!(!src.ignored_hint);
}

#[test]
fn symlinks_report_inside_outside_and_broken_targets() {
    let tree = TempTree::new("symlinks");
    tree.dir("src");
    tree.file("src/main.rs");
    tree.file("README.md");
    let outside = std::env::temp_dir();
    if !tree.symlink("link-dir", &tree.path.join("src"), true) {
        return;
    }
    assert!(tree.symlink("link-file", &tree.path.join("README.md"), false));
    assert!(tree.symlink("link-out", &outside, true));
    assert!(tree.symlink("link-broken", &tree.path.join("missing.txt"), false));

    let root = tree.root();
    let page = list(&root, "");
    let entry = |name: &str| page.entries.iter().find(|e| e.name == name).unwrap();
    for name in ["link-dir", "link-file", "link-out", "link-broken"] {
        assert_eq!(entry(name).kind, DirEntryKind::Symlink, "{name}");
        assert!(entry(name).size.is_none(), "{name}");
    }
    let link_dir = entry("link-dir").symlink.as_ref().unwrap();
    assert!(!link_dir.broken);
    assert!(link_dir.resolves_inside_root);
    assert_eq!(link_dir.target_kind, Some(DirEntryKind::Dir));
    let link_file = entry("link-file").symlink.as_ref().unwrap();
    assert!(!link_file.broken);
    assert!(link_file.resolves_inside_root);
    assert_eq!(link_file.target_kind, Some(DirEntryKind::File));
    let link_out = entry("link-out").symlink.as_ref().unwrap();
    assert!(!link_out.broken);
    assert!(!link_out.resolves_inside_root);
    let link_broken = entry("link-broken").symlink.as_ref().unwrap();
    assert!(link_broken.broken);
    assert!(!link_broken.resolves_inside_root);
    assert_eq!(link_broken.target_kind, None);

    // A symlink that resolves inside the root can be listed through.
    assert_eq!(names(&list(&root, "link-dir")), ["main.rs"]);
    // One that escapes is refused, and so is anything beneath it.
    assert!(matches!(
        list_with(&root, "link-out", ListOptions::default()),
        Err(WorkspaceError::PermissionDenied(_))
    ));
    assert!(matches!(
        root.resolve(&WorkspacePath::parse("link-out").unwrap()),
        Err(WorkspaceError::PermissionDenied(_))
    ));
    assert!(matches!(
        list_with(&root, "link-broken", ListOptions::default()),
        Err(WorkspaceError::NotFound(_))
    ));
}

#[test]
fn path_policy_rejects_traversal_and_absolute_paths_before_touching_disk() {
    let tree = TempTree::new("policy");
    tree.file("inside.txt");
    let root = tree.root();
    for raw in [
        "..",
        "../",
        "../../etc/passwd",
        "src/../..",
        "/etc",
        "\\",
        "..\\..\\Windows",
    ] {
        assert!(
            matches!(
                list_with(&root, raw, ListOptions::default()),
                Err(WorkspaceError::InvalidPath { field: "path", .. })
            ),
            "{raw:?}"
        );
    }
    #[cfg(windows)]
    for raw in ["C:/Windows", "C:", "//server/share"] {
        assert!(
            matches!(
                list_with(&root, raw, ListOptions::default()),
                Err(WorkspaceError::InvalidPath { field: "path", .. })
            ),
            "{raw:?}"
        );
    }
    // A sibling directory whose name extends the root's is not inside it.
    let sibling = PathBuf::from(format!("{}-sibling", tree.path.display()));
    assert!(!root.contains(&sibling));
}

#[test]
fn unicode_and_spaced_names_round_trip() {
    let tree = TempTree::new("unicode");
    tree.dir("spa ced dir");
    tree.file("spa ced dir/inner file.txt");
    tree.file("héllo wörld.md");
    tree.file("日本語.txt");
    tree.file("emoji 🎉.rs");
    let root = tree.root();
    let page = list(&root, "");
    assert_eq!(
        names(&page),
        ["spa ced dir", "emoji 🎉.rs", "héllo wörld.md", "日本語.txt"]
    );
    assert_eq!(names(&list(&root, "spa ced dir")), ["inner file.txt"]);
    assert!(matches!(
        list_with(&root, "héllo wörld.md", ListOptions::default()),
        Err(WorkspaceError::NotADirectory(_))
    ));
}

#[test]
fn large_repo_root_lists_within_budget_without_recursing() {
    let root = large_repo();
    let listing = list_directory(
        &root,
        &WorkspacePath::root(),
        &ListOptions::default(),
        &never,
    )
    .unwrap();
    assert_eq!(names(&listing.page), LARGE_REPO_ROOT_ENTRIES);
    assert_eq!(listing.stats.directories_read, 1);
    assert_eq!(listing.stats.entries_seen, 12);
    assert!(
        listing.stats.elapsed < Duration::from_millis(100),
        "root listing took {:?}",
        listing.stats.elapsed
    );
    let hinted: Vec<&str> = listing
        .page
        .entries
        .iter()
        .filter(|e| e.ignored_hint)
        .map(|e| e.name.as_str())
        .collect();
    assert_eq!(hinted, [".git", "dist", "node_modules", "target"]);
}

#[test]
fn large_directory_is_truncated_and_continues_with_the_cursor() {
    let root = large_repo();
    let first = list(&root, BULK_DIRECTORY);
    assert_eq!(first.entries.len(), MAX_PAGE_SIZE);
    assert!(first.truncated);
    assert_eq!(first.total, BULK_ENTRIES as u64);
    let cursor = first.cursor.clone().expect("cursor when truncated");

    let second = list_with(
        &root,
        BULK_DIRECTORY,
        ListOptions {
            page_size: MAX_PAGE_SIZE,
            cursor: Some(cursor),
        },
    )
    .unwrap();
    assert_eq!(second.entries.len(), BULK_ENTRIES - MAX_PAGE_SIZE);
    assert!(!second.truncated);
    assert!(second.cursor.is_none());

    let mut all: Vec<String> = first
        .entries
        .iter()
        .chain(second.entries.iter())
        .map(|e| e.name.clone())
        .collect();
    let expected: Vec<String> = (0..BULK_ENTRIES)
        .map(|i| format!("entry_{i:04}.txt"))
        .collect();
    assert_eq!(all, expected, "pages are contiguous and sorted");
    all.dedup();
    assert_eq!(all.len(), BULK_ENTRIES);

    // Smaller pages walk the same directory in more steps, never repeating.
    let mut walked = Vec::new();
    let mut cursor = None;
    let mut pages = 0;
    loop {
        let page = list_with(
            &root,
            BULK_DIRECTORY,
            ListOptions {
                page_size: 1_000,
                cursor,
            },
        )
        .unwrap();
        pages += 1;
        walked.extend(page.entries.iter().map(|e| e.name.clone()));
        if !page.truncated {
            break;
        }
        cursor = page.cursor;
    }
    assert_eq!(pages, 6);
    assert_eq!(walked, expected);

    assert!(matches!(
        list_with(
            &root,
            BULK_DIRECTORY,
            ListOptions {
                page_size: 10,
                cursor: Some("garbage".into()),
            },
        ),
        Err(WorkspaceError::InvalidPath {
            field: "cursor",
            ..
        })
    ));
}

#[test]
fn cancellation_discards_partial_results() {
    let root = large_repo();
    let path = WorkspacePath::parse(BULK_DIRECTORY).unwrap();
    let checks = AtomicUsize::new(0);
    let cancel_on_third_check = || checks.fetch_add(1, Ordering::SeqCst) >= 2;
    let result = list_directory(
        &root,
        &path,
        &ListOptions::default(),
        &cancel_on_third_check,
    );
    assert!(
        matches!(result, Err(WorkspaceError::Cancelled)),
        "{result:?}"
    );
    assert!(checks.load(Ordering::SeqCst) >= 3);

    let already_cancelled = || true;
    assert!(matches!(
        list_directory(&root, &path, &ListOptions::default(), &already_cancelled),
        Err(WorkspaceError::Cancelled)
    ));
}

#[test]
fn deep_tree_is_reachable_one_level_at_a_time() {
    let root = large_repo();
    let mut path = WorkspacePath::parse(DEEP_DIRECTORY).unwrap();
    for level in 1..=DEEP_LEVELS {
        let page = list(&root, path.as_str());
        let next = format!("level_{level:02}");
        assert_eq!(page.entries[0].name, next, "at {path}");
        assert_eq!(page.entries[0].kind, DirEntryKind::Dir);
        path = path.join(&next).unwrap();
    }
    assert_eq!(names(&list(&root, path.as_str())), ["leaf.rs", "note.md"]);
}

#[test]
fn root_removed_after_open_is_reported_as_not_found() {
    let tree = TempTree::new("removed");
    tree.file("a.txt");
    let root = tree.root();
    assert!(root.is_present());
    fs::remove_dir_all(&tree.path).unwrap();
    assert!(!root.is_present());
    assert!(matches!(
        list_with(&root, "", ListOptions::default()),
        Err(WorkspaceError::NotFound(_))
    ));
}
