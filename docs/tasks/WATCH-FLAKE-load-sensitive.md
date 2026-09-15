# Task WATCH-FLAKE — two load-sensitive tests fail in the full-workspace run

- **Phase:** maintenance (found by the ADR-032 gate)
- **Status:** review
- **Depends on:** —

## Objective

Two tests fail intermittently under `cargo test --workspace` and pass in
isolation. Neither is reproducible on demand. Make them deterministic without
weakening what they assert.

## The two

**1. `crates/workspace/tests/watch.rs:348`**
`a_directory_created_after_the_watch_started_is_followed` —
*"a directory created after the walk was never watched: []"*.
Failed 4 of 9 runs with ~200 concurrent build processes on the machine; passes
alone. T070 (file watcher) sat at review with this test in it when this was
filed; it is done.

**2. The desktop crate's `commands` integration test, line 598** —
`root_removed_while_open_fails_with_not_found_and_closes_the_workspace`.
Two different signatures have now been seen, which is itself evidence:

- `left Null, right "ROOT_MISSING"` (2026-09-09, first observation)
- `left ["WorkspaceOpened", "WorkspaceEntriesChanged", "WorkspaceClosed"]`,
  `right ["WorkspaceOpened", "WorkspaceClosed"]`, asserted as
  *"closed exactly once"* (2026-09-09, seen again while gating MONACO-DEFER)

The second is the more informative: an extra `WorkspaceEntriesChanged` arrives
between open and close, so the test is not seeing a spurious *close* but an
ordinary watcher event it did not budget for. Re-run in isolation immediately
afterwards: **4 of 4 green.**

## Dependencies

None. T070 is at review carrying test 1; coordinate so the fix is not written
twice.

## Required reading

- `crates/workspace/tests/watch.rs`
- the desktop crate's `commands` integration test
- The fixes for the same shape: `d7e47e2` and `cef9fd6` (the T090 output-storm
  tests), and the task files `PTY-FLAKE-process-tree-kill.md` and
  `VITEST-FLAKES-load-sensitive.md`, which are the same failure mode elsewhere.

## Allowed scope

The two test files above and the helpers they use. Production code only if the
investigation shows a real race, in which case say so rather than adjusting the
test.

## Explicit non-goals

- Raising a timeout until it passes. A fixed wait for an asynchronous delivery
  is the diagnosis, not the cure.
- `#[ignore]`, retry wrappers, or serialising the whole suite.

## Required behavior

Both tests pass under `cargo test --workspace` on a loaded machine, and still
fail if the behavior they cover regresses.

The shape of the fix in both cases is to wait for the condition rather than for
a duration: poll for the expected event with a generous deadline, and — for
test 2 — assert that the close happened *exactly once* while tolerating
unrelated events in between, rather than comparing the whole event list to a
literal.

## Failure behavior

A test that times out must say what it was waiting for and what it saw instead;
`assert_eq!` over a collected event list is what made this one hard to read.

## Acceptance criteria

- `cargo test --workspace` green in 10 consecutive runs on a machine under load
  (run a parallel build to create it).
- Each test still fails when its behavior is deliberately broken — demonstrate
  this, do not assume it.

## Tests required

The two tests themselves, plus the loaded-machine repetition above.

## Commands to run

```
cargo test -p gritty-workspace --test watch
cargo test -p gritty-desktop --test commands root_removed_while_open
cargo test --workspace
pnpm check
```

## Completion report

Worked 2026-09-14 to 15, together with `CARGO-FLAKE` (three desktop command
tests), whose first test is this ticket's second.

### Summary

Both were **test assumptions, not production races**, and both are now waits
for a condition rather than for a duration. Neither fix touches production
code. Each old failure was reproduced **deterministically** before it was
fixed, and each new test was shown to fail when the behavior it covers is
broken on purpose.

**1. `a_directory_created_after_the_watch_started_is_followed`.** The test
treated two seconds of silence (`drain()`) as proof that the pump had absorbed
`Create(late)` and registered `late/inner`. Nothing observable marks that
moment, and a write that lands before it is not late but *lost*: the backend
filters an event whose parent is not watched. Removing the silent wait makes
the old test fail **10 of 10** runs with exactly the filed signature (`[]`).
The new test keeps writing `late/inner/file-N.txt` until a batch names
`late/inner`, up to `DELIVERY_TIMEOUT`, through a new
`Observed::poke_until(what, poke, wanted)` that panics with what it waited for
and every event it saw instead. `settle()` uses the same helper, so its
timeout now says what arrived.

**2. `root_removed_while_open_fails_with_not_found_and_closes_the_workspace`.**
Written for T020, before the watcher existed. Two closers race for a removed
root and either may win: the command's presence check
(`WorkspaceManager::require`, which closes and says `details.reason =
ROOT_MISSING`) or the watcher's root check (T070, `close_missing_root`, which
closes with no correlation id, so the command then finds nothing open and
returns a plain `NotFound`). With an injected delay between `remove_dir_all`
and `workspace.list`, both filed signatures reproduce **every time**: 300 ms
gives the extra `WorkspaceEntriesChanged`, 1500 ms gives `details.reason`
null. The new test:
- asserts `NotFound`;
- asserts `workspace.current` is null **before waiting for anything**. Both
  closers hold the workspace lock, so whichever won had finished before the
  command could fail;
- waits for `WorkspaceClosed` with a deadline (`Harness::wait_for_event`, new);
- tells the closers apart by the close's `correlationId`. Equal to the error's
  means the command closed it, and the reason must be in the error's details.
  Absent means the watcher did, and the error carries no details;
- counts only `WorkspaceOpened` and `WorkspaceClosed`: exactly one each, in
  order.

### Files changed

- `crates/workspace/tests/watch.rs`: `poke_until`, `settle` on it, and test 1.
- `apps/desktop/src-tauri/tests/commands.rs`: `Harness::wait_for_event` and
  test 2.
- This file.

### Tests actually executed

- **Deterministic reproductions, before the fix.**
  - Old test 2 with a 300 ms delay: fails, extra `WorkspaceEntriesChanged`.
  - Old test 2 with a 1500 ms delay: fails, `left: Null`.
  - Old test 1 without its silent wait: fails 10/10, `saw []`.
- **The new tests under the same injections.** Test 2 passes at 0, 300 and
  1500 ms. It took the command branch at 0 and 300 ms and the watcher branch
  at 1500 ms, so both branches were exercised (checked with a temporary print,
  since removed). Test 1 writes with no wait by construction.
- **Mutations, all caught.**
  - `follow_tree` disabled (`crates/workspace/src/watch.rs`): test 1 times out
    with "saw [Changed(root)]".
  - `WorkspaceClosed` emitted twice in `emit_closed`: test 2 fails, "closed
    exactly once, in [Opened, Closed, Closed]", at both 0 and 1500 ms.
  - `require` skipping its presence check: missed by the first version of the
    new test, which passed through the watcher branch. The test was reordered
    to ask `workspace.current` before waiting, and the mutant then failed 3/3
    with "closed by neither the failing command nor the watcher".
  - Every mutation was reverted, and each file was compared against its saved
    copy.
- **Repetition by exact name.**
  - No load added by this session (load 6–27 from other sessions): test 1
    0/50, test 2 0/50.
  - Under a looping `cargo build --release --workspace` into a scratch target
    directory (load 24–57): test 1 0/50, test 2 0/50.
  - Whole watch suite: 20 runs at load 8–14, 0 failures. 60 runs with the pump
    instrumented at load 15–36, 0 failures.
  - Test 2 after its last change: 0/30.
- **`cargo test --workspace --no-fail-fast`, 10 runs** under the release-build
  loop, load 17–79. The machine slept during several of them, hence runs of up
  to 3.8 h.
  - Test 1: **passed in all 10.**
  - Test 2: **failed once (run 6)**, with `["WorkspaceOpened",
    "AgentSessionsChanged", "WorkspaceClosed"]`. Agent discovery is
    machine-wide, so a session starting anywhere lands in the harness's event
    list. The filter then was "everything except `WorkspaceEntriesChanged`";
    it now counts only the open and the close. This is the last change, and it
    was re-verified by 30 runs and the double-close mutant, **not** by another
    ten full workspace runs.
  - **The criterion "green in 10 consecutive runs" is not met as written.**
    Runs 4, 6, 8 and 10 failed on tests outside this ticket (follow-ups below).
    Runs 1, 2, 3, 5, 7 and 9 were fully green.
- `pnpm check` after the last change: **exit 0** (vitest 129 files, 1433 tests;
  cargo fmt, clippy `-D warnings` and `cargo test --workspace` green;
  generated types fresh).

### Architectural decisions

None. `poke_until` and `wait_for_event` are test helpers.

### Known limitations

- **A rarer stall in test 1 remains.** Twice, the new test saw *nothing at
  all* for 10 s across ten writes, not even the root batch for `late/`:
  - once in a first batch of 10 exact-name runs, at load around 14;
  - once in a fail-fast `cargo test --workspace` run, at load 21–46, where the
    watch suite took 43 s instead of 10.

  It did not recur in about 330 further executions, including 60 instrumented
  suite runs. This is not the diagnosed race: the root-level `Create(late)`
  was never delivered either, so the pump was either stuck in the FSEvents
  stream restart that every `backend.watch()` performs (notify 8.2 stops and
  restarts the whole stream) or the event was dropped. It is confined to the
  per-directory strategy running on macOS FSEvents, a combination only tests
  use; production runs `Recursive` on macOS. Filed as `WATCH-FSEVENTS-STALL`.
- With a live watcher, the desktop test cannot force the lazy close path on
  demand. It proves that whichever closer won kept its contract, and the
  presence-check mutant is caught by the ordering above.

### Follow-up tasks

- `WATCH-FSEVENTS-STALL`: the rare total silence above. Investigate in a `git
  worktree` with its own target directory: temporary instrumentation in a
  shared crate broke another session's `check:rust` on 2026-09-14.
- `WATCH-FLAKE-CORE`: gritty-core's own
  `root_removed_while_open_closes_the_workspace_and_fails_with_not_found`
  (`crates/gritty-core/src/workspace/mod.rs`) has the same two-closer race and
  the same exact-count assertion.
- `WATCH-PERDIR-NEWDIR`: with the per-directory strategy (Linux in production),
  a file created in a new directory before its watch attaches is never
  reported. `follow_tree` registers the directory but does not record it, or
  what it already contains, as changed. A product gap, not a test issue. The
  session planning `WATCH-DEGRADED` (say when part of the tree isn't being
  watched) has the same diagnosis.
- `WATCH-CLOSE-ORDER`: `Pump::deliver` checks `stopped` and then calls the sink
  without synchronisation, so a `WorkspaceEntriesChanged` can land just after
  `WorkspaceClosed`, contrary to `watch.rs`'s doc comment.
- Flaky tests outside this ticket, seen in the 10 workspace runs:
  - `agent-claude` `process.rs::a_child_that_dies_mid_turn_fails_and_carries_its_stderr`:
    3 of 10, plus twice more in fail-fast runs; the panic message is empty.
  - `agent-hook` `installed_command.rs::a_verdict_written_while_the_hook_waits_is_what_the_agent_reads`: 1.
  - `gritty-core` `agent_run::tests::claude_permissions::a_child_that_dies_before_anyone_answers_withdraws_the_ask`: 1.
  - `gritty-desktop` `commands.rs::a_large_search_is_cancelled_through_operations_cancel`: 1.
- Other `watch.rs` tests still assert exactly on a `drain()` (for example
  `the_per_directory_strategy_reports_the_same_changes`). None failed here, but
  they share test 1's old shape.
