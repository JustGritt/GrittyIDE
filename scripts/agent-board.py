#!/usr/bin/env python3
"""The live ticket board for the agent sessions building Gritty.

`AGENT-LEASES.md` answers *may I write this file* — a question about a working
tree, and correctly per-checkout. This answers *is anyone building T070* — a
question about the work, which must be visible from every checkout at once.

State is an append-only log in the shared git directory, so every linked
worktree reads and writes the same file:

    $(git rev-parse --path-format=absolute --git-common-dir)/gritty-board.jsonl

Append-only because a lock-free log has no write path to get wrong: a small
O_APPEND write is atomic, so two sessions cannot lose each other's events. The
board state is a fold over the log, and the fold — not the writer — decides who
won a contended `take`. See `resolve()`.

The log is machine-local and never committed. `docs/tasks/README.md` holds the
durable projection of one field, status, and is written by `render --index`.
Losing the log costs only the live holders; `sync --from-index` rebuilds the
rest from the last commit.

    agent-board.py list                       # what is taken, by whom
    agent-board.py take T070 --agent W-t070 --session <uuid>
    agent-board.py move T070 --to review --pr 24
    agent-board.py drop T070
    agent-board.py sync --from-index          # bootstrap from docs/tasks/README.md
    agent-board.py render --index             # write the committed table back
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import uuid
from datetime import datetime, timedelta, timezone
from pathlib import Path

BOARD_FILENAME = "gritty-board.jsonl"
VIEW_FILENAME = "AGENT-BOARD.md"
INDEX_RELPATH = "docs/tasks/README.md"
TASKS_RELDIR = "docs/tasks"
TABLE_START = "<!-- board:table start -->"
TABLE_END = "<!-- board:table end -->"
TTL_HOURS = 8
MIN_SESSION_PREFIX = 8

# Rendered uppercase, stored and written to the index lowercase.
STATES = ("unspec", "todo", "blocked", "doing", "review", "tofix", "done", "superseded")
TERMINAL = ("done", "superseded")
ALIASES = {
    "develop": "doing",
    "wip": "doing",
    "in-progress": "doing",
    "in_progress": "doing",
    "fix": "tofix",
    "to-fix": "tofix",
    "not yet started": "todo",
    "not yet specified": "unspec",
    "unspecified": "unspec",
}

# Lane scopes come from docs/plans/execution-board.md, whose glob lists are
# already machine-readable. The task files' "## Allowed scope" sections are not:
# multiple paths per bullet, prose carrying the real constraint, bare filenames
# whose directory lives in the previous clause. Parsing those would claim a
# precision they do not have.
# A lane ending in `u` is the shell half of the lane above it, split off so the
# Rust and the React halves of one feature can be built at the same time. The
# split only works because a wire contract lands first (`C`): without it both
# halves want `packages/protocol` and neither can start.
LANE_SCOPES: dict[str, list[str]] = {
    "C": [
        "packages/protocol/src/**",
        "crates/gritty-core/src/protocol/**",
        "crates/gritty-core/src/event/**",
    ],
    "W": [
        "crates/workspace/**",
        "crates/gritty-core/src/workspace/**",
        "apps/desktop/src/features/explorer/**",
        "crates/search/**",
        "crates/gritty-core/src/search/**",
    ],
    "Wu": [
        "apps/desktop/src/features/search/**",
    ],
    "G": [
        "crates/git/**",
        "crates/gritty-core/src/git/**",
    ],
    "Gu": [
        "apps/desktop/src/features/git/**",
    ],
    "T": [
        "crates/terminal/**",
        "crates/gritty-core/src/terminal/**",
        "apps/desktop/src/features/terminal/**",
    ],
    "A": [
        "crates/agents/**",
        "crates/agent-hook/**",
        "crates/gritty-core/src/agent_session/**",
        "apps/desktop/src/features/agents/**",
        "apps/desktop/src/features/workstreams/**",
        "crates/platform/**",
    ],
    "O": [
        "crates/gritty-core/src/agent_run/**",
    ],
    "O2": [
        "crates/agent-fake/**",
    ],
    "O3": [
        "apps/desktop/src/features/runs/**",
    ],
    "D": [
        "crates/gritty-core/src/diagnostics/**",
        "apps/desktop/src/features/diagnostics/**",
    ],
}


# --------------------------------------------------------------------------
# paths and time
# --------------------------------------------------------------------------


def repo_root() -> Path:
    """The current checkout. Same convention as agent-lease.py."""
    env = os.environ.get("CLAUDE_PROJECT_DIR")
    if env:
        candidate = Path(env)
        if candidate.is_dir():
            return candidate.resolve()
    return Path(__file__).resolve().parent.parent


def board_path() -> Path:
    """The log, in the *shared* git directory — one file for every worktree.

    A linked worktree's `--git-common-dir` is the main repository's `.git`, so
    every checkout of this repository agrees on this path. Falling back to the
    checkout root is only for a source tree that is not a git repository at all.
    """
    try:
        out = subprocess.run(
            ["git", "rev-parse", "--path-format=absolute", "--git-common-dir"],
            cwd=repo_root(),
            capture_output=True,
            text=True,
            timeout=10,
            check=True,
        )
        common = out.stdout.strip()
        if common:
            return Path(common) / BOARD_FILENAME
    except (subprocess.SubprocessError, OSError):
        pass
    return repo_root() / ("." + BOARD_FILENAME)


def now_utc() -> datetime:
    return datetime.now(timezone.utc)


def stamp(when: datetime) -> str:
    return when.strftime("%Y-%m-%dT%H:%MZ")


def parse_stamp(raw: str):
    text = (raw or "").strip()
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


def age_of(raw: str, now: datetime) -> str:
    when = parse_stamp(raw)
    if when is None:
        return "?"
    minutes = int((now - when).total_seconds() // 60)
    if minutes < 60:
        return "%dm" % max(minutes, 0)
    if minutes < 60 * 48:
        return "%dh" % (minutes // 60)
    return "%dd" % (minutes // (60 * 24))


def is_stale(raw: str, now: datetime) -> bool:
    when = parse_stamp(raw)
    if when is None:
        return True
    return now - when > timedelta(hours=TTL_HOURS)


def normalise_state(raw: str) -> str:
    text = (raw or "").strip().lower()
    text = ALIASES.get(text, text)
    if text not in STATES:
        raise SystemExit(
            "unknown status %r; expected one of: %s" % (raw, ", ".join(STATES))
        )
    return text


# --------------------------------------------------------------------------
# the log
# --------------------------------------------------------------------------


def read_events() -> list[dict]:
    path = board_path()
    if not path.exists():
        return []
    events = []
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            event = json.loads(line)
        except ValueError:
            continue  # a torn or hand-edited line is skipped, never fatal
        if isinstance(event, dict) and event.get("op"):
            events.append(event)
    return events


def append_event(event: dict) -> dict:
    event.setdefault("ts", stamp(now_utc()))
    event.setdefault("nonce", uuid.uuid4().hex[:12])
    path = board_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a", encoding="utf-8") as handle:
        handle.write(json.dumps(event, ensure_ascii=False, sort_keys=True) + "\n")
    return event


def blank_ticket(ticket_id: str) -> dict:
    return {
        "id": ticket_id,
        "label": ticket_id,
        "link": None,
        "title": "",
        "phase": "",
        "depends": "–",
        "status": "unspec",
        "lane": None,
        "scope": [],
        "order": 10_000,
        "holder": None,
        "branch": None,
        "pr": None,
        "note": None,
        "reason": None,
        "by": None,
        "updated": None,
    }


def resolve(events: list[dict]) -> dict[str, dict]:
    """Fold the log into ticket state.

    The fold, not the writer, decides a contended `take`: a take on a ticket
    that already has a holder is ignored unless it carries `force`. Every
    reader of the log therefore reaches the same answer, and two sessions that
    appended a take within the same instant agree on which one won — the first
    one in the file. That is the whole concurrency design; there is no lock.
    """
    tickets: dict[str, dict] = {}
    for event in events:
        op = event.get("op")
        ticket_id = event.get("id")
        if not ticket_id:
            continue
        ticket = tickets.setdefault(ticket_id, blank_ticket(ticket_id))
        ticket["updated"] = event.get("ts") or ticket["updated"]

        if op == "add":
            for field in ("label", "link", "title", "phase", "depends", "lane", "by"):
                if event.get(field) is not None:
                    ticket[field] = event[field]
            if event.get("scope") is not None:
                ticket["scope"] = list(event["scope"])
            if event.get("order") is not None:
                ticket["order"] = event["order"]
            if event.get("status"):
                ticket["status"] = event["status"]

        elif op == "take":
            held = ticket.get("holder")
            if held and not event.get("force"):
                continue  # first take wins; the loser sees this on read-back
            ticket["holder"] = {
                "agent": event.get("agent") or "?",
                "session": event.get("session") or "-",
                "since": event.get("ts"),
                "ended": False,
            }
            if event.get("branch"):
                ticket["branch"] = event["branch"]
            if event.get("scope"):
                ticket["scope"] = list(event["scope"])
            ticket["status"] = "doing"

        elif op == "move":
            to = event.get("to")
            if to:
                ticket["status"] = to
            if event.get("pr") is not None:
                ticket["pr"] = event["pr"]
            if event.get("note"):
                ticket["note"] = event["note"]
            if event.get("by"):
                ticket["by"] = event["by"]
            if to in TERMINAL or to in ("todo", "unspec", "blocked"):
                ticket["holder"] = None

        elif op == "drop":
            # Handing back clears the working state too: a branch and a PR
            # belong to the attempt, not to the ticket.
            ticket["holder"] = None
            ticket["branch"] = None
            ticket["pr"] = None
            ticket["status"] = event.get("to") or "todo"
            if event.get("note"):
                ticket["note"] = event["note"]

        elif op == "block":
            ticket["status"] = "blocked"
            ticket["reason"] = event.get("reason")
            ticket["holder"] = None

        elif op == "session-end":
            holder = ticket.get("holder")
            if holder and holder.get("session") == event.get("session"):
                holder["ended"] = True

        elif op == "meta":
            for field in ("lane", "scope", "title", "phase", "depends", "order"):
                if event.get(field) is not None:
                    ticket[field] = event[field]

    return tickets


def ordered(tickets: dict[str, dict]) -> list[dict]:
    return sorted(tickets.values(), key=lambda t: (t.get("order", 10_000), t["id"]))


# --------------------------------------------------------------------------
# the committed index
# --------------------------------------------------------------------------

ROW = re.compile(r"^\|(?P<cells>.*)\|\s*$")
LINK = re.compile(r"^\[(?P<label>[^\]]+)\]\((?P<link>[^)]+)\)$")


def split_row(line: str):
    match = ROW.match(line.rstrip("\n"))
    if not match:
        return None
    cells = [c.strip() for c in match.group("cells").split("|")]
    if len(cells) < 5:
        return None
    if not cells[0] or cells[0].lower() == "id":
        return None
    if set(cells[0]) <= set("-: "):
        return None
    return cells


def index_path() -> Path:
    return repo_root() / INDEX_RELPATH


def parse_index() -> list[dict]:
    """Read the committed table. Bootstrap only — the board owns status after."""
    text = index_path().read_text(encoding="utf-8")
    rows = []
    for position, line in enumerate(text.splitlines()):
        cells = split_row(line)
        if not cells:
            continue
        id_cell, title, phase, depends, status = cells[0], cells[1], cells[2], cells[3], cells[4]
        link = None
        label = id_cell
        match = LINK.match(id_cell)
        if match:
            label = match.group("label")
            link = match.group("link")
        ticket_id = label.strip()
        by = None
        state = status.strip()
        if state.lower().startswith("superseded"):
            by = state[len("superseded") :].strip()
            by = by[3:].strip() if by.lower().startswith("by ") else by or None
            state = "superseded"
        rows.append(
            {
                "id": ticket_id if " " not in ticket_id else ticket_id.upper().replace(" ", "-"),
                "label": label,
                "link": link,
                "title": title,
                "phase": phase,
                "depends": depends,
                "status": normalise_state(state),
                "by": by,
                "order": position,
            }
        )
    return rows


def status_cell(ticket: dict) -> str:
    state = ticket.get("status", "unspec")
    if state == "superseded" and ticket.get("by"):
        return "superseded by %s" % ticket["by"]
    return state


def render_index_table(tickets: dict[str, dict]) -> str:
    lines = ["| ID | Task | Phase | Depends on | Status |", "|---|---|---|---|---|"]
    for ticket in ordered(tickets):
        id_cell = (
            "[%s](%s)" % (ticket["label"], ticket["link"])
            if ticket.get("link")
            else ticket["label"]
        )
        lines.append(
            "| %s | %s | %s | %s | %s |"
            % (
                id_cell,
                ticket.get("title", ""),
                ticket.get("phase", ""),
                ticket.get("depends", "–"),
                status_cell(ticket),
            )
        )
    return "\n".join(lines)


def splice_index(text: str, table: str) -> str:
    """Replace the table, whether it is already marked or still bare."""
    if TABLE_START in text and TABLE_END in text:
        head, rest = text.split(TABLE_START, 1)
        _, tail = rest.split(TABLE_END, 1)
        return "%s%s\n%s\n%s%s" % (head, TABLE_START, table, TABLE_END, tail)

    lines = text.splitlines(keepends=True)
    first = last = None
    for index, line in enumerate(lines):
        if line.lstrip().startswith("|"):
            if first is None:
                first = index
            last = index
        elif first is not None and last is not None and line.strip() == "":
            break
    if first is None:
        raise SystemExit("no table found in %s and no markers to write into" % INDEX_RELPATH)
    return "".join(lines[:first]) + "%s\n%s\n%s\n" % (TABLE_START, table, TABLE_END) + "".join(
        lines[last + 1 :]
    )


TASK_STATUS = re.compile(r"^- \*\*Status:\*\*.*$")


def rewrite_task_files(tickets: dict[str, dict], write: bool) -> list[str]:
    """Rewrite the `- **Status:**` bullet in each task file.

    Anchored to the bullet *before the first `##` heading*: T231 and T232 each
    carry a second `**Status:**` line inside `## Completion report`, describing
    what was built rather than where the task stands. A looser match rewrites
    the wrong line.
    """
    changed = []
    for ticket in tickets.values():
        link = ticket.get("link")
        if not link:
            continue
        path = repo_root() / TASKS_RELDIR / link
        if not path.exists():
            continue
        lines = path.read_text(encoding="utf-8").splitlines(keepends=True)
        wanted = "- **Status:** %s\n" % status_cell(ticket)
        for index, line in enumerate(lines):
            if line.startswith("## "):
                break
            if TASK_STATUS.match(line.rstrip("\n")):
                if line != wanted:
                    changed.append("%s: %s→ %s" % (link, line.strip(), status_cell(ticket)))
                    lines[index] = wanted
                    if write:
                        path.write_text("".join(lines), encoding="utf-8")
                break
    return changed


# --------------------------------------------------------------------------
# rendering
# --------------------------------------------------------------------------


def holder_text(ticket: dict, now: datetime) -> str:
    holder = ticket.get("holder")
    if not holder:
        return "–"
    text = holder.get("agent", "?")
    marks = []
    if holder.get("ended"):
        marks.append("session ended")
    if is_stale(holder.get("since", ""), now):
        marks.append("stale %s" % age_of(holder.get("since", ""), now))
    else:
        marks.append(age_of(holder.get("since", ""), now))
    return "%s (%s)" % (text, ", ".join(marks))


def render_view(tickets: dict[str, dict], now: datetime, compact: bool) -> str:
    buckets: dict[str, list[dict]] = {state: [] for state in STATES}
    for ticket in ordered(tickets):
        buckets.setdefault(ticket.get("status", "unspec"), []).append(ticket)

    out = []
    counts = " · ".join(
        "%s %d" % (state.upper(), len(buckets[state])) for state in STATES if buckets[state]
    )
    out.append(counts if counts else "board is empty — run: agent-board.py sync --from-index")

    show = ("doing", "tofix", "review", "blocked", "todo") if compact else STATES
    for state in show:
        rows = buckets.get(state) or []
        if not rows:
            continue
        if compact and state in ("review", "todo") and len(rows) > 6:
            rows = rows[:6]
        out.append("")
        out.append("%s" % state.upper())
        for ticket in rows:
            bits = [ticket["label"].ljust(6), (ticket.get("title") or "")[:52]]
            if ticket.get("holder"):
                bits.append("← %s" % holder_text(ticket, now))
            if ticket.get("branch"):
                bits.append("[%s]" % ticket["branch"])
            if ticket.get("pr"):
                bits.append("#%s" % ticket["pr"])
            if ticket.get("reason"):
                bits.append("(%s)" % ticket["reason"])
            out.append("  " + "  ".join(str(b) for b in bits if b))
        if compact and state in ("review", "todo") and len(buckets[state]) > 6:
            out.append("  … and %d more" % (len(buckets[state]) - 6))
    return "\n".join(out)


def render_view_markdown(tickets: dict[str, dict], now: datetime) -> str:
    out = [
        "# Agent board (git-ignored, machine-local)",
        "",
        "Live ticket state for the sessions building Gritty, shared by every",
        "worktree. Generated — do not edit. The tool is `scripts/agent-board.py`,",
        "the rule that makes it binding is in `AGENTS.md`, and the durable",
        "projection is the table in `docs/tasks/README.md`.",
        "",
        "Rendered %s." % stamp(now),
        "",
    ]
    for state in STATES:
        rows = [t for t in ordered(tickets) if t.get("status") == state]
        if not rows:
            continue
        out.append("## %s (%d)" % (state.upper(), len(rows)))
        out.append("")
        out.append("| ID | Task | Lane | Holder | Branch | PR |")
        out.append("|---|---|---|---|---|---|")
        for ticket in rows:
            out.append(
                "| %s | %s | %s | %s | %s | %s |"
                % (
                    ticket["label"],
                    ticket.get("title", ""),
                    ticket.get("lane") or "–",
                    holder_text(ticket, now),
                    ticket.get("branch") or "–",
                    ("#%s" % ticket["pr"]) if ticket.get("pr") else "–",
                )
            )
        out.append("")
    return "\n".join(out) + "\n"


# --------------------------------------------------------------------------
# commands
# --------------------------------------------------------------------------


def load() -> dict[str, dict]:
    return resolve(read_events())


def require(tickets: dict[str, dict], ticket_id: str) -> dict:
    ticket = tickets.get(ticket_id)
    if ticket is None:
        raise SystemExit(
            "no ticket %r on the board. `agent-board.py list` shows them; "
            "`agent-board.py add %s --title …` creates one." % (ticket_id, ticket_id)
        )
    return ticket


def cmd_list(args) -> int:
    tickets = load()
    now = now_utc()
    if args.status:
        wanted = normalise_state(args.status)
        tickets = {k: v for k, v in tickets.items() if v.get("status") == wanted}
    if args.lane:
        tickets = {k: v for k, v in tickets.items() if v.get("lane") == args.lane}
    if args.mine:
        session = args.session or ""
        tickets = {
            k: v
            for k, v in tickets.items()
            if v.get("holder") and session.startswith(v["holder"].get("session", "\0"))
        }
    print(render_view(tickets, now, args.compact))
    return 0


def cmd_show(args) -> int:
    tickets = load()
    ticket = require(tickets, args.id)
    now = now_utc()
    print("%s  %s" % (ticket["label"], ticket.get("title") or ""))
    print("  status   %s" % status_cell(ticket).upper())
    print("  lane     %s" % (ticket.get("lane") or "–"))
    print("  holder   %s" % holder_text(ticket, now))
    print("  branch   %s" % (ticket.get("branch") or "–"))
    print("  pr       %s" % (("#%s" % ticket["pr"]) if ticket.get("pr") else "–"))
    print("  depends  %s" % ticket.get("depends", "–"))
    print("  scope    %s" % (", ".join(ticket.get("scope") or []) or "–"))
    if ticket.get("reason"):
        print("  reason   %s" % ticket["reason"])
    if ticket.get("note"):
        print("  note     %s" % ticket["note"])
    return 0


def claim_lease(ticket: dict, args) -> None:
    scope = ticket.get("scope") or []
    if not scope:
        print(
            "  no scope recorded for %s — claim your lease by hand:\n"
            "    scripts/agent-lease.py claim --agent %s --session %s --task %s --scope …"
            % (ticket["id"], args.agent, args.session or "-", ticket["id"])
        )
        return
    lease = Path(__file__).resolve().parent / "agent-lease.py"
    if not lease.exists():
        return
    result = subprocess.run(
        [sys.executable, str(lease), "claim", "--agent", args.agent,
         "--session", args.session or "-", "--task", ticket["id"], "--scope", *scope],
        capture_output=True,
        text=True,
    )
    for line in (result.stdout or result.stderr).splitlines():
        print("  lease: %s" % line)


def release_lease(agent: str) -> None:
    lease = Path(__file__).resolve().parent / "agent-lease.py"
    if not agent or not lease.exists():
        return
    subprocess.run(
        [sys.executable, str(lease), "release", "--agent", agent],
        capture_output=True,
        text=True,
    )


def cmd_take(args) -> int:
    tickets = load()
    ticket = require(tickets, args.id)
    holder = ticket.get("holder")
    now = now_utc()

    if holder and not args.force:
        stale = is_stale(holder.get("since", ""), now)
        print(
            "%s is held by %s%s"
            % (
                ticket["label"],
                holder_text(ticket, now),
                ".\n  It looks stale — `--force` takes it, and the force is logged."
                if stale
                else ".\n  Pick another ticket, or ask the user to sequence the two sessions.",
            )
        )
        return 1
    if ticket.get("status") in TERMINAL and not args.force:
        print("%s is %s; nothing to take." % (ticket["label"], ticket["status"].upper()))
        return 1

    scope = list(args.scope) if args.scope else list(ticket.get("scope") or [])
    if not scope and ticket.get("lane"):
        scope = list(LANE_SCOPES.get(ticket["lane"], []))

    event = append_event(
        {
            "op": "take",
            "id": ticket["id"],
            "agent": args.agent,
            "session": args.session or "-",
            "branch": args.branch,
            "scope": scope or None,
            "force": bool(args.force),
        }
    )

    # Read back. The fold decides a contended take, so this is where a loser
    # finds out — not from what it just wrote.
    after = load()
    winner = (after.get(ticket["id"]) or {}).get("holder") or {}
    if winner.get("agent") != args.agent or winner.get("since") != event["ts"]:
        append_event({"op": "yield", "id": ticket["id"], "agent": args.agent,
                      "lost_to": winner.get("agent")})
        print(
            "%s went to %s — they appended first. Nothing was taken."
            % (ticket["label"], winner.get("agent", "another session"))
        )
        return 1

    print("%s → DOING, held by %s" % (ticket["label"], args.agent))
    claim_lease(after[ticket["id"]], args)
    return 0


def cmd_move(args) -> int:
    tickets = load()
    ticket = require(tickets, args.id)
    to = normalise_state(args.to)
    now = now_utc()

    if to == "done" and not args.reviewed_by:
        print(
            "an agent does not set DONE. `docs/tasks/README.md` is explicit: you\n"
            "open a PR, set the status to review, and *the reviewer flips it to\n"
            "done*. Pass --reviewed-by <name> if you are the reviewer."
        )
        return 1

    holder = ticket.get("holder")
    if holder and not args.force:
        mine = args.session and args.session.startswith(holder.get("session", "\0"))
        if not mine and not holder.get("ended"):
            print(
                "%s is held by %s. Moving another session's ticket needs --force."
                % (ticket["label"], holder_text(ticket, now))
            )
            return 1

    append_event(
        {
            "op": "move",
            "id": ticket["id"],
            "to": to,
            "pr": args.pr,
            "note": args.note,
            "by": args.reviewed_by,
            "force": bool(args.force),
        }
    )
    if to != "doing" and holder:
        release_lease(holder.get("agent", ""))
    print("%s → %s" % (ticket["label"], to.upper()))
    return 0


def cmd_drop(args) -> int:
    tickets = load()
    ticket = require(tickets, args.id)
    holder = ticket.get("holder") or {}
    append_event({"op": "drop", "id": ticket["id"], "note": args.note,
                  "to": normalise_state(args.to) if args.to else "todo"})
    release_lease(holder.get("agent", ""))
    print("%s → %s, unheld" % (ticket["label"], (args.to or "todo").upper()))
    return 0


def cmd_block(args) -> int:
    tickets = load()
    ticket = require(tickets, args.id)
    append_event({"op": "block", "id": ticket["id"], "reason": args.reason})
    print("%s → BLOCKED (%s)" % (ticket["label"], args.reason))
    return 0


def cmd_add(args) -> int:
    tickets = load()
    if args.id in tickets and not args.force:
        print("%s is already on the board; use `move`/`meta` to change it." % args.id)
        return 1
    order = None
    if args.after:
        anchor = tickets.get(args.after)
        if anchor:
            order = anchor.get("order", 0) + 0.5
    append_event(
        {
            "op": "add",
            "id": args.id,
            "label": args.label or args.id,
            "link": args.link,
            "title": args.title,
            "phase": args.phase or "",
            "depends": args.depends or "–",
            "lane": args.lane,
            "status": normalise_state(args.status) if args.status else "unspec",
            "order": order,
        }
    )
    print("%s added" % args.id)
    return 0


def cmd_meta(args) -> int:
    tickets = load()
    ticket = require(tickets, args.id)
    event = {"op": "meta", "id": ticket["id"]}
    if args.lane:
        event["lane"] = args.lane
        if not args.scope and args.lane in LANE_SCOPES:
            event["scope"] = list(LANE_SCOPES[args.lane])
    for field in ("scope", "title", "phase", "depends"):
        value = getattr(args, field, None)
        if value:
            event[field] = list(value) if field == "scope" else value
    if len(event) == 2:
        print("nothing to set")
        return 1
    append_event(event)
    print("%s updated: %s" % (ticket["label"], ", ".join(k for k in event if k not in ("op", "id"))))
    return 0


def cmd_sync(args) -> int:
    rows = parse_index()
    tickets = load()
    added, drift = [], []
    for row in rows:
        known = tickets.get(row["id"])
        if known is None:
            added.append(row)
        elif known.get("status") != row["status"]:
            drift.append((row["id"], row["status"], known.get("status")))

    if args.from_index and added:
        for row in added:
            append_event({"op": "add", **row})
        print("imported %d ticket(s) from %s" % (len(added), INDEX_RELPATH))
    elif added:
        print("%d ticket(s) in %s are not on the board:" % (len(added), INDEX_RELPATH))
        for row in added:
            print("  %s  %s" % (row["id"].ljust(8), row["title"][:60]))
        print("  run `agent-board.py sync --from-index` to import them")

    # The other direction: a ticket `add`ed on the board that the committed
    # index has never heard of. T326 arrived this way — it existed only in the
    # dependency graph, with no row and no file.
    in_index = {row["id"] for row in rows}
    extra = [t for t in ordered(tickets) if t["id"] not in in_index]

    if drift:
        print("%d ticket(s) disagree with the committed index:" % len(drift))
        for ticket_id, index_state, board_state in drift:
            print("  %s  index=%s  board=%s" % (ticket_id.ljust(8), index_state, board_state))
        print("  the board wins; `render --index` writes it back")
    if extra:
        print("%d ticket(s) on the board are not in %s:" % (len(extra), INDEX_RELPATH))
        for ticket in extra:
            print("  %s  %s" % (ticket["label"].ljust(8), (ticket.get("title") or "")[:60]))
        print("  `render --index` adds them")
    if not (added or drift or extra):
        print("board and %s agree (%d tickets)" % (INDEX_RELPATH, len(rows)))
    return 1 if (args.check and (added or drift or extra)) else 0


def cmd_render(args) -> int:
    tickets = load()
    now = now_utc()
    if not tickets:
        print("board is empty — run `agent-board.py sync --from-index` first")
        return 1

    if args.index:
        path = index_path()
        current = path.read_text(encoding="utf-8")
        wanted = splice_index(current, render_index_table(tickets))
        task_changes = rewrite_task_files(tickets, write=not args.check)
        if args.check:
            if current != wanted or task_changes:
                print("%s is stale; run `agent-board.py render --index`" % INDEX_RELPATH)
                for change in task_changes:
                    print("  %s" % change)
                return 1
            print("%s and the task files match the board" % INDEX_RELPATH)
            return 0
        if current != wanted:
            path.write_text(wanted, encoding="utf-8")
        print("wrote %s (%d rows), %d task file(s) updated"
              % (INDEX_RELPATH, len(tickets), len(task_changes)))
        for change in task_changes:
            print("  %s" % change)
        return 0

    view = repo_root() / VIEW_FILENAME
    view.write_text(render_view_markdown(tickets, now), encoding="utf-8")
    print("wrote %s" % VIEW_FILENAME)
    return 0


def cmd_log(args) -> int:
    for event in read_events():
        if args.id and event.get("id") != args.id:
            continue
        bits = [event.get("ts", "?"), event.get("op", "?").ljust(7), (event.get("id") or "")]
        for field in ("agent", "to", "branch", "pr", "reason", "note", "lost_to", "by"):
            if event.get(field):
                bits.append("%s=%s" % (field, event[field]))
        print("  ".join(str(b) for b in bits))
    return 0


def cmd_session_end(args) -> int:
    """SessionEnd hook: annotate, never change status.

    A half-finished branch is worth a human glance, so the ticket stays DOING
    and stops looking held. Releasing it silently would hide the work.
    """
    try:
        payload = json.loads(sys.stdin.read() or "{}")
    except ValueError:
        return 0
    session = payload.get("session_id") or ""
    if not session or len(session) < MIN_SESSION_PREFIX:
        return 0
    for ticket in load().values():
        holder = ticket.get("holder")
        if holder and session.startswith(holder.get("session", "\0")):
            append_event({"op": "session-end", "id": ticket["id"], "session": holder["session"]})
    return 0


# --------------------------------------------------------------------------


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    subs = parser.add_subparsers(dest="cmd")

    p = subs.add_parser("list", help="what is taken, by whom")
    p.add_argument("--status")
    p.add_argument("--lane")
    p.add_argument("--mine", action="store_true")
    p.add_argument("--session", default=os.environ.get("CLAUDE_SESSION_ID", ""))
    p.add_argument("--compact", action="store_true")

    p = subs.add_parser("show", help="one ticket in full")
    p.add_argument("id")

    p = subs.add_parser("take", help="claim a ticket, and its lane's lease")
    p.add_argument("id")
    p.add_argument("--agent", required=True)
    p.add_argument("--session", default="-")
    p.add_argument("--branch")
    p.add_argument("--scope", nargs="+")
    p.add_argument("--force", action="store_true")

    p = subs.add_parser("move", help="change a ticket's status")
    p.add_argument("id")
    p.add_argument("--to", required=True)
    p.add_argument("--pr")
    p.add_argument("--note")
    p.add_argument("--reviewed-by", dest="reviewed_by")
    p.add_argument("--session", default="-")
    p.add_argument("--force", action="store_true")

    p = subs.add_parser("drop", help="hand a ticket back")
    p.add_argument("id")
    p.add_argument("--to")
    p.add_argument("--note")

    p = subs.add_parser("block", help="record what stops a ticket")
    p.add_argument("id")
    p.add_argument("--reason", required=True)

    p = subs.add_parser("add", help="put a ticket on the board")
    p.add_argument("id")
    p.add_argument("--title", default="")
    p.add_argument("--label")
    p.add_argument("--link")
    p.add_argument("--phase")
    p.add_argument("--depends")
    p.add_argument("--lane")
    p.add_argument("--status")
    p.add_argument("--after")
    p.add_argument("--force", action="store_true")

    p = subs.add_parser("meta", help="set a ticket's lane, scope or title")
    p.add_argument("id")
    p.add_argument("--lane")
    p.add_argument("--scope", nargs="+")
    p.add_argument("--title")
    p.add_argument("--phase")
    p.add_argument("--depends")

    p = subs.add_parser("sync", help="reconcile with the committed index")
    p.add_argument("--from-index", dest="from_index", action="store_true")
    p.add_argument("--check", action="store_true")

    p = subs.add_parser("render", help="write AGENT-BOARD.md, or the committed table")
    p.add_argument("--index", action="store_true")
    p.add_argument("--check", action="store_true")

    p = subs.add_parser("log", help="the event history")
    p.add_argument("id", nargs="?")

    subs.add_parser("session-end", help="SessionEnd hook; payload on stdin")
    return parser


def main() -> int:
    args = build_parser().parse_args()
    handlers = {
        "list": cmd_list,
        "show": cmd_show,
        "take": cmd_take,
        "move": cmd_move,
        "drop": cmd_drop,
        "block": cmd_block,
        "add": cmd_add,
        "meta": cmd_meta,
        "sync": cmd_sync,
        "render": cmd_render,
        "log": cmd_log,
        "session-end": cmd_session_end,
    }
    handler = handlers.get(args.cmd or "list")
    if handler is cmd_list and args.cmd is None:
        args = build_parser().parse_args(["list"])
    return handler(args)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except BrokenPipeError:
        sys.exit(0)
