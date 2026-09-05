# Task T051 — Launcher resume rail

- **Phase:** 0/1B
- **Status:** review
- **Depends on:** T049 (startup screen), T060 (session persistence)

## Objective

Artboard 3b: when the launcher is on screen, say what the last window had open
and offer it back in one click — but only when that is something the recent
workspaces list does not already say.

## Dependencies

- T049 for the launcher and the workspace history.
- T060 for the session database, the persisted tabs and the recovery snapshots.

## Required reading

- `docs/architecture/persistence.md`, `docs/decisions/ADR-014-session-persistence.md`
- `apps/desktop/src/features/session/{restore-flow.ts,use-session.ts}`
- design artboard 3b

## Allowed scope

- `crates/gritty-core/src/session/{mod,wire}.rs`, `protocol/{mod,export}.rs`
- `apps/desktop/src-tauri/src/{commands,lib}.rs`
- `packages/protocol/src/{client,commands,types}.ts` + tests
- `apps/desktop/src/features/{launcher,session,explorer}/**`, `App.tsx`, `app.css`
- `docs/architecture/persistence.md`, `docs/tasks/README.md`, this file

## Explicit non-goals

- **No change to the restore policy.** The core still reopens the last
  workspace by itself; this task does not make restoring opt-in, and ADR-014
  is untouched.
- No shells, agent worktrees or "Restore all vs Files only" as the artboard
  draws them — Gritty persists neither, so the distinction would be a lie.
- Nothing is deleted: Dismiss hides the rail, it does not discard the session.

## Required behavior

1. `session.lastSession` returns the workspace that is *not* open and the tabs
   it had, or `null` when a workspace is open, when there is no database, or
   when the last workspace had no tabs.
2. The launcher shows the rail whenever the core offers one — which covers all
   three ways of reaching the launcher: the last root was missing at startup,
   persistence was unavailable, or the user closed the workspace mid-run.
3. Restore opens the workspace and reopens its tabs, active tab first, with
   view modes and cursors. "Open without them" opens the folder only.
4. A root that has gone offers no Restore button and says why.
5. Unsaved buffers are counted and named as such: restoring keeps them dirty.

## Failure behavior

- No database, or the command fails: no rail, no error. The rail is an offer,
  and the session notices already explain a persistence outage.
- A tab whose file is gone is reported through the existing "skipped" notice.
- `workspace.open` refusing (dirty buffers elsewhere, missing root) is reported
  by the explorer as it already is; no tabs are opened.

## Acceptance criteria

- Closing a workspace with tabs open puts the rail on the launcher; Restore
  brings the same tabs back in the same order with the same active tab.
- Opening a workspace makes the rail disappear, and closing it again brings a
  fresh one.
- The rail never appears for a workspace that had no tabs.
- `pnpm check` green.

## Tests required

- Core: `last_session` returns `None` while open / with no tabs / without a
  database; returns the workspace, tabs and unsaved count after a restart;
  reports a missing root.
- Protocol client: the command takes no arguments and passes `null` through.
- `describeLastSession` wording.
- Manual: the round trip in the running app.

## Commands to run

```
pnpm check
pnpm dev   # manual pass
```

## Completion report

### Summary

`session.lastSession` (protocol 2.4.0 → 2.5.0, additive) plus the launcher rail
that renders it. The rail answers the question the launcher could not: *what
did not come back, and why*.

The chosen rule — from the design discussion — is that the rail appears only
when it has something the recents list does not already say. The core enforces
that by returning `null` when the last workspace had no tabs, so the renderer
never has to decide whether the rail is worth showing.

No restore-policy change. The core still reopens the last workspace by itself;
this is about the times it did not.

### Files changed

| File | What |
|---|---|
| `crates/gritty-core/src/session/wire.rs` | `LastSession { workspace, tabs, unsaved }`, reusing `RecentWorkspace` |
| `crates/gritty-core/src/session/mod.rs` | `SessionService::last_session` + three tests |
| `protocol/{mod,export}.rs`, `src-tauri/src/{commands,lib}.rs` | the command, protocol 2.5.0 |
| `packages/protocol/src/{client,commands,types}.ts` (+ test) | `session.lastSession()` |
| `features/session/restore-flow.ts` | `applyTabs` extracted from `runRestore` |
| `features/session/use-session.ts` | `resumeTabs` on the controller |
| `features/launcher/use-last-session.ts` | the hook, re-asking whenever no workspace is open |
| `features/launcher/{launcher.tsx,recents.ts}` (+ test) | the rail and its wording |
| `features/explorer/use-explorer.ts` | `openFolder` returns the workspace it opened |
| `App.tsx`, `app.css` | wiring and styles |

### Tests actually executed

- `cargo test -p gritty-core` — 163 passed (3 new).
- `pnpm vitest run` — 320 passed (2 new).
- `pnpm check` — green.

**Not executed: the manual pass.** No screen access on this machine, so the
round trip (close a workspace with tabs → rail appears → Restore → same tabs,
same active tab) has not been walked in the running app.

### Architectural decisions

1. **A separate command, not a field on `session.restore`.** Restore runs once
   per WebView and reopens the last workspace itself; by the time the launcher
   is showing, the useful question is a different one. A separate command can
   also be asked again after a mid-run close, which restore never can.
2. **The core decides whether there is anything to offer.** Returning `null`
   for a tabless workspace keeps "the rail earns its place" in one place
   instead of as a renderer heuristic.
3. **`applyTabs` is shared with the startup restore.** Two code paths that
   reopen tabs differently would eventually diverge; there is one.
4. **`RecentWorkspace` is reused inside `LastSession`** rather than a parallel
   set of name/displayRoot/exists fields.
5. **Dismiss hides, it does not discard.** ADR-014: unsaved work is never
   discarded as a side effect, and neither is a tab list.
6. **`openFolder` now returns the workspace.** The caller needed the id
   immediately rather than waiting for React state to come back around.

### Known limitations

- No manual verification yet (above).
- The rail shows the *most recent* workspace, which after a mid-run close is
  the one just closed. If several were opened in one run, only the last is
  offered — the rest are in the history list.
- "Open without them" leaves the tabs persisted, so opening the same workspace
  later still offers them. That is deliberate but might read as sticky.
- Terminal sessions are not part of it: T090 does not persist them per
  workspace, so the artboard's "2 shells" has no data behind it.

### Follow-up tasks

- Persist terminal sessions per workspace, which would make the artboard's
  "N shells" line true.
- The rail could name *why* the last session did not come back (root missing
  vs. persistence off vs. you closed it); today the session notices say that
  separately.

### Remaining risks

- `last_session` reads the newest history row rather than the session pointer,
  so a workspace opened and closed *after* the one you care about wins. That is
  the same rule the launcher's "Reopen" button already follows.
