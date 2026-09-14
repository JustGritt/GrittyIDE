# Task T049 — Startup screen (design artboard 3a)

- **Phase:** 0
- **Status:** done
- **Depends on:** T020, T030, T048, T060, T095

## Objective

Give Gritty a real "no workspace open" state, from design artboard **3a
"Centered launcher"** (Claude Design project `4a56ac43`): three ways in, the
recent workspaces below, and a drop hint, on the chrome T048 built.

Until now the shell rendered the same three panes whether or not a workspace
existed. On a first run, after **Close workspace**, or when the last root had
vanished, the user faced an empty three-pane IDE whose only way in was a small
**Open…** button in the explorer header and a path field behind a `⋯` popover.

## Dependencies

The chrome and tokens are T048's; the workspace commands are T020's; the
`workspace` history table is T060's, written on every open since then and never
read. T095 is why the terminal dock must keep working with no workspace.

## Required reading

- `docs/decisions/ADR-016-design-tokens.md`, `ADR-014-session-persistence.md`,
  `ADR-009-core-protocol-conventions.md`, `ADR-010-workspace-paths.md`
- `docs/architecture/persistence.md`
- Design project `4a56ac43`: `Gritty Agent Layout.dc.html`, artboards `3a`
  (built here) and `3b` (not built — see non-goals)

## Allowed scope

`crates/persistence/**`, `crates/gritty-core/src/{workspace,session,protocol}/**`,
`apps/desktop/src-tauri/**`, `apps/desktop/src/**`, `packages/protocol/src/**`,
`docs/`.

## Explicit non-goals

- **Artboard 3b** (resume-first, with the last session and its agents on the
  right). It presumes a restore the user triggers; Gritty reopens the last
  workspace by itself (T060 behaviour 4, frozen), and changing that needs an
  ADR amendment, not a screen.
- A command palette, and therefore the ⌘K hint 3a shows.
- The git branch badge on a recents row — that is T100.
- `no agents` in the status bar — that is T200+.
- Restoring a *recent* workspace's tabs when it is reopened. `session.restore`
  is once per process; `workspace.open` has never reopened tabs and does not
  start now.

## Required behavior

1. Two additive commands over the existing `workspace` table:
   - `workspace.recents` — history, newest first, bounded by a `limit` the core
     caps. Each entry carries `root` (canonical, the key), `displayRoot` (home
     shortened to `~`), `name`, `lastOpenedAt` and `exists`.
   - `workspace.forget` — drops one root, and the last-workspace pointer with
     it when they are the same root. Idempotent.
2. `workspace.recents` never fails because persistence is unavailable: the
   history is empty and the existing notice explains. `workspace.forget` does
   fail — it is an action the user took and must not appear to succeed.
3. Forget touches nothing but the history row: tabs and recovery snapshots
   under that root stay (ADR-014).
4. The launcher replaces the sidebar-and-editor body only. The top bar, session
   notices, drop overlay, terminal dock and status bar stay mounted.
5. It appears only once the session has settled, never while the last workspace
   is being reopened.
6. A missing root is listed, struck through, unclickable, with **Forget** — not
   hidden.
7. ⌘O / Ctrl+O opens the picker anywhere in the app; ⌘⇧O / Ctrl+⇧O focuses the
   launcher's path field.
8. The top bar says "No workspace open" and the status bar says "no workspace".

## Failure behavior

- Persistence unavailable → empty history, no error, session notice unchanged.
- A recents row whose root vanished between listing and click → the core's
  `NotFound`, rendered as an error envelope; the list refetches.
- A relative or non-directory path in the field → the core's `InvalidInput`.
  The renderer validates only that the field is non-empty; duplicating the
  core's path rules would only let the two disagree.
- Forget fails → the row comes back (the optimistic removal is undone by a
  refetch) and the envelope says why.
- The core unreachable → no shortcut hints, because the shortcuts genuinely do
  not work; the status bar and health dot already say so.

## Acceptance criteria

- `pnpm check` green.
- The launcher appears on a first run, after Close workspace, and when the last
  root vanished; it never flashes during a successful restore.
- A missing root is struck through and Forget removes it, and the next launch
  no longer raises `WorkspaceRootMissing`.
- Light and dark both legible; the panel scrolls rather than clipping at 800×600.
- The terminal dock still starts a home-rooted shell with no workspace open.

## Tests required

Rust: history ordering and limit, `exists` for a directory / a file / nothing,
`displayRoot` under and outside home, Forget's pointer rule and its
non-cascade, an unavailable store, and the two adapters over the mock runtime
across a simulated restart. TypeScript: the protocol client's request shapes,
and every launcher decision (relative time, parent path, collapse/expand,
reopen candidate, path validation) plus the shortcut matcher.

No component test: `apps/desktop` runs vitest under `environment: 'node'` with
no DOM testing library, and adding one is a dependency decision outside this
task. Same position as T048, and the same known limitation.

## Commands to run

```
pnpm format
pnpm check
cargo test -p gritty-core     # rewrites packages/protocol/src/generated
GRITTY_LOG=debug pnpm dev
```

## Completion report

**Summary.** Built artboard 3a as `features/launcher`, and gave the core the
two commands it needs. The enabling fact was that the data already existed:
`crates/persistence`'s `workspace (root, last_opened_at)` table has been
written on every `WorkspaceOpened` since T060 and nothing had ever read it —
T060's own follow-up list named this work. Protocol 2.1.0 → 2.2.0 (additive).

**Files changed.**

- new: `crates/gritty-core/src/workspace/recents.rs`,
  `apps/desktop/src/features/launcher/{launcher.tsx,use-recents.ts,recents.ts,recents.test.ts}`,
  `apps/desktop/src/features/shell/{open-shortcuts.ts,open-shortcuts.test.ts,use-open-shortcuts.ts}`,
  this file
- `crates/persistence/src/store.rs` — `workspace::list` takes a limit;
  `workspace::forget` added
- `crates/gritty-core/src/workspace/mod.rs` — `RecentWorkspace`,
  `RecentWorkspacesRequest`, `ForgetWorkspaceRequest`
- `crates/gritty-core/src/session/mod.rs` — `recents` / `forget` on
  `SessionService`, which is where the database handle lives
- `crates/gritty-core/src/protocol/{mod,export}.rs` — version, exports
- `apps/desktop/src-tauri/src/{commands,lib}.rs` — the two adapters
- `packages/protocol/src/{commands,client,types}.ts` + `generated/`
- `apps/desktop/src/App.tsx` — owns the explorer controller, branches the body,
  registers the shortcuts
- `apps/desktop/src/features/explorer/{explorer-panel.tsx,use-explorer.ts}` —
  the panel takes the controller it used to create
- `apps/desktop/src/features/shell/app-bar.tsx`,
  `apps/desktop/src/features/editor/status-bar.tsx` — the empty state, named
- `apps/desktop/src/app.css`, `styles/tokens.css` — `.launcher*`, `--text-display`

**Tests actually executed.**

- `pnpm check` — green (Biome, tsc across 4 projects, 293 vitest tests,
  cargo fmt/clippy `-D warnings`, 183 cargo tests, generated-types freshness).
- New: 6 in `workspace::recents`, 2 in `session` (history across a real open,
  and an unavailable store), 2 in `crates/persistence`, 1 mock-runtime adapter
  test across a simulated restart, 10 launcher and 8 shortcut vitest cases.
- **Rendered in Chrome** against a vite dev server at 1400×900, 800×600 and
  760×430, dark and light, with the IPC stubbed to serve a seven-entry history
  including a missing root and a very long name and path. Console clean.
- **Run in the Tauri shell** on a fresh `GRITTY_DATA_DIR`: `protocol_version
  2.2.0`, `session restored workspace=None`, and `workspace.recents` served in
  371 µs and 299 µs. `useRecents` is called from nowhere but `Launcher`, so
  that call is the evidence the launcher mounted in WKWebView. No errors or
  panics in the log.

**Three defects were found by looking and fixed:** the parent-path column
rendered reversed (`/~Desktop/CurrentProjects`) because `direction: rtl`
reorders a path's neutral characters — `unicode-bidi: plaintext` only moved the
seam, so the trick was dropped for plain end-truncation; the launcher shipped a
second `<h1>` saying "Gritty" beside the app bar's, now an `<h2>`; and a
`workspace.recents` that failed on an unavailable store would have replaced the
launcher with an error envelope on exactly the run where persistence is broken.

**Architectural decisions.**

- The wire types and the logic live in `workspace`, the database handle in
  `session`. `WorkspaceManager` deliberately holds no database — persistence
  reaches it through the session tap — so giving it one for this would have
  been the larger change.
- `displayRoot` and `exists` are computed in the core because the renderer has
  no filesystem and does not know `$HOME` (hard rule 1). Home resolution reuses
  `gritty_terminal::shell::home_directory`, already used by
  `gritty_core::terminal` for the same question.
- The workspace moved from inside the explorer up into the shell. It was
  already being pushed up for the top bar; the launcher needs `openFolder` too,
  and a second `useExplorer` would have meant a second subscription and a
  second tree.
- The launcher is gated on `session.state.phase`, not on `workspace === null`,
  which conflates "still starting" with "none open".
- Only the platform's own accelerator counts, and a focused `<textarea>` keeps
  its keystrokes — one rule covering Monaco and every live shell.
- No ADR: two additive commands are ordinary ADR-009 work. Nothing about the
  envelopes, the operation model, path representation or document identity
  moves, so the Phase 0 freeze is untouched.

**Known limitations.**

- **Barely exercised by hand.** The native picker, clicking a recents row,
  Forget and the ⌘O/⌘⇧O chords have never been performed — this session has no
  screen. The browser pass covers rendering and the Tauri run covers the core
  round trip; neither covers a pointer.
- **The one exception, performed on 2026-09-04:** dropping a folder onto the
  launcher, twice, on a logged build — once on a first run with no history, and
  once after closing a workspace, which is the path the launcher is normally
  reached by. Both opened the dropped folder as the workspace (`workspace.open`
  in 251 µs and 399 µs). This is also the first time T045's drop gesture has
  been performed by anyone; the rest of its cases (a file inside the workspace,
  a file outside it, several folders at once, and whether the tab strip's own
  drag still works) remain unperformed, so T045 stays `in-progress`. Whether
  the dashed drop target is actually *visible* is still unconfirmed — only that
  `DropHoverChanged` is emitted for enter and leave.
- Reopening a recent workspace does not bring its tabs back (non-goal above),
  which is the current behaviour of `workspace.open`, not a regression.
- Under React StrictMode in development the history is read twice. Idempotent
  and dev-only, but visible in the log.
- The session writer is debounced, so a workspace opened and closed within a
  second or so may not be in the history when the launcher mounts.
- The history is capped at 20 entries; "View all (n)" reveals what came back,
  not everything ever opened.
- No component test guards the launcher against a visual regression.
- 3a's terminal-dock-free layout is not reproduced: the dock stays because T095
  makes it useful with no workspace and unmounting it would dispose every xterm.

**Follow-up tasks.**

- Perform the manual pass above once there is a human at the window; fold it
  into `docs/testing/phase-0-manual-pass.md`.
- The git branch badge on a recents row, with T100.
- Reopening a recent workspace could restore its persisted tabs — the rows are
  still in `workspace_tab` under that root.
- Snapshots left under a root the user forgot are still never offered and never
  collected (the standing T060/ADR-014 follow-up, now reachable from the UI).
