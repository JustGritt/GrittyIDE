"""Tests for scripts/agent-lease.py: the notices of LEASE-NOTIFY, and the guard
they ride on.

    python3 -m unittest scripts/test_agent_lease.py

Every test drives the real script as a subprocess, the way Claude Code runs a
hook, against a throwaway git repository — never this checkout's registry.
"""

from __future__ import annotations

import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import time
import unittest
from datetime import datetime, timedelta, timezone
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "agent-lease.py"

spec = importlib.util.spec_from_file_location("agent_lease", SCRIPT)
agent_lease = importlib.util.module_from_spec(spec)
spec.loader.exec_module(agent_lease)

ALICE = "aaaaaaaa-1111-4111-8111-111111111111"
BOB = "bbbbbbbb-2222-4222-8222-222222222222"


class Repo:
    def __init__(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name).resolve()
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        subprocess.run(["git", "-C", str(self.root), "config", "user.email", "t@t"], check=True)
        subprocess.run(["git", "-C", str(self.root), "config", "user.name", "t"], check=True)
        subprocess.run(
            ["git", "-C", str(self.root), "config", "commit.gpgsign", "false"], check=True
        )
        (self.root / "src").mkdir()
        (self.root / "src" / "app.css").write_text("a {}\n")
        (self.root / "src" / "main.ts").write_text("export {};\n")
        subprocess.run(["git", "-C", str(self.root), "add", "."], check=True)
        subprocess.run(["git", "-C", str(self.root), "commit", "-qm", "init"], check=True)
        self.env = dict(os.environ, CLAUDE_PROJECT_DIR=str(self.root))

    def run(self, *args, payload=None, timeout=30):
        return subprocess.run(
            [sys.executable, str(SCRIPT), *args],
            input=json.dumps(payload) if payload is not None else "",
            capture_output=True,
            text=True,
            env=self.env,
            timeout=timeout,
        )

    def claim(self, agent, session, *scope, force=False):
        args = ["claim", "--agent", agent, "--session", session, "--task", agent, "--scope", *scope]
        return self.run(*args, *(["--force"] if force else []))

    def release(self, agent):
        return self.run("release", "--agent", agent)

    def edit(self, session, rel):
        """What the guard sees when a session calls Edit on `rel`."""
        return self.run(
            "guard",
            payload={
                "session_id": session,
                "tool_name": "Edit",
                "tool_input": {"file_path": str(self.root / rel)},
            },
        )

    def prompt(self, session):
        return self.run("notify", payload={"session_id": session})

    def close(self):
        self.tmp.cleanup()


def context_of(result) -> str:
    if not result.stdout.strip():
        return ""
    return json.loads(result.stdout)["hookSpecificOutput"].get("additionalContext", "")


class NoticeTests(unittest.TestCase):
    def setUp(self):
        self.repo = Repo()
        # Both sessions have looked once, so rows claimed later are news to them.
        self.repo.prompt(ALICE)
        self.repo.prompt(BOB)

    def tearDown(self):
        self.repo.close()

    def test_a_refused_claim_is_told_once_when_the_lease_goes(self):
        self.assertEqual(self.repo.claim("W-alice", ALICE, "src/app.css").returncode, 0)
        refused = self.repo.claim("W-bob", BOB, "src/app.css")
        self.assertEqual(refused.returncode, 1)
        self.assertIn("you will be told when it frees", refused.stderr)

        self.assertEqual(context_of(self.repo.prompt(BOB)), "", "still held: nothing to say")
        self.repo.release("W-alice")
        told = context_of(self.repo.prompt(BOB))
        self.assertIn("`src/app.css` is free", told)
        self.assertIn("W-alice held it", told)
        self.assertEqual(context_of(self.repo.prompt(BOB)), "", "a notice is delivered once")

    def test_a_claim_given_a_session_prefix_reaches_the_hooks_full_id(self):
        self.repo.claim("W-alice", ALICE, "src/")
        self.repo.claim("W-bob", BOB[:10], "src/main.ts")
        self.repo.release("W-alice")
        self.assertIn("`src/main.ts` is free", context_of(self.repo.prompt(BOB)))

    def test_a_denied_edit_waits_and_is_told_by_the_guard_itself(self):
        self.repo.claim("W-alice", ALICE, "src/**")
        denied = json.loads(self.repo.edit(BOB, "src/main.ts").stdout)["hookSpecificOutput"]
        self.assertEqual(denied["permissionDecision"], "deny")
        self.assertNotIn("additionalContext", denied)

        self.repo.release("W-alice")
        allowed = self.repo.edit(BOB, "src/main.ts")
        self.assertIn("`src/main.ts` is free", context_of(allowed))
        self.assertNotIn("permissionDecision", allowed.stdout)

    def test_an_expired_lease_frees_a_wait_with_no_file_change(self):
        old = (datetime.now(timezone.utc) - timedelta(hours=9)).strftime("%Y-%m-%dT%H:%MZ")
        self.repo.claim("W-alice", ALICE, "src/app.css")
        self.repo.claim("W-bob", BOB, "src/app.css")
        registry = self.repo.root / "AGENT-LEASES.md"
        text = registry.read_text()
        stamp_now = text.strip().splitlines()[-1].split("|")[-2].strip()
        registry.write_text(text.replace(stamp_now, old))
        self.assertIn("`src/app.css` is free", context_of(self.repo.prompt(BOB)))

    def test_a_new_lease_over_uncommitted_edits_is_reported(self):
        self.repo.edit(BOB, "src/app.css")
        (self.repo.root / "src" / "app.css").write_text("a { color: red }\n")
        self.repo.edit(BOB, "src/main.ts")  # touched, but left unchanged

        self.repo.claim("W-alice", ALICE, "src/")
        told = context_of(self.repo.prompt(BOB))
        self.assertIn("W-alice (task: W-alice", told)
        self.assertIn("has not committed: src/app.css.", told)
        self.assertNotIn("main.ts", told, "an unchanged file is not at risk")
        self.assertEqual(context_of(self.repo.prompt(BOB)), "")

    def test_a_new_lease_over_committed_edits_says_nothing(self):
        self.repo.edit(BOB, "src/main.ts")
        self.repo.claim("W-alice", ALICE, "src/")
        self.assertEqual(context_of(self.repo.prompt(BOB)), "")

    def test_the_claimant_is_not_told_about_its_own_lease(self):
        self.repo.edit(ALICE, "src/app.css")
        (self.repo.root / "src" / "app.css").write_text("changed\n")
        self.repo.claim("W-alice", ALICE, "src/")
        self.assertEqual(context_of(self.repo.prompt(ALICE)), "")

    def test_a_forced_claim_over_your_lease_is_reported(self):
        self.repo.claim("W-bob", BOB, "src/main.ts")
        self.assertEqual(self.repo.claim("W-alice", ALICE, "src/**", force=True).returncode, 0)
        told = context_of(self.repo.prompt(BOB))
        self.assertIn("overlapping your own lease W-bob on `src/main.ts`", told)

    def test_leases_that_predate_the_session_are_not_news(self):
        self.repo.claim("W-alice", ALICE, "src/")
        carol = "cccccccc-3333-4333-8333-333333333333"
        self.assertEqual(context_of(self.repo.prompt(carol)), "")

    def test_a_session_that_claims_what_it_waited_for_hears_nothing(self):
        self.repo.claim("W-alice", ALICE, "src/app.css")
        self.repo.claim("W-bob", BOB, "src/app.css")
        self.repo.release("W-alice")
        self.repo.claim("W-bob", BOB, "src/app.css")  # between two of its hooks
        self.assertEqual(context_of(self.repo.prompt(BOB)), "")

    def test_session_end_forgets_the_state(self):
        self.repo.claim("W-alice", ALICE, "src/app.css")
        self.repo.claim("W-bob", BOB, "src/app.css")
        self.repo.run("session-end", payload={"session_id": BOB})
        self.assertFalse(list((self.repo.root / ".git" / "agent-lease-notify").glob("bbbbbbbb*")))


class WatchTests(unittest.TestCase):
    """The Stop hook: exit 2 wakes the session, anything else must stay silent."""

    def setUp(self):
        self.repo = Repo()
        self.repo.prompt(BOB)
        self.repo.claim("W-alice", ALICE, "src/app.css")
        self.repo.claim("W-bob", BOB, "src/app.css")

    def tearDown(self):
        self.repo.close()

    def start_watch(self, *extra):
        process = subprocess.Popen(
            [sys.executable, str(SCRIPT), "watch", *extra],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env=self.repo.env,
        )
        process.stdin.write(json.dumps({"session_id": BOB}))
        process.stdin.close()

        def reap():
            if process.poll() is None:
                process.kill()
            process.wait()
            process.stdout.close()
            process.stderr.close()

        self.addCleanup(reap)
        return process

    def wait_registered(self, process, previous=None):
        state = self.repo.root / ".git" / "agent-lease-notify" / ("%s.json" % BOB)
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            try:
                token = json.loads(state.read_text()).get("watcher")
            except (OSError, ValueError):
                token = None
            if token and token != previous:
                return token
            self.assertIsNone(process.poll(), process.stderr.read() if process.poll() else "")
            time.sleep(0.05)
        self.fail("the watcher never registered")

    def test_it_wakes_with_the_notice_when_the_lease_goes(self):
        watcher = self.start_watch()
        self.wait_registered(watcher)
        time.sleep(0.5)
        self.assertIsNone(watcher.poll(), "nothing to say yet, so it keeps waiting")
        self.repo.release("W-alice")
        self.assertEqual(watcher.wait(timeout=15), 2)
        err = watcher.stderr.read()
        self.assertIn("`src/app.css` is free", err)
        self.assertEqual(watcher.stdout.read(), "")
        self.assertEqual(context_of(self.repo.prompt(BOB)), "", "and it is not repeated mid-turn")

    def test_the_next_turns_watcher_supersedes_it_silently(self):
        first = self.start_watch()
        token = self.wait_registered(first)
        second = self.start_watch()
        self.wait_registered(second, previous=token)
        self.assertEqual(first.wait(timeout=10), 0)
        self.assertEqual(first.stderr.read(), "")
        self.assertIsNone(second.poll(), "the newer watcher keeps waiting")

    def test_session_end_stops_it_silently(self):
        watcher = self.start_watch()
        self.wait_registered(watcher)
        self.repo.run("session-end", payload={"session_id": BOB})
        self.assertEqual(watcher.wait(timeout=10), 0)
        self.assertEqual(watcher.stderr.read(), "")

    def test_it_stops_silently_when_its_session_dies(self):
        # A shell stands in for Claude Code: it starts the watcher, prints the
        # watcher's pid, then is killed without any SessionEnd.
        state = self.repo.root / ".git" / "agent-lease-notify" / ("%s.json" % BOB)
        shell = subprocess.Popen(
            [
                "/bin/sh",
                "-c",
                'echo \'{"session_id": "%s"}\' | "%s" "%s" watch & echo $!; wait'
                % (BOB, sys.executable, SCRIPT),
            ],
            stdout=subprocess.PIPE,
            text=True,
            env=self.repo.env,
        )
        watcher_pid = int(shell.stdout.readline())
        deadline = time.monotonic() + 10
        while not state.exists() or "watcher" not in state.read_text():
            self.assertLess(time.monotonic(), deadline, "the watcher never registered")
            time.sleep(0.05)
        shell.kill()
        shell.wait()
        shell.stdout.close()
        deadline = time.monotonic() + 10
        while True:
            try:
                os.kill(watcher_pid, 0)
            except ProcessLookupError:
                break
            self.assertLess(time.monotonic(), deadline, "the orphaned watcher kept polling")
            time.sleep(0.1)

    def test_it_gives_up_silently(self):
        watcher = self.start_watch("--max-seconds", "0.5")
        self.assertEqual(watcher.wait(timeout=10), 0)
        self.assertEqual(watcher.stderr.read(), "")


class UncommittedTests(unittest.TestCase):
    def test_modified_and_untracked_count_clean_does_not(self):
        repo = Repo()
        try:
            (repo.root / "src" / "app.css").write_text("changed\n")
            (repo.root / "src" / "new.ts").write_text("new\n")
            found = agent_lease.uncommitted(
                repo.root, ["src/app.css", "src/main.ts", "src/new.ts"]
            )
            self.assertEqual(found, ["src/app.css", "src/new.ts"])
        finally:
            repo.close()


if __name__ == "__main__":
    unittest.main()
