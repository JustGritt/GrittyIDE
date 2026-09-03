# Task T050 — Markdown

- **Phase:** 0 / 1B
- **Status:** in-progress
- **Depends on:** T030, T040

## Objective

Make Markdown a first-class operational surface: source, preview and split
views; sanitized GitHub-flavored rendering with fenced-code highlighting;
clickable repository references that open files in the editor.

## Dependencies

T030 (documents), T040 (source editing), T010 (protocol).

## Required reading

- Plan §8.2 (Markdown as an operational surface), §12 (no raw HTML), §21 (stack decision)
- ADR-006; `docs/architecture/security.md`
- CSP in `tauri.conf.json` (`img-src data: blob:` allowed; remote images are not)

## Allowed scope

- new `packages/markdown/**` (parser/renderer, sanitizer, link resolution)
- `apps/desktop/src/features/markdown/**` (view modes, scroll sync, link handling)
- `packages/protocol` (`shell.openExternal(url)` command with http/https allowlist, if needed)
- `apps/desktop/src-tauri` (adapter for opening external URLs through the core, not a WebView plugin)
- dependency additions with justification (one stack: e.g. `unified`/`remark-gfm`/`rehype-sanitize`
  + a highlighter, or `markdown-it` + DOMPurify; prefer the one with the simplest sanitization story)
- new `fixtures/markdown/**` (GFM features, XSS attempts, local links, long document)

## Explicit non-goals

- Mermaid, math, WYSIWYG editing, remote image loading, promoting checklist items to
  workstreams (later), vendor-specific handling of instruction files.

## Required behavior

1. View modes per Markdown document: source (Monaco), preview, split; mode remembered
   per tab (persisted by T060 later).
2. GFM: headings, emphasis, lists, task lists (read-only checkboxes), tables, fenced
   code with syntax highlighting, strikethrough, autolinks, footnotes optional.
3. Sanitization: raw HTML stripped or escaped; `javascript:`, `data:` (except images
   if allowed) and other unsafe URL schemes removed; no inline event handlers; tested
   with an XSS fixture.
4. Links: relative repository paths open the target document; `path:line[:col]`
   suffixes position the cursor; anchors scroll within the preview; external http(s)
   links open in the system browser only through the core command (which validates the scheme).
5. Preview updates within ~100 ms after an edit for typical documents; long documents
   render without freezing (debounce or incremental rendering; document the approach).
6. Basic scroll sync source → preview.
7. README.md and AGENTS.md are recognized as "instruction files" only for a badge/
   quick-open, with no vendor-specific semantics.

## Failure behavior

- Malformed Markdown never throws; the preview shows what it can.
- Extremely large documents (> threshold) switch to manual refresh with a notice.
- Broken local links render as such and show a tooltip, no navigation error dialog.

## Acceptance criteria

- `fixtures/markdown` renders correctly and the XSS fixture yields no script execution
  or unsafe attribute (automated test on rendered output).
- Clicking a local reference opens the file at the right line; external links leave
  the app only through the core.

## Tests required

- Vitest: rendering snapshots for GFM features, sanitizer tests, link resolution unit tests.
- Manual: split view, scroll sync, navigation; record in the platform matrix.

## Commands to run

`pnpm check`, `pnpm dev`.

## Completion report

(fill in)
