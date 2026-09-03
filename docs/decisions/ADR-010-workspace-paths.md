# ADR-010: Workspace root, path representation and lazy listing

- **Status:** Accepted
- **Date:** 2026-09-03

## Context

T020 opens the first workspace and lists it. The workspace path
representation is one of the shapes frozen after Phase 0 (addendum §17):
every later file command (T030 read/write, T070 watcher, T080 search, T100
Git) addresses files the way this task decides. The options were absolute
native paths on the wire (simple, but every command re-validates and the OS
forms differ) or a workspace identity plus a relative path.

## Decision

- **One open workspace at a time**, identified by a `WorkspaceId` (UUID v7)
  that is not persisted (T060 decides). Opening another root closes the
  current one (`WorkspaceClosed` with reason `REPLACED`); reopening the same
  root returns the same workspace. A root that disappears closes the
  workspace on the next command (`ROOT_MISSING`) and that command fails with
  `NotFound`.
- **The root is canonical and absolute.** `Workspace.root` is display only,
  in the platform's native form (Windows verbatim prefix stripped).
- **Paths on the wire are `WorkspacePath` strings**: relative to the root,
  `/`-separated, validated on deserialization (`docs/architecture/security.md`).
  Names are neither case-folded nor Unicode-normalized.
- **`workspace.list` is one level**, sorted directories first then by name in
  Unicode code point order, at most 5000 entries per page with a keyset
  cursor (`total`, `truncated`, `cursor`). Nothing is filtered: entries carry
  `hidden` and `ignoredHint`; symlinks carry where they resolve.
- **Listings have a budget.** The listing runs on the blocking pool; when it
  finishes within 100 ms the command returns `{ status: "ready", page }`,
  otherwise the same job continues as a `workspace.list` operation and the
  command returns `{ status: "pending", operation }`. The page then arrives in
  `OperationCompleted.result`; cancelling the operation discards partial
  results. Small directories never touch the operation registry.
- **The native directory picker runs on the Rust side** of `workspace.open`
  (`tauri-plugin-dialog`); the WebView is granted no dialog capability.
- **Crate layout.** `crates/workspace` (`gritty-workspace`) owns the root,
  the path policy, the listing and the fixture generator, and does not depend
  on `gritty-core`; `gritty-core::workspace` owns identity, events, the
  budget/operation hand-over and the mapping of `WorkspaceError` onto
  `CoreError`.

## Consequences

- Later commands take `workspaceId` plus a `WorkspacePath`. A second root
  (multi-root workspaces) means adding a root identity next to the path, not
  changing the string form.
- A non-UTF-8 file name is listed lossily and cannot be addressed back (to be
  covered by the `weird-files` fixture).
- Sorting is deterministic but not locale-aware; the UI may re-sort within a
  page if that ever matters.
- Symlink-heavy repositories pay one `canonicalize` per symlink entry per
  listing.

## Revisit when

Multi-root workspaces are needed, a command must address a file outside the
root (then through an explicit `workspace.outsideRoot` capability), or the
100 ms budget proves wrong on real repositories.
