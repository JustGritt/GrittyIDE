# Security model

Status: baseline written at T000; refined by T020 (paths), T030 (file
content and writes), T060 (session database), T090 (processes), T300 (agent
capabilities).

## Principles

- Repository content and agent output are untrusted input.
- The WebView gets a minimal command surface. No arbitrary filesystem, shell,
  process or path APIs are exposed; Tauri capabilities stay at the core
  app/event/window/webview defaults (`apps/desktop/src-tauri/capabilities/default.json`).
  The dialog plugin is registered for the Rust side only: the WebView is
  granted none of its commands, and `workspace.open` shows the picker itself.
- Every path crossing into the core is canonicalized and validated against the
  allowed workspace roots. Traversal (`..`), symlink escapes and paths outside a
  root are rejected unless a capability explicitly allows `workspace.outsideRoot`.
- Command and process execution for agents runs under explicit allowlists or
  capability grants; destructive or out-of-scope actions are shown before
  execution unless a persistent permission was deliberately granted.
- Markdown preview never renders raw HTML from documents or agents; output is
  sanitized against an allowlist and links to `javascript:`/`data:` URLs are
  stripped (T050, ADR-013).
- Secrets stay out of logs, events, SQLite and persisted run output. Provider
  tokens go to OS credential storage when direct API credentials appear.
- Content Security Policy is enabled in both dev and release
  (`app.security.csp` / `devCsp` in `tauri.conf.json`). Widen a directive only
  for a concrete need (Monaco workers, Markdown images) and record why.

## Workspace path policy (T020, ADR-010)

Implemented in `crates/workspace` (`WorkspaceRoot`, `WorkspacePath`) and
exercised by its unit and integration tests.

- **Root.** A workspace has one root: an absolute directory, canonicalized
  (symlinks resolved) when it is opened. It must exist, be a directory and be
  readable; otherwise `workspace.open` fails with `NotFound`,
  `PermissionDenied` or `InvalidInput` and no state changes. On Windows the
  canonical form has the `\\?\` prefix removed so it compares and displays
  like the paths users type.
- **Wire paths.** Every path argument is a `WorkspacePath`: relative to the
  root, `/`-separated, no leading or trailing slash, no `.` or `..` segments,
  no backslashes, no NUL; the empty string is the root. Absolute forms,
  including Windows drive letters (`C:/x`, `C:x`) and UNC prefixes, are
  rejected on every platform. Violations are `InvalidInput`, detected while
  the request is deserialized, before anything touches the disk.
- **Physical check.** The joined path is canonicalized and must still start
  with the root, component-wise (`/ws2` is not inside `/ws`). A symlink that
  resolves outside the root is `PermissionDenied`; a missing path is
  `NotFound`; a file where a directory was expected is `InvalidInput`.
- **Symlinks.** Entries are listed with `kind: "symlink"` and
  `symlink: { broken, resolvesInsideRoot, targetKind }`. A listing follows a
  symlink only when it resolves inside the root; nothing outside the root is
  ever read through a link.
- **Case sensitivity** follows the filesystem. On macOS (APFS is
  case-insensitive by default) and Windows (NTFS), `README.md` and `readme.md`
  address the same file and canonicalization returns the on-disk spelling; on
  Linux they are different files. Gritty never folds case or normalizes
  Unicode: names are passed through as the filesystem reports them, and a
  non-UTF-8 name is decoded lossily (it can be shown but not addressed).
- **Hidden** means the name starts with a dot, on every platform. Windows
  hidden attributes are not consulted (follow-up).
- **Nothing is filtered.** `.git`, `node_modules`, `target`, `dist` and
  similar directories are listed with `ignoredHint: true` so the UI can
  de-emphasize them; the user decides what to open.
- **Listings are bounded.** One directory level per call, at most 5000
  entries per page with a continuation cursor, and a 100 ms budget after which
  the listing continues as a cancellable operation. There is no recursive
  scan and no full-repository index in this task.

## File content and writes (T030, ADR-011)

Implemented in `crates/workspace/src/{detect,file}.rs` and
`gritty_core::document`.

- **Reads are bounded.** A document read refuses files above 64 MiB before
  reading them; the whole file is validated as UTF-8 and scanned for NUL
  bytes, and anything that fails is `binary`: no buffer exists, so a save can
  never rewrite it. Only UTF-8 (with or without BOM) is decoded.
- **Writes only through resolved paths.** A save resolves the document path
  against the root like every other command (`resolve_for_write`): an
  existing target is canonicalized and must stay inside the root, a missing
  target needs a parent that resolves inside the root. Symlinks that escape
  the root are `PermissionDenied` before anything is written.
- **Atomic replacement.** New content goes to `.<name>.gritty-<pid>-<n>.tmp`
  in the destination directory (created exclusively), is `fsync`ed, gets the
  destination's permission bits and is renamed over the destination. The
  destination is never partially written; a failure removes the temporary
  file; a crash leaves it behind, recognizable by name.
- **No silent overwrite.** A save compares the disk hash with the hash the
  buffer was synchronized with and refuses (`Conflict`) when they differ or
  the file is gone; only an explicit `force` from the UI overwrites or
  recreates. The same rule guards `documents.close`, `workspace.close` and a
  replacing `workspace.open` while documents are dirty.
- **Content never reaches logs or events.** Events carry document metadata
  (path, hashes, states); text travels only in command responses. Logs
  record ids, paths, sizes and states.
- **Temporary files are hidden entries** (leading dot) and appear in listings
  only when hidden files are shown.

## Markdown preview and external URLs (T050, ADR-013)

- The preview renders repository Markdown through two layers: markdown-it
  with raw HTML disabled, then a DOMPurify allowlist (`docs/architecture/markdown.md`).
  Only inline `data:image/*` images render; nothing is fetched. Ids are
  prefixed so a document cannot clobber the application's DOM.
- Links never navigate the WebView: clicks, middle clicks and context menus on
  preview links are intercepted, and a Tauri `on_navigation` hook refuses any
  navigation away from the application origin.
- External `http(s)` URLs leave the app only through `shell.openExternal`,
  validated by the core (scheme, length, characters, host, no credentials) and
  opened by the system handler from the Rust side; the WebView has no opener
  or shell capability. Only scheme and host are logged.

## Session database (T060, ADR-014)

- **What is stored.** `<app data dir>/gritty.sqlite` holds the last workspace
  root, per-root tab paths with view state, editor preferences and layout,
  window geometry, durable operation records, and the **full text of every
  dirty buffer**: repository content the user edited, `.env` files included,
  in plaintext, protected only by the user account's permissions on the data
  directory. A snapshot is removed when the document is saved, reloaded or
  explicitly discarded; a moved-aside copy (`gritty.sqlite.corrupt-<ms>`) may
  keep older snapshots until the user deletes it.
- **Never onto the disk.** Recovery only ever replaces a buffer; the
  repository file changes through `documents.save` like any other edit, with
  the same conflict rules.
- **Paths.** The data directory is resolved on the Rust side (`tauri::path`
  or `GRITTY_DATA_DIR`); the WebView receives the database path for display
  only and holds no filesystem capability. Every persisted `path` is a
  `WorkspacePath` and is re-validated when read back; a row that fails
  validation is ignored with a warning.
- **Logs and events** carry paths, sizes, versions, hashes and states, never
  snapshot text; `session.update` blobs are bounded (64 KiB, 500 tabs).
- **Corruption** is contained: a file that is not a database, fails its
  integrity check or was written by a newer version never crashes or loops
  the application; it is moved aside or the layer turns itself off and says so.

## Process boundaries

Agents, language servers, terminals, indexers and external tools run outside the
renderer as supervised processes. Their failure must not freeze or crash the UI.
Process trees are terminated predictably on cancel and on application exit.

## Actor attribution

All mutations carry an actor (`USER`, `SYSTEM`, `AGENT(runId)`, `EXTERNAL`) so
the user can always see who changed what. See `overview.md`.
