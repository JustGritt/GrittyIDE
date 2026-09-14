#!/usr/bin/env python3
"""File leases for the agent sessions sharing this working tree.

Several Claude sessions build Gritty at once in one checkout. `AGENT-LEASES.md`
at the repository root records who currently owns which paths; this script
maintains that file and, wired as a PreToolUse hook, denies an edit to a path
another live session holds.

The guard fails *open* on purpose. A missing or unparseable registry, an
unclaimed path, an advisory row, a stale lease, a path outside the repository —
all allowed. The only thing it refuses is a recorded, live, foreign lease.

    agent-lease.py list
    agent-lease.py claim --agent T097-blocks --session <uuid> --task T097 \
                         --scope 'apps/desktop/src/features/terminal/**'
    agent-lease.py release --agent T097-blocks
    agent-lease.py guard          # PreToolUse hook, payload on stdin
    agent-lease.py notify         # UserPromptSubmit hook, payload on stdin
    agent-lease.py watch          # Stop hook (asyncRewake), payload on stdin
    agent-lease.py session-end    # SessionEnd hook, payload on stdin

Notices (LEASE-NOTIFY). A session is told, without polling, when the registry
changes in a way that concerns it: a path it was refused is free again, a new
foreign lease covers files it edited and has not committed, or a new foreign
lease overlaps one of its own. `guard` and `notify` deliver mid-turn through
`additionalContext`; `watch` runs in the background after each turn and wakes
an idle session by exiting 2. Each session's state — the rows it has seen, the
paths it touched, the paths it waits on — is machine-local, under the
checkout's git directory.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import time
import uuid
from datetime import datetime, timedelta, timezone
from pathlib import Path

try:
    import fcntl
except ImportError:  # not POSIX: notices run unlocked rather than not at all
    fcntl = None

LEASE_FILENAME = "AGENT-LEASES.md"
HELD_HEADING = "## held now"
TTL_HOURS = 8
MIN_SESSION_PREFIX = 8

NOTIFY_DIRNAME = "agent-lease-notify"
TOUCH_LIMIT = 400
WATCH_POLL_SECONDS = 2.0
# Stale leases stop being enforced by the clock alone, with no file change, so
# a waiting watcher re-reads the registry at least this often.
WATCH_RECHECK_SECONDS = 60.0

HEADER = """# Agent leases (git-ignored, machine-local)

Live edit-ownership for the Claude sessions sharing this working tree. Not
history: this file is git-ignored, like `HANDOFF.md`. The rule that makes it
binding lives in `AGENTS.md`.

## The protocol

1. **Read this file before your first edit of a session.**
2. **Claim before you edit.** One row per lease, scoped as narrowly as the task
   actually needs:

   ```
   scripts/agent-lease.py claim --agent T097-blocks --session <your-session-uuid> \\
       --task 'T097 shell integration' \\
       --scope 'apps/desktop/src/features/terminal/**' 'crates/gritty-terminal/**'
   ```

   Your session UUID is the directory name in your own scratchpad path
   (`.../<session-id>/scratchpad`), given in your system prompt. Without it the
   row is *advisory*: it is listed for humans but enforces nothing.
3. **If a path you need sits in another agent's scope, do not edit it.** Work
   elsewhere, or tell the user so they can sequence the two sessions.
4. **Never edit another agent's row.**
5. **Release when the task is done**, when you hand back, or when you are asked
   to stop: `scripts/agent-lease.py release --agent <name>`. Sessions ending
   normally are released automatically by the SessionEnd hook.
6. **Shared magnets** — `docs/tasks/README.md`, `packages/protocol/src/generated/**`,
   `App.tsx`, `app.css`, `Cargo.toml`, lockfiles — are claimed for the minute you
   edit them and released immediately. Never hold one for the length of a task.

A `PreToolUse` hook (`scripts/agent-lease.py guard`) denies Edit/Write/NotebookEdit
and best-effort Bash writes that land inside a live foreign lease. It fails open:
it stops recorded conflicts, it does not discover unrecorded ones. Leases older
than {ttl}h stop being enforced so a crashed session cannot deadlock the tree.

Scope syntax: `path/to/file.ts` exact, `dir/` or `dir` whole subtree,
`crates/**` glob (`*` does not cross `/`, `**` does).

## Held now

| Agent | Session | Scope | Task | Claimed (UTC) |
|---|---|---|---|---|
""".format(ttl=TTL_HOURS)


def repo_root() -> Path:
    env = os.environ.get("CLAUDE_PROJECT_DIR")
    if env:
        candidate = Path(env)
        if candidate.is_dir():
            return candidate.resolve()
    return Path(__file__).resolve().parent.parent


def lease_file(root: Path) -> Path:
    return root / LEASE_FILENAME


def now_utc() -> datetime:
    return datetime.now(timezone.utc)


def stamp(when: datetime) -> str:
    return when.strftime("%Y-%m-%dT%H:%MZ")


def parse_stamp(raw: str):
    text = raw.strip()
    if not text:
        return None
    if text.endswith("Z"):
        text = text[:-1] + "+00:00"
    try:
        parsed = datetime.fromisoformat(text)
    except ValueError:
        return None
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed


def compile_scope(pattern: str):
    """Translate a scope pattern into a regex. `*` stops at `/`, `**` does not."""
    text = pattern.strip().strip("`").strip()
    while text.startswith("./"):
        text = text[2:]
    text = text.lstrip("/")
    if not text:
        return None
    if not any(ch in text for ch in "*?"):
        # A bare path: the file itself, or the whole subtree if it is a directory.
        return re.compile(re.escape(text.rstrip("/")) + r"(?:/.*)?$")
    out = []
    i = 0
    while i < len(text):
        if text.startswith("**/", i):
            out.append("(?:.*/)?")
            i += 3
        elif text.startswith("**", i):
            out.append(".*")
            i += 2
        elif text[i] == "*":
            out.append("[^/]*")
            i += 1
        elif text[i] == "?":
            out.append("[^/]")
            i += 1
        else:
            out.append(re.escape(text[i]))
            i += 1
    return re.compile("".join(out) + "$")


class Lease:
    def __init__(self, agent, session, scope_cell, task, claimed_raw, line):
        self.agent = agent
        self.session = session
        self.scope_cell = scope_cell
        self.task = task
        self.claimed_raw = claimed_raw
        self.line = line
        self.patterns = [s.strip().strip("`").strip() for s in scope_cell.split(",")]
        self.patterns = [p for p in self.patterns if p]
        self.regexes = [(p, compile_scope(p)) for p in self.patterns]
        self.regexes = [(p, rx) for p, rx in self.regexes if rx is not None]
        self.claimed = parse_stamp(claimed_raw)

    def has_session(self) -> bool:
        return (
            bool(self.session)
            and self.session != "-"
            and len(self.session) >= MIN_SESSION_PREFIX
        )

    def is_stale(self, now: datetime) -> bool:
        if self.claimed is None:
            return True
        return now - self.claimed > timedelta(hours=TTL_HOURS)

    def enforceable(self, now: datetime) -> bool:
        return self.has_session() and not self.is_stale(now) and bool(self.regexes)

    def owned_by(self, session_id: str) -> bool:
        return bool(session_id) and session_id.startswith(self.session)

    def match(self, rel_path: str):
        for pattern, rx in self.regexes:
            if rx.match(rel_path):
                return pattern
        return None

    def status(self, now: datetime) -> str:
        if not self.has_session():
            return "advisory (no session id)"
        if self.claimed is None:
            return "advisory (unreadable timestamp)"
        if self.is_stale(now):
            return "stale (> %dh)" % TTL_HOURS
        if not self.regexes:
            return "advisory (no usable scope)"
        return "live"


def row_key(lease: Lease) -> str:
    return "|".join((lease.agent, lease.session, lease.scope_cell, lease.claimed_raw))


def overlaps(scope: str, lease: Lease) -> bool:
    """Whether a path or scope pattern and a lease's scope can name the same file."""
    rx = compile_scope(scope)
    probe = literal_prefix(scope) or scope
    return bool(
        lease.match(probe.rstrip("/") or scope)
        or (rx and any(rx.match(p) for p in lease.patterns))
    )


def read_text(root: Path) -> str:
    path = lease_file(root)
    try:
        return path.read_text(encoding="utf-8")
    except OSError:
        return ""


def parse_rows(text: str):
    rows = []
    in_table = False
    for line in text.splitlines():
        stripped = line.strip()
        if stripped.startswith("## "):
            in_table = stripped.lower().startswith(HELD_HEADING)
            continue
        if not in_table or not stripped.startswith("|"):
            continue
        cells = [c.strip() for c in stripped.strip("|").split("|")]
        if len(cells) < 5:
            continue
        first = cells[0]
        if not first or first.lower() == "agent":
            continue
        if set(first) <= set("-: "):
            continue
        rows.append(Lease(first, cells[1], cells[2], cells[3], cells[4], line))
    return rows


def ensure_file(root: Path) -> Path:
    path = lease_file(root)
    if not path.exists():
        path.write_text(HEADER, encoding="utf-8")
    return path


# --------------------------------------------------------------------------- #
# notices (LEASE-NOTIFY)
# --------------------------------------------------------------------------- #


def git_dir(root: Path):
    """The checkout's own git directory, without spawning git: hooks run often."""
    dot = root / ".git"
    if dot.is_dir():
        return dot
    if dot.is_file():
        text = dot.read_text(encoding="utf-8").strip()
        if text.startswith("gitdir:"):
            path = Path(text[len("gitdir:"):].strip())
            return (path if path.is_absolute() else root / path).resolve()
    return None


def notify_dir(root: Path):
    base = git_dir(root)
    return base / NOTIFY_DIRNAME if base else None


def state_name(directory: Path, session: str) -> str:
    """The state file for a session. A claim may give a prefix of the id the
    hooks see, so a file named by either one is the same session's."""
    if len(session) >= MIN_SESSION_PREFIX and directory.is_dir():
        for entry in directory.glob("*.json"):
            name = entry.stem
            if len(name) >= MIN_SESSION_PREFIX and (
                name.startswith(session) or session.startswith(name)
            ):
                return name
    return session


class NoticeState:
    """One session's view: rows seen, paths touched, paths waited on.

    Used as a context manager, it holds an exclusive lock for the read-modify-
    write, so the guard and the watcher never deliver the same notice twice."""

    def __init__(self, root: Path, session: str):
        self.root = root
        directory = notify_dir(root)
        if directory is None or not session:
            raise OSError("no git directory or session to keep notice state in")
        directory.mkdir(parents=True, exist_ok=True)
        name = state_name(directory, session)
        self.path = directory / ("%s.json" % name)
        self.lock_path = directory / ("%s.lock" % name)
        self.data = {}
        self.dirty = False
        self._lock = None

    def __enter__(self):
        self._lock = self.lock_path.open("a")
        if fcntl is not None:
            fcntl.flock(self._lock, fcntl.LOCK_EX)
        try:
            self.data = json.loads(self.path.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            self.data = {}
        self.data.setdefault("seen", None)
        self.data.setdefault("touches", {})
        self.data.setdefault("waits", {})
        return self

    def __exit__(self, *exc):
        try:
            if self.dirty:
                tmp = self.path.with_suffix(".tmp")
                tmp.write_text(json.dumps(self.data, indent=1), encoding="utf-8")
                tmp.replace(self.path)
        finally:
            if fcntl is not None:
                fcntl.flock(self._lock, fcntl.LOCK_UN)
            self._lock.close()
        return False


def fresh(raw: str, now: datetime) -> bool:
    when = parse_stamp(raw or "")
    return when is not None and now - when <= timedelta(hours=TTL_HOURS)


def remember_waits(root: Path, session: str, conflicts, now: datetime) -> None:
    with NoticeState(root, session) as state:
        for scope, lease in conflicts:
            state.data["waits"][scope] = {"at": stamp(now), "holder": lease.agent}
        state.dirty = True


def uncommitted(root: Path, paths):
    """The paths git reports as changed or untracked; None when git cannot say."""
    try:
        out = subprocess.run(
            ["git", "-C", str(root), "status", "--porcelain", "--untracked-files=all", "--"]
            + list(paths),
            capture_output=True,
            text=True,
            timeout=10,
            check=True,
        ).stdout
    except (OSError, subprocess.SubprocessError):
        return None
    changed = set()
    for line in out.splitlines():
        entry = line[3:]
        if " -> " in entry:
            entry = entry.split(" -> ", 1)[1]
        changed.add(entry.strip().strip('"'))
    return [p for p in paths if p in changed]


def describe_lease(lease: Lease) -> str:
    return "%s (task: %s, claimed %s)" % (lease.agent, lease.task or "-", lease.claimed_raw)


def compute_notices(root: Path, session: str, state: NoticeState, now: datetime, changed=uncommitted):
    """What this session should be told now, updating what it has been told.

    Nothing is reported about rows that already existed when the session first
    looked: the guard enforces those, and a session that just started has no
    edits for them to cover."""
    data = state.data
    rows = parse_rows(read_text(root))
    live = [r for r in rows if r.enforceable(now)]
    foreign = [r for r in live if not r.owned_by(session)]
    own = [r for r in live if r.owned_by(session)]
    notices = []

    for target, info in sorted(data["waits"].items()):
        if not fresh(info.get("at", ""), now):
            del data["waits"][target]
            state.dirty = True
            continue
        if any(overlaps(target, r) for r in foreign):
            continue
        del data["waits"][target]
        state.dirty = True
        if any(overlaps(target, r) for r in own):
            continue  # this session holds it now; nothing to announce
        notices.append(
            "`%s` is free: no live lease from another session covers it any more "
            "(%s held it when you were refused at %s). Claim it before you edit it."
            % (target, info.get("holder", "another session"), info.get("at", "?"))
        )

    touches = {p: t for p, t in data["touches"].items() if fresh(t, now)}
    if len(touches) != len(data["touches"]):
        data["touches"] = touches
        state.dirty = True

    keys = sorted(row_key(r) for r in rows)
    if data["seen"] is not None:
        seen = set(data["seen"])
        for lease in foreign:
            if row_key(lease) in seen:
                continue
            mine = [r for r in own if any(overlaps(p, r) for p in lease.patterns)]
            if mine:
                notices.append(
                    "%s now leases `%s`, overlapping your own lease %s on `%s`. One of "
                    "the two was claimed with --force. Neither of you should edit the "
                    "overlap until the user sequences the two sessions."
                    % (describe_lease(lease), lease.scope_cell, mine[0].agent, mine[0].scope_cell)
                )
            covered = sorted(p for p in touches if lease.match(p))
            if covered:
                dirty = changed(root, covered)
                if dirty is None:
                    dirty = covered
                if dirty:
                    notices.append(
                        "%s now leases `%s`, which covers files this session edited and "
                        "has not committed: %s. Do not edit them again. Commit them if "
                        "that work is finished, or tell the user so they can sequence "
                        "the two sessions."
                        % (describe_lease(lease), lease.scope_cell, ", ".join(dirty))
                    )
    if data["seen"] != keys:
        data["seen"] = keys
        state.dirty = True
    return notices


def format_notices(notices) -> str:
    return "Lease registry changed (agent-lease.py, LEASE-NOTIFY):\n" + "\n".join(
        "- " + n for n in notices
    )


def collect_notices(root: Path, session: str, now: datetime, touch=None, wait=None):
    """Record this call's touch or wait, then compute notices, under one lock."""
    with NoticeState(root, session) as state:
        if touch:
            previous = parse_stamp(state.data["touches"].get(touch, ""))
            if previous is None or now - previous > timedelta(minutes=1):
                state.data["touches"][touch] = stamp(now)
                if len(state.data["touches"]) > TOUCH_LIMIT:
                    ordered = sorted(state.data["touches"].items(), key=lambda kv: kv[1])
                    state.data["touches"] = dict(ordered[-TOUCH_LIMIT:])
                state.dirty = True
        if wait:
            target, holder = wait
            state.data["waits"][target] = {"at": stamp(now), "holder": holder}
            state.dirty = True
        return compute_notices(root, session, state, now)


# --------------------------------------------------------------------------- #
# hook modes
# --------------------------------------------------------------------------- #

WRITE_VERBS = re.compile(
    r"\b(?:sed\s+-i|tee|mv|cp|rm|truncate|dd|patch|install|ln|chmod|chown"
    r"|git\s+(?:checkout|restore|apply|stash|clean))\b"
)


def command_writes(command: str) -> bool:
    cleaned = re.sub(r"\d?>\s*/dev/null", " ", command)
    cleaned = re.sub(r"\d>&\d", " ", cleaned)
    if re.search(r"(?<![0-9])>>?", cleaned):
        return True
    return bool(WRITE_VERBS.search(cleaned))


def literal_prefix(pattern: str) -> str:
    head = re.split(r"[*?]", pattern, maxsplit=1)[0]
    if "*" in pattern or "?" in pattern:
        head = head[: head.rfind("/") + 1] if "/" in head else ""
    return head


def relativize(raw: str, root: Path):
    if not raw:
        return None
    try:
        path = Path(raw)
        if not path.is_absolute():
            path = root / path
        return path.resolve().relative_to(root).as_posix()
    except (OSError, ValueError):
        return None


class Denied(Exception):
    def __init__(self, reason: str, lease: Lease, target: str):
        super().__init__(reason)
        self.reason = reason
        self.lease = lease
        self.target = target


def deny(reason: str, lease: Lease, target: str) -> None:
    raise Denied(reason, lease, target)


def hook_output(event: str, context: str = "", denial: str = "") -> None:
    specific = {"hookEventName": event}
    if denial:
        specific["permissionDecision"] = "deny"
        specific["permissionDecisionReason"] = denial
    if context:
        specific["additionalContext"] = context
    if len(specific) > 1:
        print(json.dumps({"hookSpecificOutput": specific}))


def explain(lease: Lease, pattern: str, target: str, session_id: str) -> str:
    return (
        "%s is leased by %s (task: %s, claimed %s) through the pattern `%s` in "
        "%s. Do not edit it — work on something outside their scope, or tell the "
        "user so they can sequence the two sessions. If you *are* %s, this "
        "session's id is %s: correct the Session cell in your own row."
        % (
            target,
            lease.agent,
            lease.task or "-",
            lease.claimed_raw or "-",
            pattern,
            LEASE_FILENAME,
            lease.agent,
            session_id or "(not reported)",
        )
    )


def check_guard(root: Path, session_id: str, tool: str, tool_input, now: datetime):
    """The file this call writes inside the repository, if the guard can tell;
    raises `Denied` when that write lands in a live foreign lease."""
    live = [r for r in parse_rows(read_text(root)) if r.enforceable(now)]
    foreign = [r for r in live if not r.owned_by(session_id)]

    if tool == "Bash":
        command = tool_input.get("command") or ""
        if not foreign or not command_writes(command):
            return None
        for lease in foreign:
            for pattern, _ in lease.regexes:
                head = literal_prefix(pattern)
                if len(head) >= 4 and head in command:
                    deny(
                        explain(lease, pattern, head, session_id)
                        + " (Matched the path inside a shell command that appears "
                        "to write; if this command only reads, say so and use a "
                        "read-only form.)",
                        lease,
                        pattern,
                    )
        return None

    target = tool_input.get("file_path") or tool_input.get("notebook_path") or ""
    rel = relativize(target, root)
    if rel is None or rel == LEASE_FILENAME:
        return None
    for lease in foreign:
        pattern = lease.match(rel)
        if pattern:
            deny(explain(lease, pattern, rel, session_id), lease, rel)
    return rel


def cmd_guard() -> None:
    payload = json.loads(sys.stdin.read() or "{}")
    session_id = payload.get("session_id") or ""
    tool = payload.get("tool_name") or ""
    tool_input = payload.get("tool_input") or {}
    if not session_id:
        # No caller identity means nobody can prove ownership of their own row.
        # Denying here would block the holder too, so fail open.
        return
    root = repo_root()
    now = now_utc()

    denial, touch, wait = "", None, None
    try:
        touch = check_guard(root, session_id, tool, tool_input, now)
    except Denied as refused:
        denial, wait = refused.reason, (refused.target, refused.lease.agent)

    context = ""
    try:
        notices = collect_notices(root, session_id, now, touch=touch, wait=wait)
        if notices:
            context = format_notices(notices)
    except Exception:  # notices are a courtesy; the decision above stands alone
        pass
    hook_output("PreToolUse", context=context, denial=denial)


def cmd_notify() -> None:
    """UserPromptSubmit: whatever changed while the user was typing."""
    payload = json.loads(sys.stdin.read() or "{}")
    session_id = payload.get("session_id") or ""
    if not session_id:
        return
    notices = collect_notices(repo_root(), session_id, now_utc())
    if notices:
        hook_output("UserPromptSubmit", context=format_notices(notices))


def cmd_watch(max_seconds: float) -> int:
    """Stop hook with `asyncRewake`: wait in the background for a notice.

    Exit 2 with the notice on stderr wakes the session even when it is idle.
    Every other ending is exit 0 and silent: superseded by the watcher the next
    turn started, the session ended, or `max_seconds` passed — the next prompt
    or tool call still delivers anything that arrived after that."""
    payload = json.loads(sys.stdin.read() or "{}")
    session_id = payload.get("session_id") or ""
    if not session_id:
        return 0
    root = repo_root()
    token = uuid.uuid4().hex
    handle = NoticeState(root, session_id)
    with handle as state:
        state.data["watcher"] = token
        state.dirty = True

    registry = lease_file(root)
    # Claude Code runs the hook as its direct child. A session killed without a
    # SessionEnd leaves the watcher re-parented, and it must not poll for hours.
    parent = os.getppid()
    started = last_check = time.monotonic()
    last_mtime = None
    while time.monotonic() - started < max_seconds:
        if os.getppid() != parent:
            return 0  # the session is gone
        if not handle.path.exists():
            return 0  # session-end removed the state
        try:
            mtime = registry.stat().st_mtime_ns
        except OSError:
            mtime = None
        due = time.monotonic() - last_check >= WATCH_RECHECK_SECONDS
        with handle as state:
            if state.data.get("watcher") != token:
                return 0  # a later turn's watcher took over
            notices = []
            if mtime != last_mtime or due:
                last_mtime, last_check = mtime, time.monotonic()
                notices = compute_notices(root, session_id, state, now_utc())
        if notices:
            print(format_notices(notices), file=sys.stderr)
            return 2
        time.sleep(WATCH_POLL_SECONDS)
    return 0


def cmd_session_end() -> None:
    payload = json.loads(sys.stdin.read() or "{}")
    session_id = payload.get("session_id") or ""
    if not session_id:
        return
    root = repo_root()
    try:
        # Its notice state goes with it, which also stops its watcher.
        directory = notify_dir(root)
        if directory is not None and directory.is_dir():
            name = state_name(directory, session_id)
            for suffix in (".json", ".lock", ".tmp"):
                (directory / (name + suffix)).unlink(missing_ok=True)
    except OSError:
        pass
    path = lease_file(root)
    if not path.exists():
        return
    kept, dropped = [], []
    for line in path.read_text(encoding="utf-8").splitlines(keepends=True):
        row = parse_rows("%s\n%s" % (HELD_HEADING, line))
        if row and row[0].has_session() and row[0].owned_by(session_id):
            dropped.append(row[0])
            continue
        kept.append(line)
    if dropped:
        path.write_text("".join(kept), encoding="utf-8")


# --------------------------------------------------------------------------- #
# cli modes
# --------------------------------------------------------------------------- #


def cmd_claim(args) -> int:
    root = repo_root()
    path = ensure_file(root)
    now = now_utc()
    scopes = [s.strip().strip("`").strip() for s in args.scope]
    scopes = [s for s in scopes if s]
    if not scopes:
        print("refusing to claim an empty scope", file=sys.stderr)
        return 2

    existing = parse_rows(path.read_text(encoding="utf-8"))
    if not args.force:
        conflicts = []
        for scope in scopes:
            for lease in existing:
                if lease.agent == args.agent or not lease.enforceable(now):
                    continue
                if overlaps(scope, lease):
                    conflicts.append((scope, lease))
        if conflicts:
            for scope, lease in conflicts:
                print(
                    "conflict: %s overlaps %s held by %s (task %s, claimed %s)"
                    % (scope, lease.scope_cell, lease.agent, lease.task, lease.claimed_raw),
                    file=sys.stderr,
                )
            print("nothing claimed; --force to override", file=sys.stderr)
            if args.session and args.session != "-":
                try:
                    remember_waits(root, args.session, conflicts, now)
                    print(
                        "you will be told when it frees (LEASE-NOTIFY): mid-turn, or by "
                        "a wake-up if this session is idle",
                        file=sys.stderr,
                    )
                except Exception:
                    pass
            return 1

    row = "| %s | %s | %s | %s | %s |\n" % (
        args.agent,
        args.session or "-",
        ", ".join(scopes),
        args.task or "-",
        stamp(now),
    )
    with path.open("a", encoding="utf-8") as handle:
        handle.write(row)
    note = "" if (args.session and args.session != "-") else "  (advisory: no session id)"
    print("claimed for %s: %s%s" % (args.agent, ", ".join(scopes), note))
    return 0


def cmd_release(args) -> int:
    root = repo_root()
    path = ensure_file(root)
    if not args.agent and not args.session:
        print("give --agent or --session", file=sys.stderr)
        return 2
    kept, dropped = [], []
    for line in path.read_text(encoding="utf-8").splitlines(keepends=True):
        rows = parse_rows("%s\n%s" % (HELD_HEADING, line))
        if rows:
            lease = rows[0]
            if args.agent and lease.agent == args.agent:
                dropped.append(lease)
                continue
            if args.session and lease.has_session() and lease.owned_by(args.session):
                dropped.append(lease)
                continue
        kept.append(line)
    if dropped:
        path.write_text("".join(kept), encoding="utf-8")
    for lease in dropped:
        print("released %s: %s" % (lease.agent, lease.scope_cell))
    if not dropped:
        print("no matching lease")
    return 0


def cmd_list(_args) -> int:
    root = repo_root()
    path = ensure_file(root)
    rows = parse_rows(path.read_text(encoding="utf-8"))
    if not rows:
        print("no leases held (%s)" % path)
        return 0
    now = now_utc()
    width = max(len(r.agent) for r in rows)
    for lease in rows:
        print(
            "%-*s  %-10s  %-24s  %s  [%s]"
            % (
                width,
                lease.agent,
                lease.session[:10],
                lease.task[:24],
                lease.scope_cell,
                lease.status(now),
            )
        )
    return 0


def main(argv) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="cmd", required=True)

    sub.add_parser("guard", help="PreToolUse hook; payload on stdin")
    sub.add_parser("notify", help="UserPromptSubmit hook; payload on stdin")
    watch = sub.add_parser("watch", help="Stop hook with asyncRewake; payload on stdin")
    watch.add_argument(
        "--max-seconds",
        type=float,
        default=TTL_HOURS * 3600,
        help="give up silently after this long (keep it under the hook's timeout)",
    )
    sub.add_parser("session-end", help="SessionEnd hook; payload on stdin")
    sub.add_parser("list", help="show current leases")

    claim = sub.add_parser("claim", help="take a lease")
    claim.add_argument("--agent", required=True)
    claim.add_argument("--session", default="-", help="your session UUID (or a >=8 char prefix)")
    claim.add_argument("--task", default="-")
    claim.add_argument("--scope", nargs="+", required=True)
    claim.add_argument("--force", action="store_true", help="claim despite an overlap")

    release = sub.add_parser("release", help="drop a lease")
    release.add_argument("--agent")
    release.add_argument("--session")

    args = parser.parse_args(argv)

    if args.cmd == "guard":
        try:
            cmd_guard()
        except Exception:  # fail open: never block an edit because of this script
            pass
        return 0
    if args.cmd == "notify":
        try:
            cmd_notify()
        except Exception:
            pass
        return 0
    if args.cmd == "watch":
        try:
            return cmd_watch(args.max_seconds)
        except Exception:  # a broken watcher must never wake a session
            return 0
    if args.cmd == "session-end":
        try:
            cmd_session_end()
        except Exception:
            pass
        return 0
    if args.cmd == "claim":
        return cmd_claim(args)
    if args.cmd == "release":
        return cmd_release(args)
    return cmd_list(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
