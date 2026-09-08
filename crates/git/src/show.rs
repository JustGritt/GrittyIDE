//! `git.show`: the content of one path at one revision.
//!
//! This is the *original* side of a diff. `git.diff` returns hunks with
//! bounded context, and bounded context cannot reconstruct a whole file, so a
//! side-by-side view of a change needs the file itself at the revision it is
//! compared against.
//!
//! ## Why the decoding is `gritty_workspace`'s and not this crate's
//!
//! [`gritty_workspace::decode_text`] decides what "binary" means for a file on
//! disk: a NUL byte or invalid UTF-8, with the byte order mark stripped from
//! the text and recorded in the encoding. A blob at a revision is the same
//! bytes one commit earlier, and if the two layers disagreed about binaryness
//! the diff view would show text on one side and refuse it on the other for the
//! same file. So the same function decides both, rather than a second
//! implementation that starts identical and drifts.
//!
//! Note that this is a *decode*, never a checkout: no clean or smudge filter
//! runs, because none is loaded (ADR-023 decision 2 and `open.rs`). The bytes
//! are the blob's own, which is what a diff wants — Git compares blobs, not
//! filtered worktree content.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gritty_workspace::{DocumentEncoding, WorkspacePath, decode_text};

use crate::error::GitError;
use crate::model::{BlobContent, MAX_SHOW_BYTES, NoTextReason};
use crate::open::Repository;

/// What `git.show` should read.
///
/// No `Default`: there is no sensible default path, and a blob read with an
/// empty one would be a bug that compiled.
#[derive(Debug, Clone)]
pub struct ShowOptions {
    pub path: WorkspacePath,
    /// Revision to read the path at; `HEAD` when absent.
    pub revision: Option<String>,
    /// Blobs above this many bytes come back with no text. Defaults to
    /// [`MAX_SHOW_BYTES`].
    pub max_bytes: Option<u64>,
}

/// Read `options.path` as it was at `options.revision`.
///
/// A path that is absent at that revision is [`GitError::NotFoundAtRevision`],
/// not an empty file: "this did not exist yet" and "this was empty" are
/// different answers and a diff renders them differently.
pub fn show(
    repo: &Repository,
    options: &ShowOptions,
    interrupt: &Arc<AtomicBool>,
) -> Result<BlobContent, GitError> {
    let local = repo.to_thread_local();
    let spec = options
        .revision
        .clone()
        .unwrap_or_else(|| "HEAD".to_owned());
    let max_bytes = options.max_bytes.unwrap_or(MAX_SHOW_BYTES);

    if interrupt.load(Ordering::Relaxed) {
        return Err(GitError::Cancelled);
    }

    let id = local
        .rev_parse_single(spec.as_str())
        .map_err(|_| GitError::UnknownRevision { spec: spec.clone() })?;
    let object = id.object().map_err(|error| {
        GitError::repository(format!("could not read the object {spec} names"), error)
    })?;
    let resolved = object.id;
    let tree = object
        .peel_to_tree()
        .map_err(|_| GitError::NotATree { spec: spec.clone() })?;

    let entry = tree
        .lookup_entry_by_path(options.path.as_str())
        .map_err(|error| GitError::repository("could not walk the tree", error))?
        .ok_or_else(|| GitError::NotFoundAtRevision {
            path: options.path.to_string(),
            spec: spec.clone(),
        })?;
    let blob_id = entry.oid().to_owned();

    if interrupt.load(Ordering::Relaxed) {
        return Err(GitError::Cancelled);
    }

    let blob = local
        .find_object(blob_id)
        .map_err(|error| GitError::repository("could not read a blob", error))?;
    let bytes = blob.data.as_slice();
    let size = bytes.len() as u64;

    // The size is checked before the decode so an oversized blob is never
    // copied into a `String` on its way to being refused.
    let (text, no_text, encoding) = if size > max_bytes {
        (None, Some(NoTextReason::TooLarge), DocumentEncoding::Binary)
    } else {
        match decode_text(bytes) {
            Some((text, encoding)) => (Some(text), None, encoding),
            None => (None, Some(NoTextReason::Binary), DocumentEncoding::Binary),
        }
    };

    // Ids and counts only; never the content, never the path (`AGENTS.md`
    // rule 13).
    tracing::debug!(size, text = text.is_some(), "git show");

    Ok(BlobContent {
        path: options.path.clone(),
        revision: spec,
        resolved: resolved.to_string(),
        blob: blob_id.to_string(),
        size,
        text,
        no_text,
        encoding,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use super::*;
    use crate::fixtures::{self, TestRepo};
    use crate::open::open;

    fn options(path: &str) -> ShowOptions {
        ShowOptions {
            path: WorkspacePath::parse(path).expect("a valid workspace path"),
            revision: None,
            max_bytes: None,
        }
    }

    fn read(repo: &TestRepo, options: &ShowOptions) -> BlobContent {
        let opened = open(repo.path()).expect("open");
        show(&opened, options, &Arc::new(AtomicBool::new(false))).expect("show")
    }

    #[test]
    fn reads_a_file_at_head() {
        let repo = fixtures::history();
        let blob = read(&repo, &options("src/main.rs"));

        assert_eq!(blob.text.as_deref(), Some("fn main() { println!(); }\n"));
        assert_eq!(blob.no_text, None);
        assert_eq!(blob.encoding, DocumentEncoding::Utf8);
        assert_eq!(blob.size, "fn main() { println!(); }\n".len() as u64);
        assert_eq!(blob.revision, "HEAD");
        assert_eq!(blob.resolved, repo.head_id());
    }

    #[test]
    fn reads_the_same_path_at_an_earlier_revision() {
        let repo = fixtures::history();
        // The content before the fourth commit changed it — which is the whole
        // point of this command: the original side of a diff.
        let earlier = read(
            &repo,
            &ShowOptions {
                revision: Some("HEAD~1".to_owned()),
                ..options("src/main.rs")
            },
        );
        assert_eq!(earlier.text.as_deref(), Some("fn main() {}\n"));

        let head = read(&repo, &options("src/main.rs"));
        assert_ne!(earlier.blob, head.blob);
        assert_ne!(earlier.resolved, head.resolved);
    }

    #[test]
    fn a_binary_blob_comes_back_labelled_rather_than_as_an_error() {
        let repo = fixtures::history();
        let blob = read(&repo, &options("logo.bin"));

        assert_eq!(blob.text, None);
        assert_eq!(blob.no_text, Some(NoTextReason::Binary));
        assert_eq!(blob.encoding, DocumentEncoding::Binary);
        // The size is still reported: a caller that gets no text still learns
        // how big the thing was.
        assert_eq!(blob.size, 6);
    }

    #[test]
    fn a_blob_over_the_cap_is_refused_by_size_and_not_decoded() {
        let repo = TestRepo::init();
        repo.write("big.txt", "x".repeat(2_000));
        repo.commit_all("big");

        let blob = read(
            &repo,
            &ShowOptions {
                max_bytes: Some(1_000),
                ..options("big.txt")
            },
        );
        assert_eq!(blob.text, None);
        assert_eq!(blob.no_text, Some(NoTextReason::TooLarge));
        assert_eq!(blob.size, 2_000);
    }

    #[test]
    fn a_path_absent_at_that_revision_is_not_an_empty_file() {
        let repo = fixtures::history();
        let opened = open(repo.path()).expect("open");
        // `src/main.rs` was added by the second commit, so it is not in the
        // first — and "did not exist yet" must not read as "was empty".
        let error = show(
            &opened,
            &ShowOptions {
                revision: Some("HEAD~3".to_owned()),
                ..options("src/main.rs")
            },
            &Arc::new(AtomicBool::new(false)),
        )
        .expect_err("the path is not in that revision");
        assert!(matches!(error, GitError::NotFoundAtRevision { .. }));
    }

    #[test]
    fn an_unknown_revision_is_the_callers_fault() {
        let repo = fixtures::history();
        let opened = open(repo.path()).expect("open");
        let error = show(
            &opened,
            &ShowOptions {
                revision: Some("no-such-ref".to_owned()),
                ..options("src/main.rs")
            },
            &Arc::new(AtomicBool::new(false)),
        )
        .expect_err("an unknown revision cannot be read");
        assert!(matches!(error, GitError::UnknownRevision { .. }));
        assert!(error.is_caller_fault());
    }

    #[test]
    fn a_byte_order_mark_is_stripped_from_the_text_and_kept_in_the_encoding() {
        let repo = TestRepo::init();
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"hello\n");
        repo.write("bom.txt", bytes);
        repo.commit_all("bom");

        let blob = read(&repo, &options("bom.txt"));
        // The same bargain `documents.open` makes, because it is the same
        // decoder — a diff must not show a stray mark on one side only.
        assert_eq!(blob.text.as_deref(), Some("hello\n"));
        assert_eq!(blob.encoding, DocumentEncoding::Utf8Bom);
    }

    #[test]
    fn cancelling_before_the_read_returns_nothing_partial() {
        let repo = fixtures::history();
        let opened = open(repo.path()).expect("open");
        let interrupt = Arc::new(AtomicBool::new(true));
        let error = show(&opened, &options("src/main.rs"), &interrupt).expect_err("cancelled");
        assert!(matches!(error, GitError::Cancelled));
    }
}
