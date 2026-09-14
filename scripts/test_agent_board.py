"""Tests for scripts/agent-board.py: a block reason and a note say when they
were true (BOARD-STALE-REASON).

    python3 -m unittest scripts/test_agent_board.py

The fold is tested directly on synthetic events; `list` and `show` are driven
as a subprocess against a throwaway git repository whose board log is written
by hand, so every timestamp is chosen rather than read off the clock — never
this checkout's board.
"""

from __future__ import annotations

import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "agent-board.py"

spec = importlib.util.spec_from_file_location("agent_board", SCRIPT)
agent_board = importlib.util.module_from_spec(spec)
spec.loader.exec_module(agent_board)

WALL = "Behind T102-R, the adapter."


def add(ticket_id, ts="2026-09-01T10:00Z", status="todo"):
    return {"op": "add", "id": ticket_id, "title": "a ticket", "status": status, "ts": ts}


def block(ticket_id, reason, ts):
    return {"op": "block", "id": ticket_id, "reason": reason, "ts": ts}


def move(ticket_id, to, ts, note=None):
    event = {"op": "move", "id": ticket_id, "to": to, "ts": ts}
    if note:
        event["note"] = note
    return event


class Repo:
    def __init__(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name).resolve()
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        self.env = dict(os.environ, CLAUDE_PROJECT_DIR=str(self.root))

    def write_log(self, events):
        log = self.root / ".git" / agent_board.BOARD_FILENAME
        log.write_text("".join(json.dumps(e) + "\n" for e in events), encoding="utf-8")

    def run(self, *args):
        return subprocess.run(
            [sys.executable, str(SCRIPT), *args],
            capture_output=True,
            text=True,
            env=self.env,
            timeout=30,
        )

    def cleanup(self):
        self.tmp.cleanup()


class FoldTest(unittest.TestCase):
    def test_a_block_reason_is_live_while_the_ticket_is_blocked(self):
        ticket = agent_board.resolve([add("T1"), block("T1", WALL, "2026-09-02T10:00Z")])["T1"]
        self.assertEqual(ticket["status"], "blocked")
        self.assertTrue(ticket["reason_live"])
        self.assertEqual(ticket["reason_ts"], "2026-09-02T10:00Z")

    def test_leaving_blocked_retires_the_reason_but_keeps_it(self):
        for leave in (
            move("T1", "todo", "2026-09-03T10:00Z"),
            {"op": "take", "id": "T1", "agent": "W", "session": "s", "ts": "2026-09-03T10:00Z"},
            {"op": "drop", "id": "T1", "ts": "2026-09-03T10:00Z"},
            add("T1", ts="2026-09-03T10:00Z", status="review"),
        ):
            with self.subTest(op=leave["op"]):
                events = [add("T1"), block("T1", WALL, "2026-09-02T10:00Z"), leave]
                ticket = agent_board.resolve(events)["T1"]
                self.assertNotEqual(ticket["status"], "blocked")
                self.assertFalse(ticket["reason_live"])
                self.assertEqual(ticket["reason"], WALL)

    def test_moving_back_to_blocked_without_a_reason_does_not_revive_the_old_one(self):
        events = [
            add("T1"),
            block("T1", WALL, "2026-09-02T10:00Z"),
            move("T1", "todo", "2026-09-03T10:00Z"),
            move("T1", "blocked", "2026-09-04T10:00Z"),
        ]
        ticket = agent_board.resolve(events)["T1"]
        self.assertEqual(ticket["status"], "blocked")
        self.assertFalse(ticket["reason_live"])

    def test_a_meta_edit_leaves_a_live_reason_live(self):
        events = [
            add("T1"),
            block("T1", WALL, "2026-09-02T10:00Z"),
            {"op": "meta", "id": "T1", "title": "renamed", "ts": "2026-09-03T10:00Z"},
        ]
        self.assertTrue(agent_board.resolve(events)["T1"]["reason_live"])

    def test_blocking_again_restamps_the_reason(self):
        events = [
            add("T1"),
            block("T1", "first wall", "2026-09-02T10:00Z"),
            move("T1", "todo", "2026-09-03T10:00Z"),
            block("T1", "second wall", "2026-09-05T10:00Z"),
        ]
        ticket = agent_board.resolve(events)["T1"]
        self.assertEqual(ticket["reason"], "second wall")
        self.assertEqual(ticket["reason_ts"], "2026-09-05T10:00Z")
        self.assertTrue(ticket["reason_live"])

    def test_a_note_carries_its_time_and_the_status_it_was_written_with(self):
        events = [
            add("T1"),
            move("T1", "review", "2026-09-02T10:00Z", note="at review, pr to follow"),
            move("T1", "tofix", "2026-09-03T10:00Z"),
        ]
        ticket = agent_board.resolve(events)["T1"]
        self.assertEqual(ticket["note_ts"], "2026-09-02T10:00Z")
        self.assertEqual(ticket["note_status"], "review")


class OutputTest(unittest.TestCase):
    def setUp(self):
        self.repo = Repo()

    def tearDown(self):
        self.repo.cleanup()

    def test_list_prints_a_reason_only_while_it_is_live(self):
        self.repo.write_log(
            [
                add("STILL"),
                block("STILL", "still walled", "2026-09-02T10:00Z"),
                add("FREED"),
                block("FREED", "was walled", "2026-09-02T10:00Z"),
                move("FREED", "todo", "2026-09-03T10:00Z"),
            ]
        )
        out = self.repo.run("list")
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertIn("(still walled)", out.stdout)
        self.assertNotIn("was walled", out.stdout)

    def test_show_marks_a_retired_reason_as_history_with_its_stamp(self):
        self.repo.write_log(
            [
                add("FREED"),
                block("FREED", WALL, "2026-09-02T10:00Z"),
                move("FREED", "todo", "2026-09-03T10:00Z", note="unblocked by the adapter"),
            ]
        )
        out = self.repo.run("show", "FREED")
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertIn(
            "reason   (history: blocked 2026-09-02T10:00Z, since unblocked) " + WALL, out.stdout
        )
        self.assertIn(
            "note     (2026-09-03T10:00Z, moving to todo) unblocked by the adapter", out.stdout
        )

    def test_show_prints_a_live_reason_plainly(self):
        self.repo.write_log([add("STILL"), block("STILL", WALL, "2026-09-02T10:00Z")])
        out = self.repo.run("show", "STILL")
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertIn("reason   " + WALL, out.stdout)
        self.assertNotIn("history", out.stdout)


if __name__ == "__main__":
    unittest.main()
