# Task WATCH-FLAKE — two load-sensitive tests fail in the full-workspace run

- **Phase:** maintenance (found by the ADR-032 gate)
- **Status:** todo
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
alone. T070 (file watcher) sits at review with this test in it.

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

_Not started._
