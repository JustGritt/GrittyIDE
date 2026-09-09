//! Every argument this crate can hand to `git`, as a pure function of the
//! request.
//!
//! ADR-029 decision 3 is enforced here, and it is enforced here *because* this
//! is a pure function: the argv a request produces can be asserted without a
//! repository, a process or a filesystem, which makes "no repository content
//! ever reaches an option position" a unit test rather than a code review.
//!
//! Three rules, each of which has a test below:
//!
//! 1. **Pathspecs are always last, always after `--`, and always literal.**
//!    Never `git add -A`, never `git commit -a`. Gritty ships a terminal, so a
//!    user's own `git` may be running concurrently; a wildcard write would
//!    sweep up whatever another process changed a moment ago.
//! 2. **`--` is not enough on its own.** Git reads pathspec *magic* after `--`
//!    as well, so a file genuinely named `:(icase)notes.md` would be read as a
//!    case-insensitive pattern rather than as itself. Every pathspec is
//!    therefore prefixed `:(literal)`, which turns the rest off — no magic, no
//!    wildcards, exactly one path.
//! 3. **The commit message is delivered by file**, never interpolated into a
//!    command line, and with `--cleanup=verbatim` so it is committed as the
//!    user typed it.

use std::ffi::OsString;
use std::path::PathBuf;

use gritty_workspace::WorkspacePath;

/// The subcommands this crate is allowed to invoke, and the whole of them.
///
/// A closed set, asserted against [`GitWrite::subcommand`] by
/// `every_subcommand_is_on_the_allowlist`. Nothing networked appears here:
/// `fetch`, `pull`, `push`, `clone` and `remote` are ADR-029 decision 3's
/// exclusion, and adding one is a decision an ADR has to make, not a patch.
pub const ALLOWED_SUBCOMMANDS: &[&str] = &["add", "reset", "stash", "commit"];

/// What a commit should carry beyond its message. Each is a choice the user
/// made in the UI; none is a default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CommitOptions {
    /// Replace the previous commit rather than adding one.
    pub amend: bool,
    /// Append a `Signed-off-by` trailer.
    pub signoff: bool,
    /// Skip the repository's own `pre-commit` and `commit-msg` hooks. Running
    /// them is the point (ADR-029 decision 4), so this is never a default.
    pub no_verify: bool,
}

/// One write, as the caller asked for it.
///
/// `Stash` is not one of the four commands on the wire: it is the first half of
/// a discard, and it exists as its own variant because a discard that could not
/// stash must not go on to restore (ADR-029, and recoverability's place in the
/// priority order).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitWrite {
    Stage {
        paths: Vec<WorkspacePath>,
    },
    Unstage {
        paths: Vec<WorkspacePath>,
    },
    Stash {
        paths: Vec<WorkspacePath>,
    },
    Commit {
        /// A file this crate wrote, holding the message verbatim. Never a path
        /// from the repository.
        message_file: PathBuf,
        options: CommitOptions,
    },
}

impl GitWrite {
    /// The `git` subcommand, for the allowlist and for logging. Never the
    /// arguments: a path from the repository is not a thing to log (rule 13).
    pub fn subcommand(&self) -> &'static str {
        match self {
            Self::Stage { .. } => "add",
            Self::Unstage { .. } => "reset",
            Self::Stash { .. } => "stash",
            Self::Commit { .. } => "commit",
        }
    }

    /// The full argument vector, `git` itself excluded.
    pub fn argv(&self) -> Vec<OsString> {
        match self {
            Self::Stage { paths } => with_paths(&["add"], paths),
            // `reset`, not `restore --staged`: `git restore --staged` fails
            // with "could not resolve HEAD" on an unborn branch, where staging
            // the very first file and changing your mind is an ordinary thing
            // to do. `git reset -- <paths>` handles that case and needs no
            // second code path to detect it. Verified against git 2.50.
            Self::Unstage { paths } => with_paths(&["reset", "--quiet"], paths),
            // `stash push` *is* the discard for a tracked path: it reverts the
            // worktree and the index and keeps a recoverable copy, in one
            // command. Nothing follows it, so there is no window in which the
            // change is gone and not yet saved.
            Self::Stash { paths } => with_paths(&["stash", "push", "--quiet"], paths),
            Self::Commit {
                message_file,
                options,
            } => {
                let mut argv: Vec<OsString> = vec!["commit".into()];
                // Verbatim, because the default (`whitespace`) strips every
                // line beginning with `#` — which silently eats a message
                // whose first line is a Markdown heading or which cites an
                // issue as `#42`. What the user typed is what gets committed.
                argv.push("--cleanup=verbatim".into());
                argv.push("--file".into());
                argv.push(message_file.clone().into_os_string());
                if options.amend {
                    argv.push("--amend".into());
                }
                if options.signoff {
                    argv.push("--signoff".into());
                }
                if options.no_verify {
                    argv.push("--no-verify".into());
                }
                argv
            }
        }
    }
}

/// `<leading…> -- :(literal)<path>…`
fn with_paths(leading: &[&str], paths: &[WorkspacePath]) -> Vec<OsString> {
    let mut argv: Vec<OsString> = leading.iter().map(OsString::from).collect();
    argv.push("--".into());
    for path in paths {
        argv.push(pathspec(path));
    }
    argv
}

/// One pathspec, with magic turned off.
fn pathspec(path: &WorkspacePath) -> OsString {
    let mut spec = OsString::from(":(literal)");
    spec.push(path.as_str());
    spec
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(raw: &str) -> WorkspacePath {
        WorkspacePath::parse(raw).expect("a valid workspace path")
    }

    fn strings(write: &GitWrite) -> Vec<String> {
        write
            .argv()
            .into_iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn every_subcommand_is_on_the_allowlist() {
        // The allowlist is the reviewable statement of what this crate can do.
        // A new variant that forgot to be considered fails here.
        for write in [
            GitWrite::Stage { paths: vec![] },
            GitWrite::Unstage { paths: vec![] },
            GitWrite::Stash { paths: vec![] },
            GitWrite::Commit {
                message_file: PathBuf::from("/tmp/msg"),
                options: CommitOptions::default(),
            },
        ] {
            assert!(
                ALLOWED_SUBCOMMANDS.contains(&write.subcommand()),
                "{} is not on the allowlist",
                write.subcommand()
            );
        }
    }

    #[test]
    fn nothing_networked_is_reachable() {
        // ADR-029 decision 3: credentials and transports stay out of reach.
        // Stated as an assertion so that adding `push` to this crate has to
        // delete a test, which is a thing a reviewer notices.
        for networked in ["fetch", "pull", "push", "clone", "remote", "ls-remote"] {
            assert!(
                !ALLOWED_SUBCOMMANDS.contains(&networked),
                "{networked} must not be reachable from this crate"
            );
        }
    }

    #[test]
    fn staging_puts_paths_after_a_double_dash() {
        let argv = strings(&GitWrite::Stage {
            paths: vec![path("src/a.ts"), path("README.md")],
        });
        assert_eq!(
            argv,
            ["add", "--", ":(literal)src/a.ts", ":(literal)README.md"]
        );
    }

    #[test]
    fn a_path_that_looks_like_a_flag_is_an_argument() {
        // The failure this prevents: a file named `--upload-pack=…` reaching an
        // option position. `--` is what guarantees it cannot.
        let argv = strings(&GitWrite::Stage {
            paths: vec![path("--upload-pack=evil")],
        });
        assert_eq!(argv, ["add", "--", ":(literal)--upload-pack=evil"]);
        let separator = argv.iter().position(|arg| arg == "--").expect("a `--`");
        assert!(
            argv[separator + 1..]
                .iter()
                .all(|arg| arg.starts_with(":(literal)")),
            "every argument after `--` must be a literal pathspec"
        );
    }

    #[test]
    fn a_path_that_looks_like_pathspec_magic_is_an_argument() {
        // `--` does not turn magic off; `:(literal)` does. Without it this file
        // would be read as a case-insensitive pattern rather than as itself.
        let argv = strings(&GitWrite::Stage {
            paths: vec![path(":(icase)notes.md")],
        });
        assert_eq!(argv, ["add", "--", ":(literal):(icase)notes.md"]);
    }

    #[test]
    fn a_glob_in_a_filename_matches_only_that_file() {
        let argv = strings(&GitWrite::Stage {
            paths: vec![path("weird/*.ts")],
        });
        assert_eq!(argv, ["add", "--", ":(literal)weird/*.ts"]);
    }

    #[test]
    fn staging_is_never_wholesale() {
        // `-A`/`--all`/`.` would sweep up what another process changed a moment
        // ago — and Gritty ships a terminal.
        for write in [
            GitWrite::Stage {
                paths: vec![path("a.ts")],
            },
            GitWrite::Unstage {
                paths: vec![path("a.ts")],
            },
            GitWrite::Stash {
                paths: vec![path("a.ts")],
            },
        ] {
            let argv = strings(&write);
            for forbidden in ["-A", "--all", "-a", "."] {
                assert!(
                    !argv.iter().any(|arg| arg == forbidden),
                    "{argv:?} must not contain {forbidden}"
                );
            }
        }
    }

    #[test]
    fn unstaging_uses_reset_so_an_unborn_branch_works() {
        let argv = strings(&GitWrite::Unstage {
            paths: vec![path("a.ts")],
        });
        assert_eq!(argv, ["reset", "--quiet", "--", ":(literal)a.ts"]);
    }

    #[test]
    fn a_discard_stashes_before_anything_is_lost() {
        let argv = strings(&GitWrite::Stash {
            paths: vec![path("a.ts")],
        });
        assert_eq!(argv, ["stash", "push", "--quiet", "--", ":(literal)a.ts"]);
    }

    #[test]
    fn a_commit_takes_its_message_from_a_file_and_verbatim() {
        let argv = strings(&GitWrite::Commit {
            message_file: PathBuf::from("/tmp/gritty-msg"),
            options: CommitOptions::default(),
        });
        assert_eq!(
            argv,
            ["commit", "--cleanup=verbatim", "--file", "/tmp/gritty-msg"]
        );
        // Never `-m`: a message is not a command-line argument.
        assert!(!argv.iter().any(|arg| arg == "-m" || arg == "--message"));
    }

    #[test]
    fn commit_options_are_opt_in_and_appear_only_when_chosen() {
        let none = strings(&GitWrite::Commit {
            message_file: PathBuf::from("/tmp/m"),
            options: CommitOptions::default(),
        });
        for opt in ["--amend", "--signoff", "--no-verify"] {
            assert!(
                !none.iter().any(|arg| arg == opt),
                "{opt} must not default on"
            );
        }

        let all = strings(&GitWrite::Commit {
            message_file: PathBuf::from("/tmp/m"),
            options: CommitOptions {
                amend: true,
                signoff: true,
                no_verify: true,
            },
        });
        for opt in ["--amend", "--signoff", "--no-verify"] {
            assert!(all.iter().any(|arg| arg == opt), "{opt} should be present");
        }
    }
}
