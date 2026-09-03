# ADR-012: Monaco editor binding and line-ending policy

- **Status:** Accepted
- **Date:** 2026-09-03

## Context

Plan §8.1 names Monaco as the code editor. ADR-011 fixes the contract it must
consume: the core owns every buffer, the UI sends full-text updates that
quote the version they were derived from, `dirty` is derived by the core, and
line endings are stored and written verbatim. Monaco brings its own model
with its own ideas about line endings, large files and web workers. T040
binds the two; the choices below keep Monaco a rendering/editing component
and never a second source of truth (architecture addendum §4.1, rule 3).

## Decision

- **One editor, one model per document.** A single `IStandaloneCodeEditor`
  (`packages/editor/src/monaco/editor-host.ts`); tabs switch models and keep
  a per-document view state (cursor, selection, scroll, folding). Models are
  keyed by `DocumentId` with the URI `gritty://<documentId>/<path>` (the
  extension drives the TypeScript and JSON workers), created from
  `documents.open().text` by `DocumentModelRegistry`, and disposed with the tab.
- **Text enters a model only from the core.** A tab carries a
  `bufferGeneration` that moves when the core replaced the text (reopen,
  reload, conflict refetch, explicit replacement) and never when an update was
  accepted, because that text is already in the model. The editor applies the
  text as one minimal, undoable edit (`computeMinimalEdit` +
  `pushEditOperations`) that is not echoed back as an edit, so cursor, folding
  and undo survive a reload.
- **Text leaves lazily.** Monaco reports a *touch* per content change; the
  debounced flush reads the model once (`getValue()`), so a 12 MiB document is
  never serialized per keystroke. The store mirrors the confirmed version and
  text in refs that are updated synchronously, so a flush chained right after
  an accepted update quotes the new `baseVersion` rather than the one still on
  screen. Debounce: 300 ms (ADR-011), 1000 ms in large-file mode. `save` and
  `close` *settle* first: wait for the update in flight, flush what was typed
  meanwhile, and only then act; a refused update leaves its error on the tab
  and blocks the save so stale text is never written. `UpdateScheduler`
  serializes updates per document.
- **Dirty is the core's.** The tab indicator is `dirty || draft !== null ||
  pending`: the core's flag plus the edits still on their way. Monaco's undo
  stack never decides.
- **Line endings.** Monaco rewrites every line ending to the dominant one when
  a model is created (piece-tree builder; verified in 0.56.0 and pinned by
  `packages/editor/src/monaco/model-eol.dom.test.ts`). Consistently LF or CRLF
  text round-trips byte-exact and is edited normally. A document with mixed or
  lone-CR endings opens read-only in a normalized view with a notice; the user
  may normalize it explicitly to LF or CRLF (one `documents.update`, visible as
  dirty until saved). A reload that changes the kind switches the model's EOL.
  A file without line endings takes Monaco's default (LF on macOS and Linux,
  CRLF on Windows) for its first line break.
- **Large files.** Monaco's own heuristics (20 MB or 300k lines) do not fire
  for files just above the core's 10 MiB threshold, so `largeFileMode`
  degrades explicitly: plaintext model (no tokenization); minimap, folding,
  word wrap, bracket colorization and matching, occurrence and selection
  highlight, links, sticky scroll and suggestions off; `maxTokenizationLineLength`
  1000; read-only until `documents.open { largeFileEditing: true }`.
- **Workers.** Vite `?worker` chunks for the editor, JSON, CSS, HTML and
  TypeScript workers are injected into `installMonacoEnvironment` as factories,
  so every construction runs through `WorkerStatus` (construction errors,
  worker `error` events, `securitypolicyviolation` events). The pane shows a
  warning with the details and Monaco degrades (in-process editor worker, no
  language diagnostics). Monaco 0.56 would otherwise construct its language
  workers itself (`new Worker(new URL('x.worker.js', import.meta.url))`);
  `MonacoEnvironment.getWorker` takes precedence. Workers load from `'self'`;
  the CSP is unchanged. `?worker&inline` (a blob URL, already allowed) is the
  fallback should the packaged app's origin refuse them.
- **Languages.** The core's `languageId` (VS Code vocabulary) is used when
  Monaco registers it, `plaintext` otherwise (`toml`, `ignore`, `dotenv`,
  `makefile`, `cmake`, …). TypeScript and JavaScript semantic validation is
  off: the worker sees one file, so every import would be an error; syntax
  diagnostics stay on.
- **Preferences** (font size, tab size, word wrap, minimap) are in-memory
  state of the editor pane until T060 persists them.
- **Package boundary.** `@gritty/editor` is pure and node-tested (line
  endings, minimal edit, language fallback, worker status, registry and
  binding over a structural `TextModelLike`); `@gritty/editor/monaco` is the
  browser-only entry that imports `monaco-editor`. `apps/desktop/src/features/editor`
  holds React, the Vite worker imports and the store glue;
  `apps/desktop/src/features/documents` keeps the store, the scheduler and the hook.

## Consequences

- The document contract did not change: no protocol, Rust or generated-type changes.
- Up to one debounce interval of typing (300 ms, 1 s for large files) lives
  only in Monaco; a renderer crash inside that window loses it, and T060
  snapshots will not cover it either.
- A large document still crosses the IPC boundary as one string per flush;
  deltas (`edits[]`, the upgrade path of ADR-011) are the follow-up.
- Mixed-EOL files cannot be edited in place; normalizing them is a deliberate,
  visible change.
- The full Monaco entry (every language, a 4 MB minified main chunk) ships; a
  slim custom bundle is a follow-up.
- `marked` and `dompurify` arrive as Monaco's transitive dependencies; T050
  chooses its Markdown stack explicitly rather than leaning on them.

## Revisit when

Delta updates land; a second editing surface (Markdown source, diff editor)
needs the registry; Phase 0 startup or memory measurements say the full entry
is too heavy; the mixed-EOL policy proves too strict on real repositories.
