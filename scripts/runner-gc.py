#!/usr/bin/env python3
"""Disk retention for `target/` and other unbounded build output (CI-RUNNER-GC).

The self-hosted macOS runner is also Alexis' dev Mac, and since CI-COST it is
the *only* place `macos` CI jobs run. Nothing has ever pruned `target/`
(`.github/workflows/ci.yml` uses `clean: false` on purpose, to avoid a cold
Rust build every job) and `git worktree` means every active agent lane grows
its **own** `target/` from cargo's unset `CARGO_TARGET_DIR` default. Measured
2026-09-06: a `cargo clean` on this machine's one long-lived `target/` freed
95.9 GiB (261,844 files) that nothing but a human noticing had ever reclaimed.

This script is deliberately stdlib-only (matches `agent-board.py` /
`agent-lease.py`) and safe-by-default: dry-run unless `--apply` is given, never
deletes anything outside an explicit named allowlist of paths inside a
verified Cargo target directory, and skips (never forces through) a target
directory a live build appears to be using.

    runner-gc.py guard  [--floor-gb N] [--path P]
        Fail loudly, before a build starts, if free space at P is under the
        floor. Exit 0 = OK to build, 1 = below floor, 2 = could not measure
        (fails safe: refuses rather than guessing).

    runner-gc.py sweep  [--apply] [--root PATH ...]
        Classify and (dry-run by default) prune build output across every
        discovered target directory: this checkout's and every linked
        `git worktree`'s. Reports a `df` delta across the whole run, not a
        sum of `du` of what it deleted (the two disagree on APFS — see the
        task file's "What was measured").

    runner-gc.py selftest
        Runs the required tests (see docs/tasks/completed/CI-RUNNER-GC-disk-retention.md,
        "Tests required") against scratch directories only — never the real
        machine's real `target/`.

Why a bespoke script rather than `cargo-sweep` / `cargo-cache`: see the
"Eviction mechanism" note in the task file's completion report. Short version:
neither tool discovers worktree-shaped fleets of target directories, prunes
`target/release/bundle` after an upload step, gates a job start on free space,
or reports a verified `df` delta — this script's actual job is that
orchestration, with a simple oldest-first-by-mtime eviction underneath it, not
a re-derivation of Cargo's own fingerprinting.
"""

from __future__ import annotations

import argparse
import fcntl
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
import unittest.mock
from dataclasses import dataclass, field
from pathlib import Path

GB = 1024**3

# A directory is trusted as a Cargo target root only if it carries Cargo's own
# markers — never by name, never by "parent of X". CACHEDIR.TAG is written by
# Cargo at the target root; `.rustc_info.json` and `.cargo-lock` are Cargo's
# own bookkeeping files there. Requiring both rules out an unrelated directory
# that merely happens to be named `target`.
CACHEDIR_MARKER = "CACHEDIR.TAG"
LOCK_MARKERS = (".rustc_info.json", ".cargo-lock")

# The only subpaths this script will ever remove something from. Enforced in
# code (assert_safe), not just by convention: plan_* functions can only ever
# enqueue paths built by joining these names onto a verified target root, and
# assert_safe re-checks that invariant right before every deletion.
ALLOWED_PROFILES = ("debug", "release")
ALLOWED_SUBDIRS = ("incremental", "deps", "build", ".fingerprint")
# release/bundle is handled separately (see plan_bundle): keep-most-recent,
# not budget/age eviction, and only ever pruned by the caller after it has
# confirmed the current run's artifact already uploaded.


# --------------------------------------------------------------------------
# Discovery: every target-shaped root this machine has, named explicitly.
# --------------------------------------------------------------------------


def repo_worktrees() -> list[Path]:
    """Every checkout sharing this repo's .git: main tree first, then each
    `git worktree`. Works from any of them — they share one git-common-dir."""
    try:
        out = subprocess.run(
            ["git", "worktree", "list", "--porcelain"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout
    except (subprocess.SubprocessError, OSError):
        return []
    roots = []
    for line in out.splitlines():
        if line.startswith("worktree "):
            roots.append(Path(line.split(" ", 1)[1]))
    return roots


def is_target_root(path: Path) -> bool:
    """Cargo's own markers, not a name match. See module docstring."""
    try:
        if not path.is_dir():
            return False
        if not (path / CACHEDIR_MARKER).is_file():
            return False
        return any((path / marker).exists() for marker in LOCK_MARKERS)
    except OSError:
        return False


def discover_target_roots(extra: list[Path] | None = None) -> list[Path]:
    roots: list[Path] = []
    for worktree in repo_worktrees():
        candidate = worktree / "target"
        if is_target_root(candidate) and candidate not in roots:
            roots.append(candidate)
    for candidate in extra or []:
        candidate = candidate.resolve()
        if is_target_root(candidate) and candidate not in roots:
            roots.append(candidate)
    return roots


# --------------------------------------------------------------------------
# Lock / in-progress detection. "Detect and skip, try again next trigger" —
# there is no safe forced case (see task file, Failure behavior).
# --------------------------------------------------------------------------


def is_build_in_progress(
    target_root: Path, other_checkouts: list[Path] | None = None
) -> tuple[bool, str]:
    """True (and why) if a live build appears to hold this target directory.

    Primary signal: Cargo's own `.cargo-lock` advisory flock — the same file
    `cargo` itself locks for the duration of a build/test invocation. We take
    a non-blocking exclusive lock ourselves; failing to get it means Cargo (or
    us, recursively) already holds it.

    Secondary, best-effort backstop: a running `cargo`/`rustc`/`tauri` process
    whose command line names this checkout. This is a heuristic, not a proof
    — a process can build without the target dir appearing in its argv — which
    is why the flock check is authoritative and this is only a backstop, the
    same honest framing `agent-lease.py`'s shell-write guard uses.

    `other_checkouts` matters because `.claude/worktrees/*` lives *inside* the
    main checkout's own directory tree: a worktree's absolute path always
    contains the main checkout's path as a string prefix. Without excluding
    them, every worktree's build would also look, wrongly, like a build in
    the main checkout. A match is only evidence for `target_root` if it is
    not better explained by one of the other, more specific checkout paths.
    """
    lock_path = target_root / ".cargo-lock"
    if lock_path.exists():
        try:
            fd = os.open(str(lock_path), os.O_RDWR)
        except OSError as exc:
            # Fail safe: can't prove it's free, so treat it as held.
            return True, f"cannot open {lock_path} ({exc}); treating as locked"
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            os.close(fd)
            return True, f"{lock_path} is held by a live cargo process"
        except OSError as exc:
            os.close(fd)
            return True, f"could not flock {lock_path} ({exc}); treating as locked"
        else:
            fcntl.flock(fd, fcntl.LOCK_UN)
            os.close(fd)

    checkout = str(target_root.parent)
    # Longer (more specific) paths first, so a nested worktree's own path is
    # checked before we fall back to attributing a match to its parent.
    more_specific = sorted(
        (str(c) for c in (other_checkouts or []) if str(c) != checkout and str(c).startswith(checkout + os.sep)),
        key=len,
        reverse=True,
    )
    try:
        ps = subprocess.run(
            ["ps", "-axo", "pid=,command="], capture_output=True, text=True, check=True
        ).stdout
    except (subprocess.SubprocessError, OSError) as exc:
        # Fail safe here too: if we cannot see the process table, we cannot
        # rule out a live build, so skip this pass rather than guess.
        return True, f"cannot enumerate processes ({exc}); treating as locked"
    needles = ("cargo", "rustc", "tauri")
    for line in ps.splitlines():
        cmd = line.strip()
        if not cmd or checkout not in cmd:
            continue
        if any(nested in cmd for nested in more_specific):
            # Explained by a nested worktree's own checkout, not this one.
            continue
        lowered = cmd.lower()
        if any(needle in lowered for needle in needles):
            return True, f"live process rooted at {checkout}: {cmd[:160]}"
    return False, ""


# --------------------------------------------------------------------------
# Classification and eviction planning. Every function here only ever builds
# paths from ALLOWED_PROFILES / ALLOWED_SUBDIRS joined onto a verified root.
# --------------------------------------------------------------------------


@dataclass
class Removal:
    path: Path
    size: int
    reason: str


def _entry_size(entry: Path) -> int:
    try:
        if entry.is_symlink() or entry.is_file():
            return entry.lstat().st_size
        total = 0
        for dirpath, _dirnames, filenames in os.walk(entry, onerror=lambda e: None):
            for name in filenames:
                fp = Path(dirpath) / name
                try:
                    total += fp.lstat().st_size
                except OSError:
                    pass
        return total
    except OSError:
        return 0


def plan_incremental(target_root: Path) -> list[Removal]:
    """CI never uses this (CARGO_INCREMENTAL=0, workflow-wide). Only local dev
    benefits from it, and losing it costs one slower next build, nothing
    correctness-relevant. Safest, highest-value prune target — always
    eligible, subject only to the lock check the caller already did."""
    out = []
    for profile in ALLOWED_PROFILES:
        d = target_root / profile / "incremental"
        if d.is_dir():
            out.append(Removal(d, _entry_size(d), "incremental cache (CI never reads this)"))
    return out


def plan_cache(
    target_root: Path, budget_bytes: int, min_age_seconds: int, now: float
) -> list[Removal]:
    """deps / build / .fingerprint, per profile: real dependency-build cache.
    Prune oldest-mtime-first, only past the budget, and never anything newer
    than min_age_seconds even if still over budget — that recency floor is a
    defense-in-depth safety margin against a build that started writing after
    this pass began enumerating but before it took the lock this script
    checks separately."""
    out: list[Removal] = []
    for profile in ALLOWED_PROFILES:
        entries: list[tuple[float, int, Path]] = []
        total = 0
        for sub in ("deps", "build", ".fingerprint"):
            d = target_root / profile / sub
            if not d.is_dir():
                continue
            try:
                children = list(d.iterdir())
            except OSError:
                continue
            for entry in children:
                try:
                    mtime = entry.lstat().st_mtime
                except OSError:
                    continue
                size = _entry_size(entry)
                total += size
                entries.append((mtime, size, entry))
        if total <= budget_bytes:
            continue
        entries.sort(key=lambda t: t[0])  # oldest first
        need = total - budget_bytes
        freed = 0
        for mtime, size, entry in entries:
            if freed >= need:
                break
            if now - mtime < min_age_seconds:
                continue  # too fresh; leave it even though we're over budget
            out.append(
                Removal(
                    entry,
                    size,
                    f"{profile} dependency cache over budget "
                    f"({total / GB:.2f} GiB > {budget_bytes / GB:.2f} GiB), oldest-first",
                )
            )
            freed += size
    return out


def plan_bundle(
    target_root: Path, keep_last: int, min_age_seconds: int, now: float
) -> list[Removal]:
    """release/bundle/**: the macos job's own deliverable. Keep the most
    recent `keep_last` per platform subdirectory; only consider entries older
    than min_age_seconds, so a bundle from the run currently in flight — which
    may not have finished uploading yet — is never a candidate. The caller is
    responsible for only invoking this after `actions/upload-artifact` has
    actually completed for the run that matters."""
    out: list[Removal] = []
    bundle_root = target_root / "release" / "bundle"
    if not bundle_root.is_dir():
        return out
    try:
        platform_dirs = [p for p in bundle_root.iterdir() if p.is_dir()]
    except OSError:
        return out
    for platform_dir in sorted(platform_dirs):
        try:
            children = list(platform_dir.iterdir())
        except OSError:
            continue
        dated = []
        for entry in children:
            try:
                mtime = entry.lstat().st_mtime
            except OSError:
                continue
            dated.append((mtime, entry))
        dated.sort(key=lambda t: t[0], reverse=True)  # newest first
        for mtime, entry in dated[keep_last:]:
            if now - mtime < min_age_seconds:
                continue
            out.append(
                Removal(
                    entry, _entry_size(entry), f"superseded bundle (keeping newest {keep_last})"
                )
            )
    return out


def assert_safe(target_root: Path, path: Path) -> None:
    """The acceptance-criteria backstop: never delete outside an explicit,
    named allowlist under a verified target root. Raises rather than removes
    if a plan somehow produced a path this does not recognise."""
    root = target_root.resolve()
    resolved = path.resolve()
    try:
        rel = resolved.relative_to(root)
    except ValueError:
        raise RuntimeError(f"refusing to remove {resolved}: not inside target root {root}")
    parts = rel.parts
    if len(parts) < 2:
        raise RuntimeError(f"refusing to remove {resolved}: not inside a recognised subpath")
    if parts[0] not in ALLOWED_PROFILES:
        raise RuntimeError(f"refusing to remove {resolved}: '{parts[0]}' is not an allowed profile")
    # bundle entries are three-plus parts deep (release/bundle/<platform>/<artifact>);
    # everything else must match a top-level allowed subdir exactly two parts deep.
    if parts[1] == "bundle":
        if len(parts) < 4:
            raise RuntimeError(f"refusing to remove {resolved}: bundle path too shallow")
        return
    if parts[1] not in ALLOWED_SUBDIRS:
        raise RuntimeError(f"refusing to remove {resolved}: '{parts[1]}' is not allowlisted")


def remove_path(path: Path) -> None:
    if path.is_symlink():
        path.unlink()
    elif path.is_dir():
        shutil.rmtree(path)
    else:
        path.unlink(missing_ok=True)


# --------------------------------------------------------------------------
# sweep: one target root
# --------------------------------------------------------------------------


@dataclass
class SweepResult:
    target_root: Path
    skipped_reason: str = ""
    removals: list[Removal] = field(default_factory=list)
    errors: list[str] = field(default_factory=list)


def sweep_one(
    target_root: Path,
    *,
    apply: bool,
    cache_budget_bytes: int,
    bundle_keep: int,
    min_age_seconds: int,
    other_checkouts: list[Path] | None = None,
) -> SweepResult:
    result = SweepResult(target_root=target_root)
    locked, why = is_build_in_progress(target_root, other_checkouts)
    if locked:
        result.skipped_reason = why
        return result

    now = time.time()
    plan = (
        plan_incremental(target_root)
        + plan_cache(target_root, cache_budget_bytes, min_age_seconds, now)
        + plan_bundle(target_root, bundle_keep, min_age_seconds, now)
    )
    for removal in plan:
        try:
            assert_safe(target_root, removal.path)
        except RuntimeError as exc:
            result.errors.append(str(exc))
            continue
        result.removals.append(removal)

    if apply:
        # Re-check the lock immediately before deleting anything: sweep_one
        # can be called well after discovery, and this is cheap insurance
        # against a build starting in the gap.
        locked, why = is_build_in_progress(target_root, other_checkouts)
        if locked:
            result.skipped_reason = f"build started mid-sweep: {why}"
            result.removals = []
            return result
        for removal in result.removals:
            try:
                remove_path(removal.path)
            except OSError as exc:
                result.errors.append(f"failed to remove {removal.path}: {exc}")
    return result


# --------------------------------------------------------------------------
# CLI
# --------------------------------------------------------------------------


def cmd_guard(args: argparse.Namespace) -> int:
    path = Path(args.path)
    try:
        usage = shutil.disk_usage(path)
    except OSError as exc:
        print(
            f"disk guard: cannot determine free space at {path} ({exc}).\n"
            "Refusing to start — a guard that fails open on a broken check is "
            "worse than no guard at all.",
            file=sys.stderr,
        )
        return 2
    free_gb = usage.free / GB
    if free_gb < args.floor_gb:
        print(
            f"disk guard: {free_gb:.1f} GiB free at {path}, floor is "
            f"{args.floor_gb:.1f} GiB — refusing to start the job.\n"
            "Run `python3 scripts/runner-gc.py sweep --apply` (or wait for the "
            "scheduled sweep) and retry.",
            file=sys.stderr,
        )
        return 1
    print(f"disk guard: {free_gb:.1f} GiB free at {path}, floor is {args.floor_gb:.1f} GiB — OK.")
    return 0


def cmd_sweep(args: argparse.Namespace) -> int:
    extra = [Path(p) for p in (args.root or [])]
    roots = discover_target_roots(extra)
    if not roots:
        print("sweep: no verified target directories found (nothing to do).")
        return 0

    volume_path = Path(args.volume_path)
    try:
        free_before = shutil.disk_usage(volume_path).free
    except OSError as exc:
        print(f"sweep: cannot measure free space at {volume_path} ({exc}).", file=sys.stderr)
        return 2

    cache_budget_bytes = int(args.cache_budget_gb * GB)
    min_age_seconds = args.min_age_minutes * 60
    all_checkouts = [root.parent for root in roots]
    results = []
    for root in roots:
        other_checkouts = [c for c in all_checkouts if c != root.parent]
        results.append(
            sweep_one(
                root,
                apply=args.apply,
                cache_budget_bytes=cache_budget_bytes,
                bundle_keep=args.bundle_keep,
                min_age_seconds=min_age_seconds,
                other_checkouts=other_checkouts,
            )
        )

    total_planned = 0
    for res in results:
        print(f"\n{res.target_root}")
        if res.skipped_reason:
            print(f"  SKIPPED — {res.skipped_reason}")
            continue
        if not res.removals:
            print("  nothing to prune")
        for removal in res.removals:
            total_planned += removal.size
            verb = "removed" if args.apply else "would remove"
            print(f"  {verb} {removal.size / GB:.3f} GiB  {removal.path}  ({removal.reason})")
        for err in res.errors:
            print(f"  ERROR: {err}", file=sys.stderr)

    print(f"\nplanned total: {total_planned / GB:.3f} GiB across {len(roots)} target root(s)")

    if args.apply:
        try:
            free_after = shutil.disk_usage(volume_path).free
        except OSError as exc:
            print(f"sweep: cannot re-measure free space at {volume_path} ({exc}).", file=sys.stderr)
            return 2
        delta_gb = (free_after - free_before) / GB
        print(
            f"df delta at {volume_path}: {free_before / GB:.2f} GiB -> "
            f"{free_after / GB:.2f} GiB free ({delta_gb:+.2f} GiB) — this is the number "
            "that counts, not the sum of what was deleted (APFS clonefile/hard-link "
            "sharing makes per-directory du an unreliable predictor)."
        )
    else:
        print("(dry run — pass --apply to actually delete anything)")

    any_errors = any(res.errors for res in results)
    return 1 if any_errors else 0


# --------------------------------------------------------------------------
# selftest — scratch directories only, never the real machine's target/.
# --------------------------------------------------------------------------


def _touch(path: Path, size: int = 1, age_seconds: float = 0.0) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(b"x" * size)
    if age_seconds:
        stamp = time.time() - age_seconds
        os.utime(path, (stamp, stamp))


def _make_fake_target(root: Path) -> Path:
    target = root / "target"
    (target / "debug" / "incremental" / "unit-a").mkdir(parents=True)
    _touch(target / "debug" / "incremental" / "unit-a" / "s.bin", 1024)
    (target / "debug" / "deps").mkdir(parents=True)
    (target / "debug" / "build").mkdir(parents=True)
    (target / "debug" / ".fingerprint").mkdir(parents=True)
    (target / "release" / "bundle" / "macos").mkdir(parents=True)
    _touch(target / CACHEDIR_MARKER, 43)
    _touch(target / ".rustc_info.json", 10)
    _touch(target / ".cargo-lock", 0)
    return target


class RunnerGcTests(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory(prefix="runner-gc-selftest-")
        self.root = Path(self._tmp.name)

    def tearDown(self) -> None:
        self._tmp.cleanup()

    def test_never_deletes_outside_allowlist(self) -> None:
        target = _make_fake_target(self.root)
        fixtures = self.root / "fixtures" / "large-repo"
        fixtures.mkdir(parents=True)
        _touch(fixtures / "keepme.bin", 4096)
        _touch(self.root / "Cargo.lock", 10)
        _touch(self.root / "pnpm-lock.yaml", 10)
        gitdir = self.root / ".git" / "objects"
        gitdir.mkdir(parents=True)
        _touch(gitdir / "deadbeef", 10)

        # Old dependency cache entries, well past any budget, so a real sweep
        # would want to prune the *target*-scoped ones — but must never touch
        # the sibling paths above.
        old = time.time() - 3600
        _touch(target / "debug" / "deps" / "libfoo.rlib", 10 * 1024 * 1024, age_seconds=3600)
        os.utime(target / "debug" / "deps", (old, old))

        result = sweep_one(
            target,
            apply=True,
            cache_budget_bytes=0,
            bundle_keep=1,
            min_age_seconds=1,
        )
        self.assertEqual(result.errors, [])
        self.assertTrue(fixtures.exists(), "fixtures/ must survive")
        self.assertTrue((fixtures / "keepme.bin").exists())
        self.assertTrue((self.root / "Cargo.lock").exists())
        self.assertTrue((self.root / "pnpm-lock.yaml").exists())
        self.assertTrue((gitdir / "deadbeef").exists())

    def test_dry_run_deletes_nothing(self) -> None:
        target = _make_fake_target(self.root)
        old = time.time() - 3600
        big = target / "debug" / "deps" / "libfoo.rlib"
        _touch(big, 10 * 1024 * 1024, age_seconds=3600)
        os.utime(target / "debug" / "deps", (old, old))
        incr = target / "debug" / "incremental" / "unit-a"

        before = sorted(p for p in target.rglob("*"))
        result = sweep_one(
            target, apply=False, cache_budget_bytes=0, bundle_keep=1, min_age_seconds=1
        )
        after = sorted(p for p in target.rglob("*"))
        self.assertEqual(before, after, "dry run must not delete anything")
        self.assertTrue(incr.exists())
        self.assertTrue(big.exists())
        self.assertGreater(len(result.removals), 0, "dry run should still report what it would do")

    def test_apply_prunes_incremental_and_over_budget_cache(self) -> None:
        target = _make_fake_target(self.root)
        incr = target / "debug" / "incremental" / "unit-a" / "s.bin"
        old = time.time() - 3600
        big = target / "debug" / "deps" / "libfoo.rlib"
        _touch(big, 10 * 1024 * 1024, age_seconds=3600)
        os.utime(target / "debug" / "deps", (old, old))
        recent = target / "debug" / "deps" / "libbar.rlib"
        _touch(recent, 10 * 1024 * 1024, age_seconds=1)

        sweep_one(target, apply=True, cache_budget_bytes=1024, bundle_keep=1, min_age_seconds=30)

        self.assertFalse(incr.exists(), "incremental must be pruned unconditionally")
        self.assertFalse(big.exists(), "old over-budget deps entry must be pruned")
        self.assertTrue(recent.exists(), "entry newer than min-age must survive even over budget")

    def test_bundle_keeps_newest_n(self) -> None:
        target = _make_fake_target(self.root)
        bundle_dir = target / "release" / "bundle" / "macos"
        old_app = bundle_dir / "Gritty-old.app"
        new_app = bundle_dir / "Gritty-new.app"
        _touch(old_app, 10, age_seconds=3600)
        _touch(new_app, 10, age_seconds=30)

        sweep_one(target, apply=True, cache_budget_bytes=10**12, bundle_keep=1, min_age_seconds=1)

        self.assertFalse(old_app.exists())
        self.assertTrue(new_app.exists())

    def test_skips_locked_target(self) -> None:
        target = _make_fake_target(self.root)
        _touch(target / "debug" / "incremental" / "unit-a" / "s.bin", 10)
        lock_path = target / ".cargo-lock"
        fd = os.open(str(lock_path), os.O_RDWR)
        fcntl.flock(fd, fcntl.LOCK_EX)
        try:
            locked, why = is_build_in_progress(target)
            self.assertTrue(locked)
            self.assertIn(".cargo-lock", why)
            result = sweep_one(
                target, apply=True, cache_budget_bytes=0, bundle_keep=1, min_age_seconds=0
            )
            self.assertTrue(result.skipped_reason)
            self.assertEqual(result.removals, [])
            self.assertTrue((target / "debug" / "incremental" / "unit-a" / "s.bin").exists())
        finally:
            fcntl.flock(fd, fcntl.LOCK_UN)
            os.close(fd)

    def test_nested_worktree_build_is_not_mistaken_for_parent_checkout(self) -> None:
        """A worktree living under the main checkout's own tree (as
        `.claude/worktrees/*` does) has a path that literally contains the
        main checkout's path as a string prefix. A naive substring match on
        `ps` output would see a build running in the *nested* worktree and
        wrongly conclude the *parent* checkout is also mid-build. Regression
        test for that false positive — see the completion report."""
        main_checkout = self.root / "main"
        main_target = _make_fake_target(main_checkout)
        nested_checkout = main_checkout / ".claude" / "worktrees" / "agent-xyz"
        _make_fake_target(nested_checkout)

        fake_cmd = f"cargo build --release --manifest-path {nested_checkout}/Cargo.toml"
        with unittest.mock.patch(
            "subprocess.run",
            return_value=subprocess.CompletedProcess(
                args=["ps"], returncode=0, stdout=f"1234 {fake_cmd}\n"
            ),
        ):
            locked, why = is_build_in_progress(main_target, other_checkouts=[nested_checkout])
            self.assertFalse(
                locked, f"main checkout wrongly reported locked by nested worktree's build: {why}"
            )
            # The nested worktree itself must still see its own build as live.
            locked, why = is_build_in_progress(
                nested_checkout / "target", other_checkouts=[main_checkout]
            )
            self.assertTrue(locked)

    def test_is_target_root_requires_cargo_markers(self) -> None:
        plain = self.root / "not_a_target"
        plain.mkdir()
        _touch(plain / "some_file.txt", 1)
        self.assertFalse(is_target_root(plain))

        cachedir_only = self.root / "half_marked"
        cachedir_only.mkdir()
        _touch(cachedir_only / CACHEDIR_MARKER, 1)
        self.assertFalse(is_target_root(cachedir_only), "CACHEDIR.TAG alone is not enough")

        real = _make_fake_target(self.root / "real_project")
        self.assertTrue(is_target_root(real))

    def test_disk_guard_fails_safe_on_unmeasurable_path(self) -> None:
        missing = self.root / "does" / "not" / "exist" / "at" / "all"
        with self.assertRaises(OSError):
            shutil.disk_usage(missing)


def cmd_selftest(_args: argparse.Namespace) -> int:
    loader = unittest.TestLoader()
    suite = loader.loadTestsFromTestCase(RunnerGcTests)
    runner = unittest.TextTestRunner(verbosity=2)
    outcome = runner.run(suite)
    return 0 if outcome.wasSuccessful() else 1


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Disk retention for target/ and other build output (CI-RUNNER-GC)."
    )
    sub = parser.add_subparsers(dest="command", required=True)

    guard = sub.add_parser("guard", help="refuse to start a job if free space is below a floor")
    guard.add_argument("--path", default=str(Path.home()), help="path on the volume to check")
    guard.add_argument(
        "--floor-gb",
        type=float,
        default=25.0,
        # Measured 2026-09-06, this worktree, cold: a from-scratch `pnpm
        # check` (debug) plus `pnpm tauri build --bundles app --ci` (release,
        # the macos job's own step) peaked at 4.5 GiB debug + ~1.24 GiB
        # release + 16 MiB bundle ≈ 5.7 GiB combined in target/. 25 GiB is
        # ~4.4x that measured full-clean-build peak — headroom for node_modules
        # (281M measured), a second concurrent lane's incremental churn before
        # the next sweep, and the guard's own point: refuse well before the
        # edge, not at it.
        help="minimum free GiB required to proceed (see task file for how this was measured)",
    )
    guard.set_defaults(func=cmd_guard)

    sweep = sub.add_parser("sweep", help="classify and (dry-run by default) prune build output")
    sweep.add_argument(
        "--apply", action="store_true", help="actually delete; default is dry-run/report-only"
    )
    sweep.add_argument(
        "--root",
        action="append",
        default=[],
        help="an additional explicit target-directory path to include (repeatable)",
    )
    sweep.add_argument(
        "--cache-budget-gb",
        type=float,
        default=12.0,
        # Measured 2026-09-06, in this worktree, cold: a full `pnpm check`
        # (fmt, clippy --all-targets, cargo test --workspace incl. doctests)
        # left target/debug/{deps,build,.fingerprint} at 3.1G + 360M + 13M =
        # ~3.47 GiB. 12 GiB is ~3.5x that single-build figure, room for
        # several dependency bumps between sweeps without every pass
        # forcing a cold rebuild. Release wasn't independently measured to
        # the same rigor (see completion report); this budget applies to
        # each profile separately, so it covers both.
        help="per profile (debug/release), per target root: deps+build+.fingerprint budget",
    )
    sweep.add_argument(
        "--bundle-keep", type=int, default=1, help="release/bundle entries to keep, newest first"
    )
    sweep.add_argument(
        "--min-age-minutes",
        type=int,
        default=15,
        help="never prune anything touched more recently than this, even if over budget",
    )
    sweep.add_argument(
        "--volume-path",
        default=str(Path.home()),
        help="path used to measure the df delta this run reclaimed",
    )
    sweep.set_defaults(func=cmd_sweep)

    selftest = sub.add_parser("selftest", help="run the required tests against scratch trees")
    selftest.set_defaults(func=cmd_selftest)

    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
