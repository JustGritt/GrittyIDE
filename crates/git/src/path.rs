//! Turning Git's byte paths into the protocol's [`WorkspacePath`].
//!
//! Git stores paths as bytes, `/`-separated and relative to the repository
//! root. The protocol (ADR-010) needs UTF-8, `/`-separated, relative, no `.`
//! or `..` segments and **no backslashes** — the last one so the same string
//! means the same thing on Windows as on Unix.
//!
//! Decoding follows the workspace's policy for filenames exactly: lossy UTF-8,
//! passed through, no normalization and no case folding
//! (`docs/architecture/security.md`). A name that is not valid UTF-8 can then
//! be *shown* but, having lost bytes to `U+FFFD`, not addressed — which is the
//! same bargain the file explorer already makes.
//!
//! The one shape that has no representation at all is a filename containing a
//! backslash, which Unix allows and Windows does not. Those are counted, not
//! hidden: see [`RepositoryStatus::unrepresentable`](crate::RepositoryStatus).

use gritty_workspace::WorkspacePath;

/// Decode a Git path, or `None` when it has no protocol representation.
pub(crate) fn decode(raw: &[u8]) -> Option<WorkspacePath> {
    let lossy = String::from_utf8_lossy(raw);
    // Git never stores a trailing slash, but the directory walk reports
    // collapsed untracked directories with one. Trim it rather than rejecting
    // the entry: the trailing slash is the walk's notation, not part of the
    // name.
    let trimmed = lossy.trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    WorkspacePath::parse(trimmed).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_an_ordinary_path() {
        let path = decode(b"src/main.rs").expect("representable");
        assert_eq!(path.to_string(), "src/main.rs");
    }

    #[test]
    fn trims_the_trailing_slash_a_collapsed_directory_carries() {
        let path = decode(b"vendor/").expect("representable");
        assert_eq!(path.to_string(), "vendor");
    }

    #[test]
    fn decodes_invalid_utf8_lossily_rather_than_dropping_the_entry() {
        let path = decode(b"src/caf\xffe.rs").expect("representable");
        assert!(path.to_string().contains('\u{fffd}'));
    }

    #[test]
    fn rejects_a_name_with_no_protocol_representation() {
        // Legal on Unix, unaddressable on Windows, so unaddressable here.
        assert!(decode(br"src/we\ird.rs").is_none());
        assert!(decode(b"").is_none());
        assert!(decode(b"/etc/passwd").is_none());
    }
}
