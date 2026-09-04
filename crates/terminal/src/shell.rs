//! Shell and environment policy (addendum §14 "Environment/processes").
//!
//! Gritty runs the user's **login** shell, the way a terminal emulator does:
//! `argv[0]` is the shell's basename prefixed with `-`, so the shell sources
//! the user's profile. This is not cosmetic. An application launched from
//! Finder or a desktop launcher inherits a minimal environment (a short
//! `PATH`, no toolchain managers), and a non-login shell would leave the
//! terminal unable to find the tools the same user has in their own terminal.
//!
//! Resolution order for the program: an explicit `shell` from the caller, then
//! `$SHELL`, then the platform default, then `/bin/sh`. Every candidate must
//! be an executable file; a candidate that is not is a spawn error rather than
//! a silent fall-through, so "my shell did not start" is never mistaken for
//! "Gritty started something else".
//!
//! The child inherits Gritty's environment (which is the user's own, plus
//! whatever the launcher set) and then gets `TERM` and `COLORTERM` forced to
//! values the frontend can actually render. Caller-supplied variables are
//! applied last. Nothing is injected that names a workspace, a document or a
//! session: the shell is the user's, not the application's.

use std::path::{Path, PathBuf};

use crate::error::TerminalError;

/// Terminal type reported to the child. `xterm-256color` is what xterm.js
/// implements and what every mainstream terminfo database carries.
pub const TERM_VALUE: &str = "xterm-256color";
/// Advertises 24-bit colour support, which xterm.js renders.
pub const COLORTERM_VALUE: &str = "truecolor";

/// Shell used when neither the caller nor `$SHELL` names one.
pub const fn platform_default_shell() -> &'static str {
    if cfg!(target_os = "macos") {
        "/bin/zsh"
    } else {
        "/bin/bash"
    }
}

/// Last resort: POSIX guarantees this path.
const FALLBACK_SHELL: &str = "/bin/sh";

/// Resolve the shell to start, in the documented order. `lookup_env` supplies
/// `$SHELL` so tests do not depend on the developer's environment.
pub fn resolve_shell(
    explicit: Option<&Path>,
    lookup_env: impl Fn(&str) -> Option<String>,
) -> Result<PathBuf, TerminalError> {
    // An explicit request is never silently replaced: if it cannot run, say so.
    if let Some(path) = explicit {
        return check_executable(path).map(|()| path.to_path_buf());
    }
    let from_env = lookup_env("SHELL").map(PathBuf::from);
    let candidates = [
        from_env,
        Some(PathBuf::from(platform_default_shell())),
        Some(PathBuf::from(FALLBACK_SHELL)),
    ];
    let mut last_error = None;
    for candidate in candidates.into_iter().flatten() {
        match check_executable(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) => {
                tracing::debug!(shell = %candidate.display(), %error, "shell candidate rejected");
                last_error = Some(error);
            }
        }
    }
    Err(last_error.unwrap_or_else(|| TerminalError::Spawn {
        program: PathBuf::from(FALLBACK_SHELL),
        source: std::io::Error::from(std::io::ErrorKind::NotFound),
    }))
}

/// The working directory a session starts in. The caller's directory must
/// exist and be a directory; `portable-pty` would otherwise fall back to
/// `$HOME` without telling anyone.
pub fn check_cwd(path: &Path) -> Result<(), TerminalError> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(TerminalError::Cwd {
            path: path.to_path_buf(),
            reason: "not a directory".into(),
        }),
        Err(error) => Err(TerminalError::Cwd {
            path: path.to_path_buf(),
            reason: error.to_string(),
        }),
    }
}

/// An existing file with at least one execute bit.
fn check_executable(path: &Path) -> Result<(), TerminalError> {
    let metadata = std::fs::metadata(path).map_err(|source| TerminalError::Spawn {
        program: path.to_path_buf(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(TerminalError::Spawn {
            program: path.to_path_buf(),
            source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "not a regular file"),
        });
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(TerminalError::Spawn {
                program: path.to_path_buf(),
                source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "not executable"),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use crate::testing::TempDir;

    fn executable(dir: &TempDir, name: &str) -> PathBuf {
        let path = dir.join(name);
        let mut file = std::fs::File::create(&path).expect("create");
        file.write_all(b"#!/bin/sh\n").expect("write");
        drop(file);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        path
    }

    #[test]
    fn an_explicit_shell_wins() {
        let dir = TempDir::new("shell-explicit");
        let shell = executable(&dir, "myshell");
        let resolved = resolve_shell(Some(&shell), |_| Some("/bin/sh".into())).expect("resolves");
        assert_eq!(resolved, shell);
    }

    #[test]
    fn an_explicit_shell_that_cannot_run_is_an_error_not_a_fallback() {
        let dir = TempDir::new("shell-missing");
        let missing = dir.join("nope");
        let error = resolve_shell(Some(&missing), |_| Some("/bin/sh".into()))
            .expect_err("must not silently start another shell");
        assert!(matches!(error, TerminalError::Spawn { .. }));
        assert!(error.is_caller_fault());
    }

    #[test]
    fn a_non_executable_explicit_shell_is_rejected() {
        let dir = TempDir::new("shell-noexec");
        let path = dir.join("plain");
        std::fs::write(&path, b"not executable").expect("write");
        let error = resolve_shell(Some(&path), |_| None).expect_err("rejected");
        assert!(matches!(error, TerminalError::Spawn { .. }));
    }

    #[test]
    fn falls_back_from_shell_env_to_the_platform_default() {
        let dir = TempDir::new("shell-env");
        let missing = dir.join("gone");
        let resolved = resolve_shell(None, |key| {
            (key == "SHELL").then(|| missing.display().to_string())
        })
        .expect("a shell exists on any developer machine");
        assert_ne!(resolved, missing, "the broken $SHELL was skipped");
        assert!(resolved.is_absolute());
    }

    #[test]
    fn uses_shell_from_the_environment_when_it_runs() {
        let dir = TempDir::new("shell-ok");
        let shell = executable(&dir, "envshell");
        let resolved = resolve_shell(None, |key| {
            (key == "SHELL").then(|| shell.display().to_string())
        })
        .expect("resolves");
        assert_eq!(resolved, shell);
    }

    #[test]
    fn cwd_must_be_a_directory() {
        let dir = TempDir::new("cwd");
        assert!(check_cwd(dir.path()).is_ok());

        let file = dir.join("file.txt");
        std::fs::write(&file, b"x").expect("write");
        let error = check_cwd(&file).expect_err("a file is not a working directory");
        assert!(error.is_caller_fault());

        let error = check_cwd(&dir.join("missing")).expect_err("missing directory");
        assert!(matches!(error, TerminalError::Cwd { .. }));
    }
}
