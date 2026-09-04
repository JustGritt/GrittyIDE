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

/// Where a shell starts when no workspace roots it (T095).
///
/// `$HOME` first, then the account's own home from the password database.
/// The second source is not paranoia: an application started by a launcher,
/// a `launchd` job or a service manager can be handed an environment without
/// `HOME`, and "your terminal will not open" is a poor answer to that.
///
/// The renderer never names this path — it asks for a terminal without a
/// workspace and the core decides where that is. That is what keeps ADR-010's
/// property intact: the WebView cannot root a shell wherever it likes.
pub fn home_directory(
    lookup_env: impl Fn(&str) -> Option<String>,
) -> Result<PathBuf, TerminalError> {
    let from_env = lookup_env("HOME").map(PathBuf::from).filter(|path| {
        // A relative `HOME` would resolve against Gritty's own directory.
        path.is_absolute()
    });
    let candidates = [from_env, passwd_home()];
    let mut last_error = None;
    for candidate in candidates.into_iter().flatten() {
        match check_cwd(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) => {
                tracing::debug!(home = %candidate.display(), %error, "home candidate rejected");
                last_error = Some(error);
            }
        }
    }
    Err(last_error.unwrap_or_else(|| TerminalError::Cwd {
        path: PathBuf::from("$HOME"),
        reason: "no home directory for this account".into(),
    }))
}

#[cfg(unix)]
fn passwd_home() -> Option<PathBuf> {
    nix::unistd::User::from_uid(nix::unistd::Uid::current())
        .ok()
        .flatten()
        .map(|user| user.dir)
}

#[cfg(not(unix))]
fn passwd_home() -> Option<PathBuf> {
    None
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

/// A shell the user can start a session with (`terminal.profiles`).
///
/// Discovery reports what the machine can actually run: every candidate has
/// been checked with [`check_executable`], so a listed profile is one
/// [`resolve_shell`] would accept. It can still go stale — a shell uninstalled
/// between the listing and the spawn is a spawn error, not a silent
/// substitution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellProfile {
    pub path: PathBuf,
    /// The shell [`resolve_shell`] picks when the caller names none.
    pub is_default: bool,
}

/// Every shell the machine can run, the default first and the rest ordered by
/// name so the list does not reshuffle between calls.
///
/// Two sources, both authoritative:
///
/// - `$SHELL`, resolved exactly the way a spawn resolves it, so the entry
///   marked default is the one the user actually gets.
/// - `/etc/shells`, which *is* the operating system's register of valid login
///   shells — the list `chsh` validates against.
///
/// Deliberately nothing else. An earlier version also looked for a fixed set
/// of shell *names* under the usual `bin` directories, and on the first
/// machine it ran against it offered `/usr/local/bin/nu` — a symlink to an npm
/// package manager helper, not nushell. A file name is not evidence that a
/// program is a shell, and a picker that lists things the user cannot use is
/// worse than one that misses something they can add by hand.
///
/// Both inputs are injected so the tests do not depend on the developer's
/// machine.
///
/// This only reads file metadata. It starts no process, which is what makes it
/// safe to call from a command handler.
pub fn discover_shells(
    lookup_env: impl Fn(&str) -> Option<String>,
    etc_shells: Option<&str>,
) -> Vec<ShellProfile> {
    // The same resolution a spawn without an explicit shell would perform, so
    // the entry marked default is the one the user actually gets.
    let default = resolve_shell(None, &lookup_env).ok();

    let mut seen = Vec::new();
    let mut found: Vec<ShellProfile> = Vec::new();
    let mut accept = |path: PathBuf, profiles: &mut Vec<ShellProfile>| {
        if check_shell(&path).is_err() {
            return;
        }
        // A merged-`/usr` system reaches the same binary as `/bin/bash` and
        // `/usr/bin/bash`; the user does not want it listed twice. The first
        // spelling encountered is the one displayed.
        let identity = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if seen.contains(&identity) {
            return;
        }
        seen.push(identity);
        profiles.push(ShellProfile {
            path,
            is_default: false,
        });
    };

    if let Some(path) = default.clone() {
        let mut first = Vec::new();
        accept(path, &mut first);
        // `resolve_shell` already proved it runs, so `first` cannot be empty.
        for profile in &mut first {
            profile.is_default = true;
        }
        found.extend(first);
    }

    for line in etc_shells.unwrap_or_default().lines() {
        let entry = line.split('#').next().unwrap_or_default().trim();
        // `/etc/shells` is a system file, but it is still parsed defensively:
        // a relative entry would resolve against Gritty's cwd.
        if entry.is_empty() {
            continue;
        }
        accept(PathBuf::from(entry), &mut found);
    }

    // The default keeps position 0; the rest are sorted so two runs on one
    // machine produce the same list and the picker does not jump around.
    let rest = if default.is_some() { 1 } else { 0 };
    found[rest..].sort_by(|a, b| {
        let name = |p: &Path| p.file_name().map(|n| n.to_owned()).unwrap_or_default();
        name(&a.path)
            .cmp(&name(&b.path))
            .then_with(|| a.path.cmp(&b.path))
    });
    found
}

/// Whether a path the user typed can be started as a shell.
///
/// The same check discovery and `spawn` apply, exposed so the terminal
/// settings can tell the user *why* a path was rejected while they are typing
/// it, instead of failing later at spawn time with the same message.
pub fn check_shell(path: &Path) -> Result<(), TerminalError> {
    if !path.is_absolute() {
        return Err(TerminalError::Spawn {
            program: path.to_path_buf(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "must be an absolute path",
            ),
        });
    }
    check_executable(path)
}

/// [`discover_shells`] against this machine.
pub fn system_shells() -> Vec<ShellProfile> {
    let etc_shells = std::fs::read_to_string("/etc/shells").ok();
    discover_shells(|key| std::env::var(key).ok(), etc_shells.as_deref())
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

    /// `discover_shells` looks in real system directories, so every test here
    /// pins what it asserts to files it created itself and only checks that
    /// machine-provided entries do not break the contract.
    fn only_under(dir: &TempDir, found: Vec<ShellProfile>) -> Vec<PathBuf> {
        found
            .into_iter()
            .filter(|profile| profile.path.starts_with(dir.path()))
            .map(|profile| profile.path)
            .collect()
    }

    #[test]
    fn the_login_shell_is_the_default_and_comes_first() {
        let dir = TempDir::new("discover-default");
        let shell = executable(&dir, "myshell");
        let found = discover_shells(
            |key| (key == "SHELL").then(|| shell.display().to_string()),
            None,
        );
        let first = found.first().expect("at least the default");
        assert_eq!(first.path, shell);
        assert!(first.is_default);
        assert_eq!(
            found.iter().filter(|profile| profile.is_default).count(),
            1,
            "exactly one entry is the one a spawn without a shell would use"
        );
    }

    #[test]
    fn etc_shells_entries_are_offered() {
        let dir = TempDir::new("discover-etc");
        let bash = executable(&dir, "bash");
        let fish = executable(&dir, "fish");
        let etc = format!(
            "# a comment\n\n{}\n{}  # trailing comment\n",
            bash.display(),
            fish.display()
        );
        let found = discover_shells(|_| None, Some(&etc));
        let paths = only_under(&dir, found);
        assert!(paths.contains(&bash), "{paths:?}");
        assert!(paths.contains(&fish), "{paths:?}");
    }

    #[test]
    fn comments_relative_paths_and_missing_files_are_skipped() {
        let dir = TempDir::new("discover-junk");
        let real = executable(&dir, "zsh");
        let plain = dir.join("not-executable");
        std::fs::write(&plain, b"x").expect("write");
        let etc = format!(
            "#{}\nbin/relative\n{}\n{}\n{}\n",
            real.display(),
            dir.join("missing").display(),
            plain.display(),
            real.display(),
        );
        let found = discover_shells(|_| None, Some(&etc));
        assert_eq!(
            only_under(&dir, found),
            vec![real],
            "a commented, relative, missing, non-executable or repeated entry is not a shell"
        );
    }

    #[test]
    fn the_same_binary_reached_by_two_paths_is_listed_once() {
        let dir = TempDir::new("discover-dedup");
        let real = executable(&dir, "zsh");
        let link = dir.join("zsh-link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &link).expect("symlink");
        let etc = format!("{}\n{}\n", real.display(), link.display());
        let found = discover_shells(|_| None, Some(&etc));
        assert_eq!(
            only_under(&dir, found),
            vec![real],
            "the first spelling wins; the same file is not offered twice"
        );
    }

    #[test]
    fn entries_after_the_default_are_ordered_by_name() {
        let dir = TempDir::new("discover-order");
        let default = executable(&dir, "zsh");
        let etc = format!(
            "{}\n{}\n{}\n",
            executable(&dir, "fish").display(),
            executable(&dir, "bash").display(),
            executable(&dir, "dash").display(),
        );
        let found = discover_shells(
            |key| (key == "SHELL").then(|| default.display().to_string()),
            Some(&etc),
        );
        let names: Vec<String> = only_under(&dir, found)
            .iter()
            .skip(1)
            .filter_map(|path| path.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted, "{names:?}");
    }

    #[test]
    fn a_machine_with_no_shell_env_and_no_etc_shells_still_reports_something() {
        // Every supported platform has /bin/sh; discovery must not depend on
        // the environment a GUI launcher gives the application.
        let found = discover_shells(|_| None, None);
        assert!(!found.is_empty());
        assert!(found.iter().all(|profile| profile.path.is_absolute()));
    }

    #[test]
    fn a_program_is_not_a_shell_because_of_its_name() {
        // The regression this list exists for: `/usr/local/bin/nu` on the
        // first machine Gritty ran on is a symlink to an npm helper, not
        // nushell. Discovery reports what `$SHELL` and `/etc/shells` say, so
        // an executable that merely has a shell's name is never offered.
        let dir = TempDir::new("discover-impostor");
        let impostor = executable(&dir, "nu");
        let found = discover_shells(|_| None, None);
        assert!(
            !found.iter().any(|profile| profile.path == impostor),
            "an unregistered program was offered as a shell"
        );
    }

    #[test]
    fn check_shell_rejects_what_cannot_be_started() {
        let dir = TempDir::new("check-shell");
        let good = executable(&dir, "myshell");
        assert!(check_shell(&good).is_ok());

        let plain = dir.join("plain");
        std::fs::write(&plain, b"x").expect("write");
        for bad in [
            PathBuf::from("bin/relative"),
            dir.join("missing"),
            plain,
            dir.path().to_path_buf(),
        ] {
            let error = check_shell(&bad).expect_err("rejected");
            assert!(error.is_caller_fault(), "{bad:?}");
        }
    }

    #[test]
    fn home_comes_from_the_environment_when_it_is_usable() {
        let dir = TempDir::new("home-env");
        let home = home_directory(|key| (key == "HOME").then(|| dir.path().display().to_string()))
            .expect("resolves");
        assert_eq!(home, dir.path());
    }

    #[test]
    fn a_relative_or_broken_home_falls_back_to_the_account() {
        let dir = TempDir::new("home-bad");
        let file = dir.join("not-a-directory");
        std::fs::write(&file, b"x").expect("write");
        for bad in [
            "relative/home".to_string(),
            dir.join("missing").display().to_string(),
            file.display().to_string(),
        ] {
            // Falls through to the password database, which every account
            // running these tests has.
            let home =
                home_directory(|key| (key == "HOME").then(|| bad.clone())).expect("resolves");
            assert!(home.is_absolute(), "{home:?}");
            assert_ne!(home.display().to_string(), bad);
        }
    }

    #[test]
    fn an_environment_without_home_still_resolves() {
        // A launcher can start an application with no HOME; "no terminal for
        // you" is not an acceptable answer to that.
        let home = home_directory(|_| None).expect("the account has a home");
        assert!(home.is_absolute());
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
