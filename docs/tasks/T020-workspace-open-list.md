# Task T020 — Workspace open/list + fixtures

- **Phase:** 0 / 1A
- **Status:** todo
- **Depends on:** T010

## Objective

Open a workspace from a native directory picker, validate and canonicalize its
root, and list directories lazily so a 10k+ file repository opens without an
eager recursive scan. Create the deterministic fixture repositories the rest of
Phase 0 relies on.

## Dependencies

T010 (commands, error envelope, operation model, ids, events).

## Required reading

- Plan §6.1, §6.2, §12 (path validation), §13 (budgets), §16 (10k files item)
- Addendum §10 (fixtures), §14 filesystem questions (symlinks, case sensitivity,
  normalization, files outside root), §3.3 backpressure
- ADR-003; `docs/architecture/security.md`

## Allowed scope

- new `crates/workspace/**` (root model, path validation, directory listing)
- `crates/gritty-core` (workspace lifecycle, events `WorkspaceOpened/Closed`)
- `apps/desktop/src-tauri` (dialog plugin wiring, command adapters)
- `packages/protocol` (commands `workspace.open`, `workspace.close`, `workspace.list`, `workspace.current`)
- `apps/desktop/src/features/explorer/**` (lazy tree UI)
- new `fixtures/small-repo/**`, `fixtures/large-repo` generator (script or Rust example), `fixtures/README.md`
- dependency additions: `tauri-plugin-dialog` (capability `dialog:allow-open` only), `ignore` or `walkdir` if justified

## Explicit non-goals

- Reading/writing file contents (T030), watching (T070), search (T080), Git (T100).
- Multi-root workspaces, workspace settings, `.gritty/` directory (undecided).
- Recursive eager tree loading or a full-repo index.

## Required behavior

1. `workspace.open(path?)`: without a path, show the native directory picker; the
   chosen root is canonicalized (symlinks resolved), must exist and be a directory,
   and receives a `WorkspaceId`; `WorkspaceOpened` is emitted. One open workspace at a time for now.
2. Path policy (documented in `docs/architecture/security.md`): every path
   argument is joined to the root, normalized, and rejected if it escapes the root
   (`..`, absolute paths outside root, symlinks resolving outside root). Symlinks
   inside the root are listed with `kind: symlink` and a resolved-inside/outside flag;
   they are not followed outside. Case sensitivity follows the filesystem; document
   macOS default (case-insensitive) behavior.
3. `workspace.list(dirPath)`: one level only; entries `{ name, kind: file|dir|symlink|other, size?, modifiedAt?, hidden, ignoredHint }`
   sorted directories first then by name; `ignoredHint` marks well-known heavy
   directories (`.git`, `node_modules`, `target`, `dist`) but nothing is filtered;
   directories with more than N entries (N = 5000) return a truncated page plus a
   `truncated: true` flag and a cursor for continuation.
4. Listing runs off the command thread; it is an operation when it may exceed
   ~100 ms (large directories) so it can be cancelled.
5. Explorer UI: lazy tree, expands one level per click, shows hidden files toggle,
   never requests more than the visible subtree.
6. Fixtures: `fixtures/small-repo` (TypeScript, Rust, JSON, Markdown, nested dirs,
   a hidden file, a symlink), `fixtures/large-repo` generated deterministically
   (10k+ files, deep trees, one 5k-entry directory, ignored dirs) by a script that
   is committed while its output is git-ignored.

## Failure behavior

- Non-existent or unreadable root → `NotFound` / `PermissionDenied`, no workspace state change.
- Root removed while open → next command fails with `NotFound`; `WorkspaceClosed`
  emitted with a reason; UI shows it, no crash.
- Broken symlink in a listing → entry reported with `kind: symlink`, `broken: true`.
- Cancelling a listing operation → `CANCELLED`, partial results discarded.

## Acceptance criteria

- `fixtures/large-repo` opens; root listing appears within 100 ms; no recursive scan
  (verified by instrumentation/log counts).
- Traversal attempts (`../`, absolute outside root, symlink escape) are rejected by tests.
- Explorer navigates `small-repo` including hidden and symlinked entries.

## Tests required

- Rust unit tests: path normalization/validation matrix (incl. Unicode, spaces, symlinks).
- Rust integration tests: listing `small-repo` and generated `large-repo` (sorting, truncation, cancellation).
- Vitest: explorer tree state reducer.
- Manual: picker flow on macOS; record in `docs/testing/platform-matrix.md`.

## Commands to run

`pnpm check`, `pnpm dev`, fixture generator command (document it in `fixtures/README.md`).

## Completion report

(fill in)
