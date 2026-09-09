//! Is there a usable `git`, and which one?
//!
//! ADR-029 decision 5: a missing or too-old binary disables the write half
//! **with a stated reason**, and never breaks a read. The reason is the whole
//! point — a button that is merely absent looks like a feature that does not
//! exist, and one that is enabled and inert is worse.
//!
//! This is the one place the crate reads `git`'s output, and it is not the
//! parser ADR-023 refused to maintain: `git version X.Y.Z` is a single stable
//! line, not porcelain, and there is no other way to learn a version.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

/// The oldest `git` this crate will drive.
///
/// 2.13 is where `git stash push -- <pathspec>` arrived, and a discard that
/// cannot stash is a discard that cannot be recovered from — which
/// recoverability's place in the priority order rules out. Released May 2017.
pub const MINIMUM_VERSION: (u32, u32) = (2, 13);

/// A probe cannot take longer than this. A `git --version` that hangs means
/// something is wrong with the binary, and hanging the write half behind it
/// would be the worst of both outcomes.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// What the probe found. Computed once per workspace and cached by the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    /// Usable, at this path and this version.
    Available { program: PathBuf, version: String },
    /// Not usable, and why — in words meant for the user, not for a log.
    Unavailable { reason: String },
}

impl Availability {
    pub fn program(&self) -> Option<&Path> {
        match self {
            Self::Available { program, .. } => Some(program.as_path()),
            Self::Unavailable { .. } => None,
        }
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Available { .. } => None,
            Self::Unavailable { reason } => Some(reason),
        }
    }
}

/// Run `git --version` once.
pub async fn probe(program: &Path) -> Availability {
    let mut command = Command::new(program);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    // A probe must not be redirected into some other repository either, and it
    // must not open a pager.
    command
        .env("GIT_PAGER", "cat")
        .env("GIT_TERMINAL_PROMPT", "0");

    let run = tokio::time::timeout(PROBE_TIMEOUT, command.output()).await;
    let output = match run {
        Err(_elapsed) => {
            return Availability::Unavailable {
                reason: format!("{} did not answer `--version`", program.display()),
            };
        }
        Ok(Err(source)) => {
            return Availability::Unavailable {
                reason: format!("{} could not be run: {source}", program.display()),
            };
        }
        Ok(Ok(output)) => output,
    };

    if !output.status.success() {
        return Availability::Unavailable {
            reason: format!("{} --version failed", program.display()),
        };
    }

    let line = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let Some(version) = parse_version(&line) else {
        return Availability::Unavailable {
            reason: format!("could not read a version from {line:?}"),
        };
    };
    if (version.0, version.1) < MINIMUM_VERSION {
        return Availability::Unavailable {
            reason: format!(
                "git {}.{} is older than the {}.{} this needs for a recoverable discard",
                version.0, version.1, MINIMUM_VERSION.0, MINIMUM_VERSION.1
            ),
        };
    }
    Availability::Available {
        program: program.to_path_buf(),
        version: line,
    }
}

/// `git version 2.50.1 (Apple Git-155)` → `(2, 50)`.
fn parse_version(line: &str) -> Option<(u32, u32)> {
    let rest = line.strip_prefix("git version ")?;
    let mut numbers = rest.split(['.', ' ', '-']);
    let major = numbers.next()?.parse().ok()?;
    let minor = numbers.next()?.parse().ok()?;
    Some((major, minor))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_versions_git_actually_prints() {
        // The middle one is what macOS ships, and its suffix is why this parses
        // rather than splitting on '.' alone.
        assert_eq!(parse_version("git version 2.13.0"), Some((2, 13)));
        assert_eq!(
            parse_version("git version 2.50.1 (Apple Git-155)"),
            Some((2, 50))
        );
        assert_eq!(parse_version("git version 2.39.3.windows.1"), Some((2, 39)));
    }

    #[test]
    fn anything_else_is_not_a_version() {
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("not git at all"), None);
        assert_eq!(parse_version("git version"), None);
    }

    #[tokio::test]
    async fn an_absent_binary_is_unavailable_with_a_reason() {
        let found = probe(Path::new("/nonexistent/git")).await;
        // Never a panic and never a silent false: the UI shows this string.
        assert!(
            found
                .reason()
                .is_some_and(|why| why.contains("/nonexistent/git"))
        );
        assert!(found.program().is_none());
    }

    #[tokio::test]
    async fn the_real_git_on_this_machine_is_usable() {
        let found = probe(Path::new("git")).await;
        assert!(
            matches!(found, Availability::Available { .. }),
            "expected the developer's own git to be usable, got {found:?}"
        );
    }
}
