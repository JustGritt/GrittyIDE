# Task T040 — Monaco shell

- **Phase:** 0 / 1A
- **Status:** review
- **Depends on:** T030

## Objective

Embed Monaco as the code editor in `packages/editor`, bound to the T030
document contract: tabs, models, language detection, basic preferences and
large-file guardrails. Produce the Phase 0 evidence for editor behavior on
WKWebView.

## Dependencies

T030 (documents API and events), T010 (protocol client).

## Required reading

- Plan §8.1 (editor baseline), §16 (editor checklist), §13 (typing responsiveness)
- Addendum §4.1 (Monaco is not the source of truth), §16 rule 3
- `apps/desktop/src-tauri/tauri.conf.json` CSP (`worker-src blob:` is already allowed)
- Monaco ESM + Vite worker setup (`?worker` imports, `MonacoEnvironment.getWorker`)

## Allowed scope

- new `packages/editor/**` (Monaco wrapper, worker setup, model ↔ document binding)
- `apps/desktop/src/features/editor/**` (tabs, editor pane, dirty indicators, save shortcut)
- `apps/desktop/vite.config.ts` (worker config), `tauri.conf.json` CSP only if a
  directive is proven necessary (record why)
- `pnpm-workspace.yaml` catalog: `monaco-editor` (justify), optionally `vitest` jsdom
  environment for component tests
- `docs/testing/platform-matrix.md` (fill editor rows)

## Explicit non-goals

- LSP, diagnostics, completion beyond Monaco defaults; diff editor (T100); command
  palette; themes beyond light/dark; Markdown preview (T050); settings UI.

## Required behavior

1. One Monaco model per open document; model text is initialized from
   `documents.open` and edits are forwarded through `documents.update` (debounced,
   with `baseVersion`); the dirty flag comes from the core, not from Monaco.
2. Tabs: open/close/reorder/activate; dirty indicator; close of a dirty tab asks
   (save / discard / cancel); Cmd+S saves through the protocol.
3. Language detection from the document `languageId`; TypeScript, Rust, JSON,
   Markdown highlighted; workers (editor, json, ts) load under the CSP.
4. Editor features verified: syntax highlighting, line numbers, minimap toggle,
   folding, multi-cursor, find/replace, bracket matching, standard keybindings.
5. Large-file mode: for `largeFileMode` documents disable minimap, folding, word wrap
   computations and tokenization beyond a line-length limit; read-only until enabled.
6. External change: when `DocumentExternalChangeDetected` arrives, show a non-modal
   banner with reload / keep; reload replaces the model text and preserves cursor
   where possible.
7. Preferences (in-memory for now, persisted by T060): font size, tab size, word wrap,
   minimap.

## Failure behavior

- Worker fails to load → editor still opens with a visible warning and degraded
  language features; error logged with the CSP/report details.
- Save failure → tab stays dirty, error shown, model untouched.
- Document closed by the core (workspace closed) → tab closes gracefully.

## Acceptance criteria

- TypeScript, Rust, Markdown and JSON fixtures edit and save correctly.
- Keyboard shortcuts, multi-cursor, selection, find, clipboard, drag/drop and font
  rendering checked on macOS and recorded in the platform matrix.
- Typing in `weird-files` large text shows no perceptible frame drops (note the
  measurement method).
- Monaco never holds state the core does not know about (dirty state test).

## Tests required

- Vitest (jsdom or node): document ↔ model binding logic, tab reducer, debounce/version
  handling.
- Manual checklist from plan §16 with results in `docs/testing/platform-matrix.md`.

## Commands to run

`pnpm check`, `pnpm dev`.

## Completion report

### Summary

Monaco 0.56.0 is the editing surface, bound to the T030 contract exactly as
ADR-011 demands and as ADR-012 now records: one model per open document,
created from `documents.open().text`; text pushed back into a model only when
the tab's `bufferGeneration` moves (reopen, reload, conflict refetch, explicit
replacement) as one minimal undoable edit; edits reported as touches and read
lazily once per debounce (300 ms, 1000 ms in large-file mode) into versioned
`documents.update` calls that a pure scheduler serializes per document;
`dirty` from the core plus the edits still on their way. Tabs open, close
(with a save / discard / cancel dialog when unsaved), reorder by drag and drop
and keep their view state; Cmd+S saves from inside and outside Monaco; the
external-change banner keeps T030's reload / keep / overwrite choice; binary
documents get a placeholder, large ones a degraded read-only editor until
"Enable editing"; workers fail visibly and the editor keeps working; the
status bar shows the contract's facts and the cursor.

Line endings: Monaco rewrites mixed or lone-CR endings when a model is created
(verified in the 0.56 source and pinned by a jsdom test against the real
model), so those documents open read-only in a normalized view with an
explicit "Normalize to LF / CRLF" action; consistent LF or CRLF text
round-trips byte-exact. The new `@gritty/editor` package is node-tested
against a fake model; `@gritty/editor/monaco` is the browser-only entry.

Verified on macOS 26 arm64 in this session: `pnpm check` green, the production
Vite build emits the worker chunks, and `pnpm dev` loads the Monaco bundle in
WKWebView and completes the health round trip. The agent cannot see or click
the WebView, so every interaction check (typing, shortcuts, save to disk,
banners, dialog, workers at runtime, frame drops) is recorded as pending the
human pass in `docs/testing/platform-matrix.md`.

### Files changed

- Root: `pnpm-workspace.yaml` (catalog: `monaco-editor` 0.56.0, `jsdom` 30.0.1), `pnpm-lock.yaml`.
- `packages/editor` (new, `@gritty/editor`): `package.json`, `tsconfig.json`, `tsconfig.node.json`,
  `vitest.config.ts`; `src/eol.ts`, `minimal-edit.ts`, `languages.ts`, `worker-status.ts`,
  `text-model.ts`, `binding.ts`, `document-models.ts`, `editor-options.ts`, `index.ts`;
  `src/testing/fake-model.ts`; `src/monaco/{monaco-environment,model-factory,editor-host,theme,index}.ts`;
  tests `eol`, `minimal-edit`, `languages`, `worker-status`, `binding`, `document-models`,
  `editor-options`, `monaco/model-eol.dom` (jsdom, real Monaco model).
- `apps/desktop/package.json` (`@gritty/editor`, `monaco-editor`).
- `apps/desktop/src/features/documents`: `documents-reducer.ts` (+ tests: `pending`,
  `bufferGeneration`, `touched`, `replaced`, `reorder`, `findTabByPath`), `update-scheduler.ts`
  (new, + tests), `use-documents.ts` (attach/touch, confirmed-version refs, settle before save
  and close, activate instead of reopen); `document-panel.tsx` deleted.
- `apps/desktop/src/features/editor` (new): `monaco-setup.ts` (Vite `?worker` imports),
  `use-editor-binding.ts`, `editor-pane.tsx`, `tab-strip.tsx`, `close-dialog.tsx`,
  `status-bar.tsx`, `orphans-panel.tsx`, `editor-preferences.ts`, `preferences-popover.tsx`,
  `file-name.ts`.
- `apps/desktop/src/App.tsx`, `app.css` (sidebar + editor pane layout, editor styles).
- Docs: `docs/decisions/ADR-012-monaco-editor-binding.md` (+ README row),
  `docs/architecture/documents.md` (editor binding section), `docs/architecture/overview.md`,
  `docs/testing/platform-matrix.md`, `docs/plans/phase-0-spike.md`, `docs/tasks/README.md`,
  this file. Housekeeping: T030 marked done, `d6ea7aa` cherry-picked.

### Tests executed (macOS 26 arm64, Rust 1.98.0, pnpm 11.25, Node 24)

- `pnpm check`: Biome clean (97 files); `tsc` for the root, `@gritty/protocol`, `@gritty/editor`
  and `@gritty/desktop`; Vitest 19 files / 102 tests passed (protocol unchanged, desktop 36 incl.
  the new reducer and scheduler tests, editor 37); `cargo fmt --check`, `clippy -D warnings` and
  `cargo test --workspace` unchanged and green; `check:generated` clean (no Rust changes).
- `pnpm --filter @gritty/editor test`: 8 files / 37 tests, including
  `monaco/model-eol.dom.test.ts` under jsdom with the real `monaco-editor/editor/editor.api`
  (two shims: `matchMedia`, `queryCommandSupported`; `server.deps.inline` for Monaco's CSS
  imports): `crlf.txt` round-trips byte-exact, `mixed-eol.txt` is rewritten at creation, a
  core replacement is applied as an undoable edit without a touch.
- `pnpm --filter @gritty/desktop build` (Vite 8.2.2, target `safari17`): succeeds;
  `dist/assets` holds `editor.worker`, `json.worker`, `css.worker`, `html.worker`, `ts.worker`
  chunks, `codicon-*.ttf` and one CSS bundle; the main chunk is 4.1 MB (1.06 MB gzip).
- `GRITTY_LOG=debug pnpm dev` (2026-09-03): incremental Rust build 3.9 s; `starting Gritty …
  protocol_version="1.1.0"`, then `health requested by webview` 1.9 s later from WKWebView
  21624.4.5.11.5 (Tauri 2.11.5): the bundle with Monaco evaluates and the React tree renders.
  One `tauri_runtime_wry: web content process terminated` DEBUG line preceded the health call.
  No panics or errors on the Rust side; the WebView console is not visible to the agent.
- Linux CI on PR #5 (2026-09-03, run 33791245161): `pnpm check` and `pnpm tauri build --no-bundle`
  passed in 4m32s, so the jsdom Monaco test and the Vite production build hold on Linux as well.
  The macOS job is label-gated (`ci:macos`) and did not run.
- Not executed (human WKWebView pass): opening `small-repo` ts/rs/json/md and editing, folding,
  bracket matching, Cmd+F / Cmd+Option+F, Cmd+D / Option+click, clipboard, undo, Cmd+S with the
  file on disk verified, dirty dot, close dialog, tab reorder, squiggles from a syntax error in
  `.ts` and `.json` (workers) with the inspector free of CSP violations, `crlf.txt` bytes after a
  save, `mixed-eol.txt` normalize flow, `utf8-bom.txt` BOM, binary placeholder,
  `one-line-100k.txt`, `generated/large-text.txt` read-only → enable → 30 s typing with the Web
  Inspector timeline, `sed -i ''` on a dirty file → refocus → CONFLICTED banner → reload / keep /
  overwrite, workspace close refused while dirty → discard → orphans shown, force-quit while
  typing leaves the disk untouched, and the same worker check in `pnpm tauri build --debug
  --bundles app` under the production CSP (`tauri://localhost`).

### Architectural decisions

- ADR-012 records the binding: one editor, one model per document keyed by `DocumentId`
  (`gritty://<id>/<path>` URIs), `bufferGeneration` as the only trigger for pushing core text
  into a model, lazy full-text reads on touch, confirmed versions mirrored in refs so a chained
  flush quotes the right `baseVersion`, settle before save and close, the mixed-EOL policy,
  explicit large-file degradation (Monaco's own heuristics do not fire at 10 MiB), workers
  injected as factories and observed by `WorkerStatus`, TypeScript semantic validation off,
  in-memory preferences.
- Package boundary: `@gritty/editor` (pure, node-tested over a structural `TextModelLike`) and
  `@gritty/editor/monaco` (imports `monaco-editor`); the Vite `?worker` imports stay in
  `apps/desktop` so the package carries no bundler syntax and keeps `types: []`-style isolation.
- The store stays the protocol mirror (`features/documents`); the editor feature only renders
  and forwards. No protocol, Rust or generated-type changes; the CSP is unchanged.

### Deviations from the task text

- Allowed scope: the task predates T030's `features/documents/`. The reducer and hook received
  additive changes (touch/pending, buffer generation, reorder, replaced, scheduler) and the
  textarea panel was deleted, because the handoff names them as the reusable pieces and the
  panel as what Monaco replaces.
- Debounce is 1000 ms in large-file mode (ADR-011 named a flat 300 ms); recorded in ADR-012.
- Required behavior 6 lists reload / keep; the banner keeps the third option T030 introduced
  (`save { force }` as overwrite / recreate), because the contract requires offering all three.
- Mixed or lone-CR line endings open read-only with an explicit normalization; the task text did
  not foresee it, ADR-011's verbatim line endings require it.
- TypeScript/JavaScript semantic validation is turned off (less than Monaco's defaults, not a
  feature beyond them): the single-file worker would flag every import of the fixtures.
- Preferences live in a `<details>` popover in the pane rather than a settings UI (non-goal).

### Dependencies

- `monaco-editor` 0.56.0: the editor component named by plan §8.1 and the Phase 0 checklist; no
  overlap with anything in the tree. It brings `marked` 14.0.0 and `dompurify` 3.4.8 as
  transitive dependencies (Monaco's hover rendering); T050 must pick its Markdown stack
  deliberately. No install scripts (`allowBuilds` unchanged).
- `jsdom` 30.0.1 (dev only): DOM environment for the one test that exercises the real Monaco
  model; every other test stays in node.

### Known limitations

- Up to one debounce interval of typing (300 ms, 1 s for large files) exists only in Monaco;
  a renderer crash inside that window loses it.
- A large document is read (`getValue()`) and sent as one string per flush; typing pauses in the
  12 MiB fixture cost a full-text IPC round trip each. Deltas are the fix.
- Mixed-EOL and lone-CR files cannot be edited in place.
- Files without any line ending take Monaco's default EOL (LF on macOS/Linux, CRLF on Windows)
  for the first line break the user types.
- The full Monaco entry ships every language and the LSP client: 4.1 MB minified main chunk;
  dev start pre-bundles it once.
- Tab reorder relies on HTML5 drag and drop, which Tauri's `dragDropEnabled` window option can
  interfere with; keyboard reorder does not exist yet.
- Preferences are not persisted; theme follows the OS only.
- Two documents on one physical file (symlink and direct path) still get independent models.
- `useEditorBinding` re-applies editor options after every store change (cheap: Monaco diffs
  options) and keeps view states for closed documents out of memory, but does not yet cap the
  number of models.

### Follow-up tasks

- Delta updates (`edits[]` next to `text`) so large-file typing stops shipping the whole buffer.
- Slim Monaco bundle (`monaco-editor/editor` + selected language definitions and features) once
  Phase 0 startup and memory measurements are in.
- Keyboard tab reorder and a tab context menu; Cmd+W stays with the native menu.
- Component tests for the React pane now that jsdom is in the tree.
- If the packaged app refuses `'self'` workers, switch to `?worker&inline` and record it.
- `dragDropEnabled: false` if the human pass shows HTML5 drag and drop blocked in WKWebView.
- T060: persist preferences, tabs, view states and dirty buffers.
- Human WKWebView pass on macOS and the Linux run; fill the matrix rows with observations.

### Remaining risks

- Nothing in the editor was clicked on this machine: Monaco's behavior under WKWebView (input,
  IME, clipboard, drag and drop, worker start under the packaged CSP) is asserted by the
  build, the module evaluation in the WebView and the tests, not by observation.
- The `web content process terminated` debug line from wry appeared once before the health
  call; the WebView then worked. Worth watching in the human pass.
- The `useEditorBinding` sync effect keys model creation on the store; a document opened and
  closed within one render cycle never gets a model, which is intended, but the interaction with
  React StrictMode's double effects was reasoned about (registry reuse via `getModel`) rather than
  observed in dev.
