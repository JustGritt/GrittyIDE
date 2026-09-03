# Task T020 — Workspace open/list + fixtures

- **Phase:** 0 / 1A
- **Status:** review
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

### Summary

Workspace open/close/current/list is implemented end to end. A new
`gritty-workspace` crate owns the root model, the `WorkspacePath` wire format,
path validation and one-level directory listing. `gritty-core` owns the
workspace lifecycle (one workspace at a time), the `WorkspaceOpened` /
`WorkspaceClosed` events and the 100 ms budget hand-over to a cancellable
`workspace.list` operation. The desktop adapters expose four commands and run
the native directory picker on the Rust side. The React explorer renders a lazy
tree (one level per click, hidden toggle, cursor continuation, cancel and retry,
close-reason banner). `fixtures/small-repo` is committed and
`fixtures/large-repo` (10,501 files, 103 directories, a 24-level chain, one
5,200-entry directory, ignored directories) is generated by a Rust example.
Decisions are recorded in ADR-010 and in the security and overview documents.

All verification happened on Windows 11: the development machine moved from
macOS during this task (see "Deviations" and "Remaining risks").

### Files changed

- Root: `Cargo.toml` (workspace member, `tauri-plugin-dialog` pin), `Cargo.lock`,
  `.gitignore` (`fixtures/large-repo/`), new `.gitattributes` (LF policy so
  rustfmt and Biome behave on Windows checkouts).
- New `crates/workspace/` (`gritty-workspace`):
  `src/{lib,error,path,root,listing,fixtures}.rs`,
  `examples/generate_large_repo.rs`, `tests/listing.rs`.
- `crates/gritty-core`: new `src/workspace/mod.rs` (wire types,
  `WorkspaceManager`, budget/operation hand-over, error mapping),
  `src/event/mod.rs` (two event variants), `src/lib.rs`,
  `src/protocol/export.rs`, `Cargo.toml`.
- `apps/desktop/src-tauri`: `src/{commands,core,lib}.rs` (adapters, picker,
  plugin wiring, log filter), `build.rs` plus new `windows-test-manifest.xml`
  (Common Controls v6 manifest for test binaries only), `tests/commands.rs`
  (four workspace scenarios; the harness now uses the webview's own origin),
  `Cargo.toml`.
- `packages/protocol`: `src/{client,commands,types}.ts`, new
  `src/workspace-client.test.ts`, 13 new files under `src/generated/` plus
  `CoreEvent.ts` (written by `cargo test -p gritty-core`).
- `apps/desktop/src`: new
  `features/explorer/{explorer-reducer.ts,explorer-reducer.test.ts,use-explorer.ts,explorer-panel.tsx}`,
  `App.tsx`, `app.css`, `components/error-envelope.tsx`.
- Fixtures: new `fixtures/README.md`, `fixtures/small-repo/**` (12 root
  entries including a hidden directory, a file with spaces and the symlink
  index entry `link-to-src`).
- Docs: new `docs/decisions/ADR-010-workspace-paths.md`,
  `docs/decisions/README.md`, `docs/architecture/{overview,security}.md`,
  `docs/testing/platform-matrix.md`, `docs/tasks/README.md`, this file; T000 and
  T010 flipped to done.

### Tests executed (Windows 11, Rust 1.98.0, pnpm 11)

- `pnpm check` (run with Git Bash as the pnpm script shell): Biome (54 files),
  tsc for the three projects, vitest, `cargo fmt --check`,
  `cargo clippy --workspace --all-targets -D warnings` and
  `cargo test --workspace` all passed. The final `check:generated` step
  requires `packages/protocol/src/generated` to be committed; it failed on the
  uncommitted tree and passed once the branch was committed (re-run on its own).
- `cargo test --workspace`: gritty-core 53 passed (9 workspace tests:
  open/close/current lifecycle, REPLACED and ROOT_MISSING events, budget
  hand-over to an operation, operation cancellation, error mapping; plus the
  generated-type export), gritty-desktop 5 unit + 10 mock-runtime adapter tests
  passed (open/list/close through the ACL, envelope on failures, traversal and
  bad page arguments rejected during parsing, root removed while open),
  gritty-workspace 15 unit tests (path matrix incl. Unicode, spaces, NUL,
  backslashes, drive and UNC forms; cursor round trips) and 11 integration
  tests passed (small-repo sorting and one-level listing, ignored hints,
  symlinks inside/outside/broken, path policy rejections, large-repo root
  listed within the budget with exactly one `read_dir`, 5,200-entry truncation
  and cursor continuation, cancellation discards partial results, 24-level deep
  tree, root removed while open).
- vitest: 46 tests in 8 files passed (12 explorer reducer tests, 3 workspace
  client tests, existing protocol and app tests).
- `cargo run -p gritty-workspace --example generate_large_repo`: 10,501 files
  in 103 directories (max depth 25) in about 2.6 s.
- `pnpm dev` on Windows 11: the app compiled with the dialog plugin, started
  (`starting Gritty … protocol_version="1.0.0"`) and answered the health probe
  from the WebView (WebView2 152). The explorer and picker were not driven by
  hand in this session (no interactive WebView access from the agent) and the
  macOS picker check could not run at all. Both remain pending in
  `docs/testing/platform-matrix.md`.

### Architectural decisions

- ADR-010: one workspace at a time; `WorkspacePath` is a relative,
  `/`-separated wire type validated on deserialize; the canonical root has the
  Windows verbatim prefix stripped; escapes are detected by canonicalizing the
  joined path and checking containment; symlinks are reported with
  `resolvesInsideRoot` and never followed outside the root.
- Listing returns `{status: "ready", page}` when it finishes inside the 100 ms
  budget and `{status: "pending", operation}` otherwise; the page then arrives
  in `OperationCompleted.result`, so the UI has one code path for both.
- Keyset cursors (`d<name>` / `f<name>`) instead of offsets, so continuation
  stays stable when the directory changes between pages.
- The native picker runs in the Rust adapter (`workspace.open` without a
  path). No `dialog:*` capability is granted to the WebView, which is stricter
  than the scope line of this task allowed.
- `WorkspaceClosedReason` (USER, REPLACED, ROOT_MISSING) is emitted by the
  core; ROOT_MISSING is detected lazily on the next command, not by a watcher
  (T070).

### Deviations from the task text

- No `dialog:allow-open` capability was added; the picker is invoked from Rust.
- Windows-specific code exists despite ADR-002 (Windows deferred): a
  `#[cfg(windows)]` verbatim-prefix strip in `crates/workspace/src/root.rs`,
  one `#[cfg(windows)]` unit test, and `build.rs` embedding a Common Controls
  v6 manifest into test binaries. This is required to develop on the current
  machine and is not a claim of Windows support; revisiting ADR-002 is a
  follow-up.
- `ignore` / `walkdir` were not added: the listing never recurses.
- The macOS manual picker check could not be performed (no macOS machine).

### Dependencies

- `tauri-plugin-dialog` 2.7.3: native directory picker, invoked from the Rust
  side only. Pulls `rfd` 0.16.0 (native dialogs) and `tauri-plugin-fs` 2.5.2
  transitively; no `fs:*` or `dialog:*` permission is granted under
  `capabilities/`, so neither is reachable from the WebView.
- `tokio` in `apps/desktop/src-tauri` (already a workspace dependency): the
  `oneshot` channel that bridges the callback-style picker to the async adapter.
- `serde_json` as a dev-dependency of `gritty-workspace` (already in the
  workspace): serialization assertions for the wire types.

### Known limitations

- Non-UTF-8 file names are listed with lossy names and cannot be addressed by
  a `WorkspacePath`; a dedicated fixture and policy are follow-up work.
- `hidden` means a leading dot only; the Windows hidden attribute and the
  macOS `UF_HIDDEN` flag are not consulted.
- Sorting is by Unicode code point, not locale-aware or natural order.
- `fixtures/small-repo/link-to-src` is a real symlink only on checkouts with
  `core.symlinks=true`; on Windows checkouts it is a regular file containing
  `src` (the tests accept both shapes).
- Every page of a directory with more than 5,000 entries re-reads and re-sorts
  the directory; fine for the fixture, unmeasured for much larger directories.
- `total` counts all entries including hidden ones; hidden filtering happens
  in the UI, so the hidden-count badge only reflects loaded pages.

### Follow-up tasks

- Fixture `fixtures/weird-files` (non-UTF-8, very long and Windows-reserved
  names) plus an explicit non-UTF-8 policy for `WorkspacePath`.
- Manual explorer and picker checks on macOS and Linux; fill the matrix rows.
- Revisit ADR-002 now that day-to-day development happens on Windows; audit
  the `#[cfg(windows)]` code introduced here.
- Remember the last workspace and reopen it on start (T060 persistence).
- Watcher-driven invalidation and ROOT_MISSING detection (T070).
- Explorer keyboard navigation and selection, wired to file opening in T030.

### Remaining risks

- All verification happened on Windows 11; macOS and Linux behavior of
  canonicalization, symlink reporting and the picker is inferred from the
  APIs, not executed.
- `tauri-plugin-dialog` links `rfd` and `tauri-plugin-fs` into the binary; the
  capability files deny them to the WebView, and future capability edits must
  keep it that way.
- The CRLF-to-LF working-tree conversion done for `.gitattributes` changed no
  committed content (the index already held LF), but Windows contributors with
  `core.autocrlf=true` should re-checkout after pulling.
