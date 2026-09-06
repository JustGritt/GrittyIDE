//! What a lease is *over*, and whether two of them can collide (T325).
//!
//! Three target kinds — a file, a directory subtree, a glob — reduce to **one**
//! matcher: every target compiles to a list of pattern segments, a file being
//! all literals and a directory being its own segments followed by `**`. One
//! routine to get right instead of nine pairwise cases.
//!
//! The pattern language is deliberately the one `AGENT-LEASES.md` already
//! documents for the sessions sharing this checkout, so the convention and the
//! product cannot disagree about what a scope means:
//!
//! ```text
//! *   any run of characters inside one segment; never crosses '/'
//! **  zero or more whole segments
//! ?   exactly one character inside one segment
//! ```
//!
//! Paths are compared as [`WorkspacePath`]s (ADR-010): workspace-relative,
//! `/`-separated, `.`/`..` and absolute forms already refused on
//! deserialization. That is the repository's one definition of path identity —
//! tabs, snapshots and documents all key on it — so ownership uses it too
//! rather than inventing a second one that could disagree.
//!
//! **Where the matcher is unsure, it reports a collision.** Two globs whose
//! intersection is not decidable cheaply (`*.ts` against `*.rs`) are treated as
//! overlapping. A false conflict costs the user a click; a missed one costs a
//! file.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use gritty_workspace::WorkspacePath;

use crate::protocol::error::CoreError;

/// Longest accepted glob pattern.
pub const MAX_PATTERN: usize = 240;
/// Most segments a glob pattern may have. Bounds the `**` backtracking below,
/// which is exponential in the number of `**` segments in the worst case.
pub const MAX_PATTERN_SEGMENTS: usize = 32;

/// The part of a workspace a lease covers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OwnershipTarget {
    /// Exactly one file.
    File { path: WorkspacePath },
    /// A directory and everything beneath it. The root directory is the whole
    /// workspace.
    Directory { path: WorkspacePath },
    /// A pattern. `pattern` is not a [`WorkspacePath`] because it carries
    /// wildcards; [`validate_pattern`] applies the same safety rules.
    Glob { pattern: String },
}

impl OwnershipTarget {
    /// Reject anything that is not a usable target. Paths are already validated
    /// by `WorkspacePath`'s deserializer, so only globs need work here.
    pub fn validate(&self) -> Result<(), CoreError> {
        match self {
            Self::File { path } => {
                if path.is_root() {
                    return Err(CoreError::invalid_input(
                        "target.path",
                        "the workspace root is a directory, not a file",
                    ));
                }
                Ok(())
            }
            Self::Directory { .. } => Ok(()),
            Self::Glob { pattern } => validate_pattern(pattern).map(|_| ()),
        }
    }

    /// How this target reads in a sentence, and what is stored.
    pub fn as_pattern(&self) -> String {
        match self {
            Self::File { path } => path.as_str().to_owned(),
            Self::Directory { path } if path.is_root() => "**".to_owned(),
            Self::Directory { path } => format!("{}/**", path.as_str()),
            Self::Glob { pattern } => pattern.clone(),
        }
    }

    /// The segments this target matches against.
    fn segments(&self) -> Vec<Segment> {
        parse_segments(&self.as_pattern())
    }

    /// Does this target include exactly this file?
    pub fn covers(&self, path: &WorkspacePath) -> bool {
        let path: Vec<&str> = path.segments().collect();
        matches_path(&self.segments(), &path)
    }

    /// Could any one path be inside both targets? Conservative: see the module
    /// note. Overlap is symmetric, which the tests assert.
    pub fn overlaps(&self, other: &Self) -> bool {
        may_intersect(&self.segments(), &other.segments())
    }

    /// The short kind name, for storage.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::File { .. } => "FILE",
            Self::Directory { .. } => "DIRECTORY",
            Self::Glob { .. } => "GLOB",
        }
    }

    /// Rebuild a target from its stored kind and pattern.
    pub fn from_storage(kind: &str, pattern: &str) -> Result<Self, CoreError> {
        let unreadable =
            |what: &str| CoreError::internal(format!("stored ownership target {what}"));
        match kind {
            "FILE" => Ok(Self::File {
                path: WorkspacePath::parse(pattern).map_err(|_| unreadable("path is not valid"))?,
            }),
            "DIRECTORY" => {
                let trimmed = pattern.strip_suffix("/**").unwrap_or(match pattern {
                    "**" => "",
                    other => other,
                });
                Ok(Self::Directory {
                    path: WorkspacePath::parse(trimmed)
                        .map_err(|_| unreadable("path is not valid"))?,
                })
            }
            "GLOB" => Ok(Self::Glob {
                pattern: pattern.to_owned(),
            }),
            other => Err(unreadable(&format!("has unknown kind {other:?}"))),
        }
    }
}

/// A glob is not a path, so it gets the same safety rules written out: no NUL,
/// no backslash, no absolute form, no `.` or `..`, no empty segment, bounded.
pub fn validate_pattern(pattern: &str) -> Result<Vec<Segment>, CoreError> {
    let field = "target.pattern";
    let trimmed = pattern.trim();
    if trimmed.is_empty() {
        return Err(CoreError::invalid_input(field, "must not be blank"));
    }
    if trimmed.chars().count() > MAX_PATTERN {
        return Err(CoreError::invalid_input(
            field,
            format!("must be at most {MAX_PATTERN} characters"),
        ));
    }
    if trimmed.contains('\u{0}') {
        return Err(CoreError::invalid_input(field, "must not contain NUL"));
    }
    if trimmed.contains('\\') {
        return Err(CoreError::invalid_input(
            field,
            "must use '/' as the separator (backslashes are not allowed)",
        ));
    }
    if trimmed.starts_with('/') || trimmed.contains(':') {
        return Err(CoreError::invalid_input(
            field,
            "must be relative to the workspace root",
        ));
    }
    let parts: Vec<&str> = trimmed.split('/').collect();
    if parts.len() > MAX_PATTERN_SEGMENTS {
        return Err(CoreError::invalid_input(
            field,
            format!("must be at most {MAX_PATTERN_SEGMENTS} segments"),
        ));
    }
    for part in &parts {
        match *part {
            "" => {
                return Err(CoreError::invalid_input(
                    field,
                    "must not contain empty segments or end with '/'",
                ));
            }
            "." | ".." => {
                return Err(CoreError::invalid_input(
                    field,
                    "must not contain '.' or '..' segments",
                ));
            }
            _ => {}
        }
    }
    Ok(parse_segments(trimmed))
}

/// One `/`-delimited piece of a pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// `**`: zero or more whole segments.
    AnyDepth,
    /// A literal name, matched byte for byte. No case folding and no Unicode
    /// normalization, because `WorkspacePath` does neither — see the known
    /// limitation in `docs/architecture/ownership.md`.
    Literal(String),
    /// A name containing `*` or `?`.
    Wild(String),
}

impl Segment {
    const fn is_any_depth(&self) -> bool {
        matches!(self, Self::AnyDepth)
    }
}

fn parse_segments(pattern: &str) -> Vec<Segment> {
    pattern
        .split('/')
        .filter(|part| !part.is_empty())
        .map(|part| {
            if part == "**" {
                Segment::AnyDepth
            } else if part.contains('*') || part.contains('?') {
                Segment::Wild(part.to_owned())
            } else {
                Segment::Literal(part.to_owned())
            }
        })
        .collect()
}

/// Does `pattern` match this concrete path, in full?
fn matches_path(pattern: &[Segment], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((Segment::AnyDepth, rest)) => {
            // Zero or more segments, shortest first.
            (0..=path.len()).any(|skip| matches_path(rest, &path[skip..]))
        }
        Some((head, rest)) => match path.split_first() {
            None => false,
            Some((name, tail)) => matches_name(head, name) && matches_path(rest, tail),
        },
    }
}

fn matches_name(segment: &Segment, name: &str) -> bool {
    match segment {
        Segment::AnyDepth => true,
        Segment::Literal(literal) => literal == name,
        Segment::Wild(pattern) => wildcard(pattern.as_bytes(), name.as_bytes()),
    }
}

/// `*` and `?` inside one segment. Iterative with one backtrack point, so a
/// pathological pattern cannot blow the stack.
fn wildcard(pattern: &[u8], name: &[u8]) -> bool {
    let (mut p, mut n) = (0usize, 0usize);
    let (mut star, mut resume) = (None, 0usize);
    while n < name.len() {
        match pattern.get(p) {
            Some(b'*') => {
                star = Some(p);
                p += 1;
                resume = n;
            }
            Some(b'?') => {
                p += 1;
                n += 1;
            }
            Some(byte) if *byte == name[n] => {
                p += 1;
                n += 1;
            }
            _ => match star {
                Some(index) => {
                    p = index + 1;
                    resume += 1;
                    n = resume;
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|byte| *byte == b'*')
}

/// Could one path match both patterns? Where that is not cheap to decide, this
/// answers `true` — the safe direction (module note).
fn may_intersect(left: &[Segment], right: &[Segment]) -> bool {
    match (left.split_first(), right.split_first()) {
        (None, None) => true,
        // One pattern ran out. The other can still match the empty remainder
        // only if everything left of it is `**`.
        (None, Some(_)) => right.iter().all(Segment::is_any_depth),
        (Some(_), None) => left.iter().all(Segment::is_any_depth),
        (Some((head, left_rest)), Some((other, right_rest))) => {
            if head.is_any_depth() {
                return (0..=right.len()).any(|skip| may_intersect(left_rest, &right[skip..]));
            }
            if other.is_any_depth() {
                return (0..=left.len()).any(|skip| may_intersect(&left[skip..], right_rest));
            }
            names_may_intersect(head, other) && may_intersect(left_rest, right_rest)
        }
    }
}

/// Two single segments. Two literals are exact; anything with a wildcard is
/// assumed to intersect rather than solved.
fn names_may_intersect(left: &Segment, right: &Segment) -> bool {
    match (left, right) {
        (Segment::Literal(a), Segment::Literal(b)) => a == b,
        (Segment::Wild(pattern), Segment::Literal(name))
        | (Segment::Literal(name), Segment::Wild(pattern)) => {
            wildcard(pattern.as_bytes(), name.as_bytes())
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::error::CoreErrorCode;

    fn path(raw: &str) -> WorkspacePath {
        WorkspacePath::parse(raw).expect("valid path")
    }

    fn file(raw: &str) -> OwnershipTarget {
        OwnershipTarget::File { path: path(raw) }
    }

    fn dir(raw: &str) -> OwnershipTarget {
        OwnershipTarget::Directory { path: path(raw) }
    }

    fn glob(raw: &str) -> OwnershipTarget {
        OwnershipTarget::Glob {
            pattern: raw.to_owned(),
        }
    }

    #[test]
    fn a_file_covers_only_itself() {
        let target = file("src/auth/session.ts");
        assert!(target.covers(&path("src/auth/session.ts")));
        assert!(!target.covers(&path("src/auth/session.tsx")));
        assert!(!target.covers(&path("src/auth")));
        assert!(!target.covers(&path("src/auth/session.ts/nested")));
    }

    #[test]
    fn a_directory_covers_itself_and_everything_under_it() {
        let target = dir("src/auth");
        assert!(target.covers(&path("src/auth")));
        assert!(target.covers(&path("src/auth/session.ts")));
        assert!(target.covers(&path("src/auth/deep/nested/file.ts")));
        assert!(!target.covers(&path("src/authentication/session.ts")));
        assert!(!target.covers(&path("src")));
    }

    #[test]
    fn the_root_directory_covers_the_whole_workspace() {
        let target = dir("");
        assert_eq!(target.as_pattern(), "**");
        assert!(target.covers(&path("Cargo.toml")));
        assert!(target.covers(&path("crates/gritty-core/src/lib.rs")));
    }

    #[test]
    fn a_single_star_does_not_cross_a_separator() {
        let target = glob("src/*.ts");
        assert!(target.covers(&path("src/index.ts")));
        assert!(!target.covers(&path("src/auth/index.ts")));

        let deep = glob("src/**/*.ts");
        assert!(deep.covers(&path("src/index.ts")));
        assert!(deep.covers(&path("src/auth/index.ts")));
        assert!(deep.covers(&path("src/a/b/c/index.ts")));
        assert!(!deep.covers(&path("src/index.rs")));
    }

    #[test]
    fn a_question_mark_is_one_character() {
        let target = glob("src/v?.ts");
        assert!(target.covers(&path("src/v1.ts")));
        assert!(!target.covers(&path("src/v10.ts")));
    }

    #[test]
    fn independent_paths_do_not_overlap() {
        assert!(!file("src/auth/session.ts").overlaps(&file("src/payments/router.ts")));
        assert!(!dir("src/auth").overlaps(&dir("src/payments")));
        assert!(!glob("tests/auth/**/*.ts").overlaps(&glob("tests/payments/**/*.ts")));
    }

    #[test]
    fn identical_targets_overlap() {
        assert!(file("src/auth/session.ts").overlaps(&file("src/auth/session.ts")));
        assert!(dir("src/auth").overlaps(&dir("src/auth")));
        assert!(glob("src/**/*.ts").overlaps(&glob("src/**/*.ts")));
    }

    #[test]
    fn a_directory_overlaps_a_file_inside_it_both_ways() {
        let directory = dir("src/auth");
        let inside = file("src/auth/session.ts");
        assert!(directory.overlaps(&inside));
        assert!(inside.overlaps(&directory));

        let outside = file("src/payments/router.ts");
        assert!(!directory.overlaps(&outside));
        assert!(!outside.overlaps(&directory));
    }

    #[test]
    fn nested_directories_overlap() {
        assert!(dir("src").overlaps(&dir("src/auth")));
        assert!(dir("src/auth").overlaps(&dir("src")));
        assert!(!dir("src/auth").overlaps(&dir("docs")));
    }

    #[test]
    fn a_glob_overlaps_a_file_it_matches() {
        assert!(glob("src/**/*.ts").overlaps(&file("src/auth/session.ts")));
        assert!(file("src/auth/session.ts").overlaps(&glob("src/**/*.ts")));
        assert!(!glob("docs/**/*.md").overlaps(&file("src/auth/session.ts")));
    }

    #[test]
    fn a_glob_overlaps_a_directory_it_could_reach_into() {
        assert!(glob("src/**/*.ts").overlaps(&dir("src/auth")));
        assert!(dir("src/auth").overlaps(&glob("src/**/*.ts")));
        assert!(!glob("docs/**/*.md").overlaps(&dir("src/auth")));
    }

    #[test]
    fn two_globs_that_cannot_be_told_apart_cheaply_report_a_collision() {
        // Decidable: different literal prefixes.
        assert!(!glob("src/auth/*.ts").overlaps(&glob("src/payments/*.ts")));
        // Not decidable cheaply: the extensions differ but both are wildcards
        // in the same position. The safe answer is "they might".
        assert!(glob("src/auth/*.ts").overlaps(&glob("src/auth/*.rs")));
    }

    #[test]
    fn overlap_is_symmetric_across_every_pair() {
        let targets = [
            file("src/auth/session.ts"),
            file("docs/readme.md"),
            dir("src"),
            dir("src/auth"),
            dir(""),
            glob("src/**/*.ts"),
            glob("docs/**"),
            glob("**/session.ts"),
        ];
        for left in &targets {
            for right in &targets {
                assert_eq!(
                    left.overlaps(right),
                    right.overlaps(left),
                    "{left:?} vs {right:?}"
                );
            }
        }
    }

    #[test]
    fn everything_overlaps_the_root() {
        let root = dir("");
        for target in [
            file("src/auth/session.ts"),
            dir("docs"),
            glob("**/*.rs"),
            glob("Cargo.toml"),
        ] {
            assert!(root.overlaps(&target), "root vs {target:?}");
            assert!(target.overlaps(&root), "{target:?} vs root");
        }
    }

    #[test]
    fn patterns_refuse_traversal_absolute_and_malformed_forms() {
        for bad in [
            "",
            "   ",
            "/etc/passwd",
            "../secrets",
            "src/../../etc",
            "src//auth",
            "src/auth/",
            "src\\auth",
            "C:/Windows",
            "src/.",
        ] {
            let error = validate_pattern(bad).unwrap_err();
            assert_eq!(error.code, CoreErrorCode::InvalidInput, "{bad:?}");
        }
        assert!(validate_pattern("src/**/*.ts").is_ok());
        assert!(validate_pattern("**").is_ok());
    }

    #[test]
    fn patterns_are_bounded_in_length_and_depth() {
        let long = "a".repeat(MAX_PATTERN + 1);
        assert!(validate_pattern(&long).is_err());
        let deep = vec!["a"; MAX_PATTERN_SEGMENTS + 1].join("/");
        assert!(validate_pattern(&deep).is_err());
    }

    #[test]
    fn a_file_target_cannot_be_the_root() {
        let error = OwnershipTarget::File {
            path: WorkspacePath::root(),
        }
        .validate()
        .unwrap_err();
        assert_eq!(error.code, CoreErrorCode::InvalidInput);
        assert!(dir("").validate().is_ok());
    }

    #[test]
    fn targets_round_trip_through_storage() {
        for target in [
            file("src/auth/session.ts"),
            dir("src/auth"),
            dir(""),
            glob("tests/**/*.ts"),
        ] {
            let back =
                OwnershipTarget::from_storage(target.kind(), &target.as_pattern()).expect("stored");
            assert_eq!(back, target);
        }
        assert_eq!(
            OwnershipTarget::from_storage("SYMBOL", "auth::session")
                .unwrap_err()
                .code,
            CoreErrorCode::Internal
        );
    }

    #[test]
    fn targets_use_a_tagged_wire_shape() {
        let json = serde_json::to_value(file("src/auth/session.ts")).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "type": "FILE", "path": "src/auth/session.ts" })
        );
        let json = serde_json::to_value(glob("src/**")).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "type": "GLOB", "pattern": "src/**" })
        );
    }

    #[test]
    fn a_wire_path_carrying_traversal_is_refused_by_the_path_type() {
        let raw = serde_json::json!({ "type": "FILE", "path": "../../etc/passwd" });
        assert!(serde_json::from_value::<OwnershipTarget>(raw).is_err());
    }
}
