# Task T040 — Monaco shell

- **Phase:** 0 / 1A
- **Status:** todo
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

(fill in)
