#!/usr/bin/env python3
"""What is queued for `main`, and what will collide when it gets there.

Two merges have gone wrong in this repository, and both were predictable:

1. PRs #16-#18 were stacked and merged bottom-up, so each landed **in its
   parent branch** while GitHub still said MERGED and `main` had none of it.
2. #20 and #21 were opened from the same base and both bumped
   `PROTOCOL_VERSION`, edited `CoreEvent` and touched three doc indexes. The
   second one to land paid for it.

Neither is visible in `gh pr list`: it reports whether a branch *merges*, not
whether two branches are about to fight over the same line, and not whether a
base branch is another PR. `docs/merge-queue.md` records the intent — what has
to land before what, and why. This renders the live half beside it.

    scripts/merge-queue.py            # same as `status`
    scripts/merge-queue.py status     # the queue, with collisions flagged

Read-only: it runs `git fetch`, `gh pr list` and `gh pr view`, and writes
nothing. Without `gh` on the PATH or an authenticated session it says so and
exits 0, because a missing tool is not a reason to fail a shell.
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import subprocess
from dataclasses import dataclass, field
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
MAIN = "main"

# Files two branches cannot both edit without meeting in a conflict. Not "files
# that are important" — files whose *shape* makes a textual merge fail or, far
# worse, succeed with the wrong answer.
MAGNETS: dict[str, str] = {
    "crates/gritty-core/src/protocol/mod.rs": "PROTOCOL_VERSION",
    "crates/persistence/src/migrations.rs": "SCHEMA_VERSION + the migration list",
    "crates/gritty-core/src/event/mod.rs": "the CoreEvent vocabulary and its match arms",
    "crates/gritty-core/src/protocol/export.rs": "the exported type list",
    "packages/protocol/src/client.ts": "the command namespaces",
    "packages/protocol/src/commands.ts": "the command table",
    "packages/protocol/src/types.ts": "the type re-exports",
    "docs/tasks/README.md": "the task table and the dependency graph",
    "docs/decisions/README.md": "the ADR index",
    "docs/plans/roadmap.md": "the roadmap",
    "AGENTS.md": "the contract everyone reads",
    "apps/desktop/src/App.tsx": "the composition root",
    "apps/desktop/src/app.css": "one stylesheet for the whole shell",
    "Cargo.toml": "the workspace dependency list",
    "pnpm-workspace.yaml": "the pnpm catalog",
    "Cargo.lock": "a lockfile",
    "pnpm-lock.yaml": "a lockfile",
}

MAGNET_PREFIXES: dict[str, str] = {
    "packages/protocol/src/generated/": "generated types (regenerate, never hand-merge)",
    "crates/persistence/src/migrations/": "a migration number can only be used once",
}

# Constants two branches must not claim the same value for. A textual conflict
# is the *lucky* outcome here: two branches that pick the same version in
# different orders merge cleanly and lie.
VERSIONED: dict[str, tuple[str, str]] = {
    "PROTOCOL_VERSION": (
        "crates/gritty-core/src/protocol/mod.rs",
        r'PROTOCOL_VERSION:\s*&str\s*=\s*"([^"]+)"',
    ),
    "SCHEMA_VERSION": (
        "crates/persistence/src/migrations.rs",
        r"SCHEMA_VERSION:\s*i64\s*=\s*(\d+)",
    ),
}


@dataclass
class Pull:
    number: int
    title: str
    head: str
    base: str
    draft: bool
    mergeable: str
    state: str
    files: list[str] = field(default_factory=list)
    versions: dict[str, str | None] = field(default_factory=dict)

    @property
    def ref(self) -> str:
        return f"origin/{self.head}"

    def magnets(self) -> dict[str, str]:
        """The shared seams this PR touches, path → why it matters."""
        hit: dict[str, str] = {}
        for path in self.files:
            if path in MAGNETS:
                hit[path] = MAGNETS[path]
                continue
            for prefix, why in MAGNET_PREFIXES.items():
                if path.startswith(prefix):
                    hit[prefix + "**"] = why
                    break
        return hit


def run(cmd: list[str]) -> str | None:
    """Stdout, or `None` when the command is unavailable or refuses.

    Stderr is swallowed on purpose: every caller has a better message for its
    own failure than the tool's, and `git show` failing on a ref that does not
    carry a file is an ordinary answer here, not an error.
    """
    try:
        done = subprocess.run(
            cmd, cwd=REPO, capture_output=True, text=True, timeout=60, check=False
        )
    except (OSError, subprocess.SubprocessError):
        return None
    return done.stdout if done.returncode == 0 else None


def value_at(ref: str, path: str, pattern: str) -> str | None:
    """Read one constant out of a file as it stands on `ref`."""
    blob = run(["git", "show", f"{ref}:{path}"])
    if blob is None:
        return None
    found = re.search(pattern, blob)
    return found.group(1) if found else None


def load() -> list[Pull] | None:
    if shutil.which("gh") is None:
        print("gh is not on the PATH, so the live half of the queue cannot be read.")
        print("docs/merge-queue.md still has the ordering; `gh auth login` fills in the rest.")
        return None
    listing = run(
        [
            "gh", "pr", "list", "--state", "open", "--limit", "50",
            "--json", "number,title,headRefName,baseRefName,isDraft,mergeable,mergeStateStatus",
        ]
    )
    if listing is None:
        # Three ways to get here and they are not the same problem, so name all
        # three rather than guessing at one.
        print(
            "gh could not list the pull requests: not authenticated (`gh auth status`), "
            "offline, or run from outside the repository."
        )
        return None

    pulls = [
        Pull(
            number=row["number"],
            title=row["title"],
            head=row["headRefName"],
            base=row["baseRefName"],
            draft=row["isDraft"],
            mergeable=row["mergeable"],
            state=row["mergeStateStatus"],
        )
        for row in json.loads(listing)
    ]
    # Branch tips, so the version constants below are read from what is
    # actually on the remote rather than from a stale local ref.
    run(["git", "fetch", "--quiet", "origin"])
    for pull in pulls:
        files = run(["gh", "pr", "view", str(pull.number), "--json", "files"])
        if files is not None:
            pull.files = [entry["path"] for entry in json.loads(files)["files"]]
        for name, (path, pattern) in VERSIONED.items():
            pull.versions[name] = value_at(pull.ref, path, pattern)
    return pulls


def collisions(pulls: list[Pull], base: dict[str, str | None]) -> list[str]:
    """Everything two open PRs are about to fight over."""
    warnings: list[str] = []

    # 1. A base that is not main is a stack, and a stack merged bottom-up lands
    #    in its own parent while GitHub reports it merged.
    for pull in pulls:
        if pull.base != MAIN:
            warnings.append(
                f"#{pull.number} targets `{pull.base}`, not `{MAIN}`. Merge the stack "
                f"top-down, or delete each base branch as it merges, and verify with "
                f"`git rev-list --count origin/{MAIN}..origin/{pull.head}` rather than "
                f"trusting the PR state."
            )

    # 2. Two branches moving the same version constant. Same value is the
    #    dangerous case: it merges cleanly and is wrong.
    for name in VERSIONED:
        at_main = base[name]
        movers = [p for p in pulls if p.versions.get(name) not in (None, at_main)]
        if len(movers) < 2:
            continue
        claimed: dict[str, list[int]] = {}
        for pull in movers:
            claimed.setdefault(str(pull.versions[name]), []).append(pull.number)
        for value, numbers in claimed.items():
            if len(numbers) > 1:
                warnings.append(
                    f"{name}: PRs {fmt(numbers)} all claim `{value}` while {MAIN} is at "
                    f"`{at_main}`. Two branches with one version is not a conflict git can "
                    f"see — pick distinct values now."
                )
        if len(claimed) > 1:
            listed = ", ".join(f"#{n} → {v}" for v, ns in claimed.items() for n in ns)
            warnings.append(
                f"{name}: more than one open PR moves it ({listed}; {MAIN} is at "
                f"`{at_main}`). Whichever lands second must re-check the constant by hand "
                f"instead of trusting the merge."
            )

    # 3. Shared seams. A file two PRs both edit is a conflict waiting for
    #    whichever merges second; a magnet is one where the merge can also
    #    succeed and be wrong.
    for index, left in enumerate(pulls):
        for right in pulls[index + 1:]:
            shared = sorted(set(left.magnets()) & set(right.magnets()))
            if not shared:
                continue
            why = left.magnets()
            lines = "\n".join(f"      - {path} — {why[path]}" for path in shared)
            warnings.append(
                f"#{left.number} and #{right.number} both touch:\n{lines}"
            )
    return warnings


def fmt(numbers: list[int]) -> str:
    return ", ".join(f"#{n}" for n in numbers)


def base_versions() -> dict[str, str | None]:
    return {
        name: value_at(f"origin/{MAIN}", path, pattern)
        for name, (path, pattern) in VERSIONED.items()
    }


def status() -> int:
    pulls = load()
    if pulls is None:
        return 0
    if not pulls:
        print(f"Nothing is queued for `{MAIN}`.")
        return 0
    base = base_versions()

    print(f"Open pull requests against `{MAIN}` ({len(pulls)}):\n")
    for pull in sorted(pulls, key=lambda p: p.number):
        flags = []
        if pull.draft:
            flags.append("draft")
        if pull.base != MAIN:
            flags.append(f"base {pull.base}")
        suffix = f"  [{', '.join(flags)}]" if flags else ""
        print(f"  #{pull.number}  {pull.mergeable}/{pull.state}{suffix}")
        print(f"        {pull.title}")
        print(f"        {pull.head} · {len(pull.files)} files")
        seams = pull.magnets()
        if seams:
            print(f"        seams: {', '.join(sorted(seams))}")
        moved = [
            f"{name} {base[name]} → {value}"
            for name, value in pull.versions.items()
            if value is not None and value != base[name]
        ]
        if moved:
            print(f"        moves: {', '.join(moved)}")
        print()

    warnings = collisions(pulls, base)
    if not warnings:
        print("No two open PRs share a seam. Merge order is free.")
        return 0
    print(f"{len(warnings)} thing{'' if len(warnings) == 1 else 's'} to sequence:\n")
    for warning in warnings:
        print(f"  · {warning}\n")
    print("docs/merge-queue.md records the order these should land in, and why.")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "command", nargs="?", default="status", choices=["status"],
        help="what to print (only `status` today)",
    )
    parser.parse_args()
    return status()


if __name__ == "__main__":
    raise SystemExit(main())
