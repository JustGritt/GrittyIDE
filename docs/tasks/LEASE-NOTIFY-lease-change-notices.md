# Task LEASE-NOTIFY — tell a session when the lease registry changes under it

- **Phase:** tooling
- **Status:** review
- **Depends on:** the lease registry (`scripts/agent-lease.py`, `AGENT-LEASES.md`)

## Why

Alexis asked, on 2026-09-14, for "a system or a hook for the other sessions to
indicate to the running session that the lease registry has been updated or a
file locked for edit", pointing at Claude Code's cross-session messaging.

Today a session learns about the registry only by reading it. So a session
refused `app.css` polls until the lease goes away, and a session holding
uncommitted edits in a file finds out someone else leased that file only when
its next edit is denied.

## Decision: hooks, not the inbox socket

Cross-session messaging (Claude Code ≥ 2.1.224) gives every session an inbox
socket, exported to hooks and Bash as `CLAUDE_CODE_MESSAGING_SOCKET`. The
documentation does not specify the line format a script would post, so a hook
writing to another session's socket would depend on an undocumented protocol
that can change in any release. It would also start a paid turn in the
receiving session for every post, relevant or not.

Hooks already deliver everything this needs through documented contracts:

- `hookSpecificOutput.additionalContext` on `PreToolUse` and
  `UserPromptSubmit` puts a notice into a session's context mid-turn.
- A `Stop` hook with `asyncRewake: true` runs in the background and, on exit
  code 2, wakes an idle session with the hook's stderr as a system reminder.

`SendMessage` stays what it is: one Claude asking another for something, such
as "could you release app.css?". This ticket does not replace that.

## Design

Nothing is added to `claim` or `release`'s write path except remembering a
refusal. Each session keeps a small state file, and the notifier compares the
registry with it:

- **Seen rows**: the lease rows this session has already been told about.
- **Touches**: paths this session edited (the guard sees every
  Edit/Write/NotebookEdit call).
- **Waits**: paths this session was refused, either denied by the guard or
  refused by `claim`.

A session hears about three things:

1. **Freed.** A path it waits on is no longer inside any live foreign lease.
   The cause doesn't matter: a release, a session ending, the 8h expiry, or a
   hand edit of the file.
2. **Leased over your edits.** A new foreign row covers a path it touched that
   still has uncommitted changes.
3. **Leased over your lease.** A new foreign row (a `--force` claim) overlaps
   one of its own live rows.

Delivery:

- Mid-turn: the `PreToolUse` guard and a `UserPromptSubmit` hook.
- Idle: a `Stop` hook with `asyncRewake`, which watches `AGENT-LEASES.md` and
  exits 2 with the notice.

A newer watcher for the same session supersedes the older one, so turns don't
pile up watchers.

State is machine-local, in the checkout's git directory (`.git/agent-lease-notify/`,
or the worktree's own git dir), like the registry itself: never committed.

## Allowed scope

- `scripts/agent-lease.py`
- `scripts/test_agent_lease.py` (new)
- `.claude/settings.json` (hook wiring)
- `AGENTS.md`, "Concurrent agent sessions" section (a short paragraph; shared
  file, claimed for the edit only)
- this task file

## Completion report

**Summary.** Sessions working in this checkout are now told when the lease
registry changes in a way that concerns them. There are three notices: a path
you were refused is free; a new foreign lease covers your uncommitted edits;
someone force-claimed over your lease. A notice reaches a session mid-turn or,
if it is idle, wakes it. Delivery uses only documented hook contracts.

**Files changed.**
- `scripts/agent-lease.py`:
  - New helpers: `overlaps()`, factored out of `claim`, and `row_key()`.
  - New notice state and logic: `NoticeState` (a locked per-session JSON file),
    `compute_notices`, `collect_notices`, `uncommitted`.
  - The guard now records touches and waits and adds notices to its output.
    Its deny decision is unchanged, and so is its fail-open handling.
  - New modes `notify` (UserPromptSubmit) and `watch` (Stop, asyncRewake).
  - A refused `claim` records a wait.
  - `session-end` deletes the session's state, which also stops its watcher.
- `scripts/test_agent_lease.py` (new): 17 tests. They drive the real script as
  a subprocess against a throwaway git repository, never this checkout's
  registry.
- `.claude/settings.json`: `UserPromptSubmit` → `notify` with a 10s timeout.
  `Stop` → `watch --max-seconds 28740` with `asyncRewake` and an 8h timeout.
- `AGENTS.md`: one paragraph in "Concurrent agent sessions".

**Tests actually executed.**
- `python3 -W error::ResourceWarning -m unittest scripts/test_agent_lease.py`:
  17 passed.
- Mutations, each restored and confirmed byte-identical afterwards:

  | Mutation | Tests that failed |
  |---|---|
  | A wait isn't consumed once announced | 2 |
  | Every touched file counts as uncommitted | 3 |
  | The watcher ignores a newer watcher | 1 |
  | The claimant is told about its own lease | 2 |
  | The watcher ignores losing its parent | 1 |

- **Real session probe** (`claude --settings` with a pty, a throwaway repo,
  Haiku, 2026-09-14):
  1. The session was refused `src/app.css` before it started.
  2. It answered one prompt and went idle.
  3. The lease was released from outside.
  4. The Stop watcher exited 2, and Claude Code showed "Stop hook feedback".
  5. The idle session woke by itself and replied "The lease on src/app.css has
     been released and is now available".
- **Live, in the session that built this:**
  - The guard delivered "W-turn-end-css … now leases `apps/desktop/src/app.css`,
    which covers files this session edited and has not committed".
  - A second live session (pid 77036) picked up the new Stop hook from the
    settings file without a restart; its watcher was seen running.

**Decisions.**
- Hooks, not the inbox socket; see "Decision" above.
- Changes are found by comparing the registry with what each session last
  saw, not by an event log. That catches expiry and hand edits too, and
  `claim` and `release` stay as they were.
- The uncommitted check uses `git status` and runs only when a new foreign row
  covers a touched path.
- A watcher exits silently when:
  - a newer watcher for the same session starts (token in the state file)
  - the session's state is deleted
  - its parent process changes (Claude Code crashed)
  - `--max-seconds` passes

**Known limitations.**
- The uncommitted check is per file. A file that also carries another
  session's uncommitted hunk reports edits this session already committed.
  This happened live with `app.css`.
- Bash writes are not recorded as touches (the guard can't tell their target).
- Only sessions in this checkout are notified. A worktree has its own registry
  and state, which matches the lease registry's own scope.
- The `watch` timeout (8h) and asyncRewake behaviour are measured on Claude
  Code 2.1.270. Documentation doesn't say how long an asyncRewake timeout may
  be.
- A woken session costs a turn. Notices are one-shot and only concern that
  session, so a busy registry doesn't make every session spend.

**Follow-ups.**
- A per-hunk uncommitted check (hash the touched file at touch time) would
  remove the false positive above.
- `agent-board.py` changes (a ticket taken or dropped) could use the same
  notices if that turns out to be wanted.

**Remaining risks.**
- asyncRewake is newer than the rest of the hook API. If its wake-up semantics
  change, mid-turn delivery through the guard and `notify` still works.
