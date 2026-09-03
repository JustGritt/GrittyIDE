# Task T050 — Markdown

- **Phase:** 0 / 1B
- **Status:** review
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

### Summary

Markdown is an operational surface: source, split and preview views per tab
(Markdown defaults to split), GitHub-flavored rendering through markdown-it
with Gritty's own plugins, a DOMPurify allowlist as the second sanitization
layer, fenced code colorized by Monaco's tokenizers, and repository links that
open documents at `line:col`, scroll to anchors, or leave the app only through
the new `shell.openExternal` command. Links are classified before sanitization
into `data-md-*` attributes (a `path:12:3` suffix would otherwise be stripped
as a URL scheme); unsupported links (`mailto:`, other schemes, escapes above
the root) render as inert spans with the reason as tooltip; a missing target
marks the link and shows an inline notice. The preview reads the live Monaco
model with an adaptive debounce, pauses above 512 KiB or a 200 ms render, and
refuses above 4 MiB. Source → preview scroll sync interpolates over blocks'
source lines. README.md and AGENTS.md carry a badge and nothing else.

`shell.openExternal` is the first `shell.*` command: the core validates
(http/https only, ≤ 2048 bytes, no whitespace or control characters checked
before the WHATWG parser can strip them, a host, no credentials), an
`ExternalOpener` trait is the seam, the desktop implements it with the `open`
crate on the blocking pool, tests inject a recording fake, only scheme and host
are logged. A Tauri `on_navigation` hook keeps the WebView on the app origin as
a backstop. Protocol `1.2.0` (additive), ADR-013 records the decisions.

Verified on macOS 26 arm64: `pnpm check` green, the production build, and
`pnpm dev` reporting protocol 1.2.0 with the health round trip. The agent
cannot see or click the WebView, so rendering, scroll sync, link navigation and
the browser actually opening are recorded as pending the human pass.

### Files changed

- Root: `Cargo.toml` (`url`, `open`), `Cargo.lock`, `pnpm-workspace.yaml` (`markdown-it`,
  `dompurify`), `pnpm-lock.yaml`, `biome.json` (`fixtures/markdown`, `__snapshots__` excluded).
- `crates/gritty-core`: `src/shell.rs` (new: request, validator, `ExternalOpener`, `ShellService`,
  tests), `src/lib.rs`, `src/protocol/mod.rs` (`1.2.0`), `src/protocol/export.rs`, `Cargo.toml`.
- `apps/desktop/src-tauri`: `src/shell.rs` (new: `SystemOpener`), `src/core.rs` (`shell`,
  `with_opener`), `src/lib.rs` (`attach_core_with`, navigation guard + test, handler row),
  `src/commands.rs` (`shell_open_external`), `tests/commands.rs` (recording opener, three shell
  tests), `Cargo.toml`.
- `packages/protocol`: `src/commands.ts`, `client.ts`, `types.ts`, `shell-client.test.ts` (new),
  `documents-client.test.ts` (version), `src/generated/{OpenExternalRequest.ts,version.ts}`.
- `packages/markdown` (new, `@gritty/markdown`): `package.json`, tsconfigs, `vitest.config.ts`
  (jsdom); `src/{sanitize,resolve-link,slug,instruction-files,render,highlight-dom,markdown-it-types,index}.ts`,
  `src/plugins/{source-lines,heading-ids,task-lists,tables,images,links}.ts`, `src/testing/fixtures.ts`,
  tests for each and `src/__snapshots__/gfm.html`.
- `packages/editor`: `src/monaco/editor-host.ts` (`onScrollChange`, `revealPosition`),
  `src/monaco/highlighter.ts` (new), `src/monaco/index.ts`.
- `apps/desktop/src/features/markdown` (new): `view-modes.ts`, `scroll-sync.ts`, `preview-links.ts`,
  `highlighter.ts` (each with tests), `renderer.ts`, `markdown-preview.tsx`, `markdown-toolbar.tsx`.
- `apps/desktop/src/features/editor`: `use-editor-binding.ts` (content subscriptions, scroll,
  pending reveal), `editor-pane.tsx` (toolbar, surfaces row, link handling), `tab-strip.tsx`
  (badge); `features/documents/use-documents.ts` (`openFile` result + `quiet`);
  `features/explorer/explorer-panel.tsx` (badge); `app.css`; `apps/desktop/package.json`.
- Fixtures: `fixtures/markdown/{gfm,xss,links,long,README,AGENTS}.md`, `docs/target.ts`,
  `docs/nested/deep.md`; `fixtures/README.md`.
- Docs: `docs/decisions/ADR-013-markdown-preview.md` (+ README row), `docs/architecture/markdown.md`
  (new), `security.md`, `overview.md` (command list, version, `## Shell`), ADR-009 (version note),
  `docs/testing/platform-matrix.md`, `docs/plans/phase-0-spike.md` (A7), `docs/tasks/README.md`,
  this file.

### Tests executed (macOS 26 arm64, Rust 1.98.0, pnpm 11.25, Node 24)

- `pnpm check`: Biome clean (135 files); `tsc` for the root, `@gritty/protocol`, `@gritty/editor`,
  `@gritty/markdown` and `@gritty/desktop`; Vitest 30 files / 169 tests (markdown 54 under jsdom:
  GFM file snapshot, XSS fixture assertions on the parsed DOM, sanitizer hook table, link resolver
  table incl. wire-grammar and percent-encoded scheme cases, slugs, instruction files, highlight
  pass with abort and stale-render drop; desktop 45 incl. view modes, scroll interpolation, link
  extraction under jsdom, highlight cache; protocol 33 incl. the shell client); `cargo fmt`,
  `clippy -D warnings`, `cargo test --workspace` green (gritty-core 69 incl. 5 shell tests;
  gritty-desktop 6 unit incl. the navigation-guard predicate and 16 integration incl. the three
  `shell_open_external` tests with the recording opener, so nothing opened a browser);
  `check:generated` clean.
- Render timing: `fixtures/markdown/long.md` (134 KB, 400 sections) renders in 366 ms under jsdom
  (markdown-it alone 15–20 ms in node; DOMPurify under jsdom is the remainder); `gfm.md` parses in
  under 10 ms. The in-app numbers on WKWebView are pending the human pass.
- `pnpm --filter @gritty/desktop build` (Vite 8.2.2, `safari17`): main chunk 4.27 MB (1.13 MB
  gzip, +160 KB over T040 for markdown-it and DOMPurify), a 3.6 KB markdown chunk, six worker chunks.
- `GRITTY_LOG=debug pnpm dev` (2026-09-03): `starting Gritty … protocol_version="1.2.0"`, then
  `health requested by webview` 3.9 s later; no "blocked webview navigation" warnings.
- Not executed (human WKWebView pass, rows in the platform matrix): open `fixtures/markdown` as a
  workspace; Source / Split / Preview keep the cursor; typing in `gfm.md` updates the preview
  within ~100 ms (Web Inspector timeline); `long.md` render time in the app and the pause banner +
  Refresh on a multi-megabyte `.md`; scroll sync follows the editor and reaches the bottom;
  `docs/target.ts:3:5` opens at Ln 3 Col 5; `deep.md#section` and in-page anchors scroll;
  `docs/missing.md` → inline notice + marked link; `mailto:` and `../escape.md` inert with
  tooltips; `https://example.com` opens the default browser exactly once and the app stays put;
  middle click and right-click "Open Link" never navigate; the image placeholder and the
  `data:image/png` render; `xss.md` shows escaped text with no alert and no CSP violation in the
  inspector; README/AGENTS badges; the same in `pnpm tauri build --debug --bundles app`.

### Architectural decisions

- ADR-013: markdown-it + DOMPurify allowlist + Monaco `colorize`; links resolved before
  sanitization into data attributes; `mailto:` unsupported (the core allowlist is http/https);
  the preview reads the live model as derived, read-only UI (rule 3 concerns persisted state);
  adaptive debounce and size/time thresholds; `shell.openExternal` with validation in the core
  and the opener injected; the Tauri navigation hook; view modes in memory; instruction files as
  a badge only.
- DOMPurify at 3.4.14 rather than Monaco's 3.4.8: the 3.4.13/3.4.14 notes fix bypasses around
  allow-listed tags and hooks, which this sanitizer relies on; the second copy is accepted.
- `packages/markdown` is pure (jsdom only because DOMPurify needs a DOM); the Monaco-backed
  highlighter lives in `@gritty/editor/monaco`; the desktop feature holds the React island and
  the browser-only singletons.

### Deviations from the task text

- Allowed scope: T050 also changed `packages/editor` (host scroll/reveal, highlighter), the editor
  feature (binding subscriptions, pane layout, tab badge), `use-documents.ts` (`openFile` result),
  the explorer badge, `app.css`, `biome.json` and the catalog. All additive.
- `mailto:` links are unsupported rather than external: the core opens http(s) only.
- Bare `www.example.com` no longer autolinks (fuzzy linkify off so file names stay text).
- Footnotes (optional in the task) are not implemented.
- Broken local links are detected at click time (the WebView cannot stat files); the link is
  marked and remembered per pane afterwards.
- Instruction files get a badge; quick-open is a follow-up.
- Repository images render as placeholders (no file access from the WebView); only inline
  `data:image/*` renders.

### Dependencies

- `markdown-it` 15.0.1 (+ `mdurl`, `argparse`, `entities`, `uc.micro`, `linkify-it`,
  `punycode.js`): the parser with source-line maps and a small plugin surface; VS Code's own
  preview uses it.
- `dompurify` 3.4.14 (direct; 3.4.8 stays as Monaco's transitive copy): the sanitizer;
  security-sensitive, reviewed against its changelog.
- `url` 2.5.8 (Rust, already in the tree via tauri): URL parsing for the allowlist.
- `open` 5.4.3 (Rust, desktop only): system handler for http(s) URLs.
- No new highlighter: Monaco's tokenizers.

### Known limitations

- The preview replaces `innerHTML` on every render, so a text selection in the preview does not
  survive typing in the source.
- Rendering runs on the main thread; the thresholds (512 KiB / 200 ms / 4 MiB) are measured live
  and pause the preview rather than freeze the app, but a multi-megabyte Markdown file has no
  preview.
- Scroll sync is source → preview only and interpolates linearly between blocks.
- Anchors into another document (`deep.md#section`) open the document but do not scroll to the
  anchor yet.
- Local images do not render; remote images never will (CSP, non-goal).
- `long.md` timing is known under jsdom only; WKWebView numbers are owed.

### Follow-up tasks

- Repository images through a core read command (bytes → blob URL) and a recorded CSP note.
- Preview → source scroll sync; DOM diffing so selections survive re-render; a Web Worker for
  markdown-it if WKWebView measurements miss the budget.
- Anchor scrolling after a cross-document reveal; quick-open for instruction files; footnotes.
- Persist view modes (T060). Trusted-workspace confirmation before opening external URLs.
- Human WKWebView pass on macOS (rows above); `origin/t030-documents` and `origin/t040-monaco`
  deletion.

### Remaining risks

- Nothing in the preview was clicked on this machine: the Monaco colorize output inside the
  preview, the navigation guard in WKWebView, the context-menu interception and the browser
  hand-off are asserted by tests and code reading, not observation.
- DOMPurify's behavior is pinned by tests under jsdom, not WebKit; the allowlist is strict enough
  that engine differences should only remove more.
- The `open` crate's macOS path returns after `/usr/bin/open` hands off; a misconfigured default
  browser surfaces as `Io` with the OS message, which is all the UI can show.
