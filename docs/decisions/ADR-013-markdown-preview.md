# ADR-013: Markdown preview: stack, sanitization, links and the shell command

- **Status:** Accepted
- **Date:** 2026-09-03

## Context

ADR-006 makes Markdown an operational surface: source, preview and split
views, GitHub-flavored rendering, clickable repository references, no raw
HTML. Plan §12 and `docs/architecture/security.md` treat repository content
as untrusted input, and the WebView holds no shell, filesystem or navigation
capability. T050 picks the stack (plan §21 left "Markdown stack and safe
custom link/reference syntax" open) and fixes how the preview talks to the
editor and to the outside world.

## Decision

- **Stack.** `markdown-it` 15 with `html: false` (raw HTML is escaped by the
  parser), GFM tables and strikethrough, `linkify` without fuzzy matching
  (`README.md` in prose is not a Moldovan domain; bare `www.` does not
  autolink), and Gritty's own plugins in `packages/markdown`: source lines,
  prefixed heading ids, task lists as their own token, table alignment as an
  attribute, inline-only images, repository links. `DOMPurify` 3.4.14 is the
  second layer, at its current version rather than Monaco's 3.4.8 because the
  3.4.13 and 3.4.14 fixes concern allow-listed tags and hooks, which this
  uses; the second copy is accepted. Fenced code is colorized by Monaco's own
  tokenizers (`editor.colorize`), so the preview has the same grammars and
  theme as the source view and no highlighter dependency. Rejected:
  unified/remark/rehype (many packages, more moving parts in the sanitization
  story), `marked` (no source-line maps), highlight.js or shiki (a second set
  of grammars), `markdown-it-task-lists` (unmaintained; ten lines in-house),
  `tauri-plugin-opener` (a plugin with its own permission schema wrapping the
  `open` crate anyway).
- **Sanitization policy.** An explicit allowlist, not DOMPurify's profiles:
  tags `p h1-h6 a img ul ol li table thead tbody tr th td pre code em strong s
  del blockquote hr br input span div sup sub`; attributes `href src alt title
  class id align start type checked disabled` plus `data-*`; URIs `http(s):`,
  fragments and relative paths (no `mailto:`, no other scheme); `img[src]` only
  `data:image/(png|jpeg|gif|webp);base64,` under 2 MiB (the WebView cannot load
  repository files, remote images are a non-goal); `input` only as a disabled
  checkbox; `id` only with the renderer's `md-` prefix (no DOM clobbering);
  `align` only on cells, `start` only on `ol`; `target` and `style` never.
  `createSanitizer` throws without a DOM: DOMPurify silently returns its input
  where it is unsupported, and that must never pass for sanitized. Everything
  that enters `innerHTML`, highlighter output included, went through the
  sanitizer.
- **Links** are resolved by a core rule *before* sanitization (`path:12:3`
  looks like a URL scheme to every URL parser and would otherwise lose its
  `href`) and recorded in `data-md-*` attributes: `document` (workspace path
  in the ADR-010 wire grammar, line, column, fragment), `anchor`, `external`,
  or `unsupported` rendered as an inert span with the reason as tooltip
  (`mailto:`, `ftp:`, `file:`, drive letters, backslashes, escapes above the
  root, percent-encoded schemes). The preview intercepts click, middle click
  and context menu on every link and acts on the data attributes; local
  targets open through `documents.open` (a `NotFound` marks the link broken
  and shows an inline notice, never a dialog), anchors scroll the preview,
  external URLs go through `shell.openExternal`. As a backstop, a Tauri
  navigation hook keeps every WebView navigation on the application origin.
- **`shell.openExternal { url }`** is the only way content leaves the app. The
  core validates (length ≤ 2048 and no whitespace or control characters
  before the WHATWG parser can strip them, `http`/`https` only, a host, no
  credentials) and hands the normalized URL to an injected `ExternalOpener`;
  the desktop implements it with the `open` crate on the blocking pool; tests
  inject a recording fake. Only scheme and host are logged. Protocol `1.2.0`.
- **The preview reads the live Monaco model**, not the store: the store lags
  by the update debounce and could not meet "updates within ~100 ms". This is
  derived, read-only UI over the same text the core receives; rule 3 (Monaco
  is never the source of truth for persisted state) is untouched. Rendering
  is debounced adaptively (three times the last render, 50 to 500 ms), pauses
  above 512 KiB or a 200 ms render with a Refresh button, and refuses above
  4 MiB or in large-file mode. Highlighting has a synchronous cache so
  re-renders while typing keep colorized blocks; misses are filled by an
  asynchronous pass that is aborted by the next render.
- **View modes** (`source | split | preview`, Markdown defaults to split) are
  per tab and in memory; T060 persists them.
- **Instruction files** (`README.md`, `AGENTS.md`, case-insensitive basename)
  get a badge in the explorer, the tab strip and the toolbar and nothing else.

## Consequences

- Two DOMPurify copies ship (3.4.14 for the preview, 3.4.8 inside Monaco)
  until Monaco moves; `marked` stays Monaco-internal.
- Source → preview scroll sync interpolates between blocks' source lines;
  preview → source sync, footnotes, repository images and anchors into other
  documents after a reveal are follow-ups.
- Replacing `innerHTML` drops a text selection in the preview on every render.
- A Markdown document of several megabytes has no preview; rendering off the
  main thread is the follow-up if the thresholds prove too tight.

## Revisit when

Repository images or remote content become a requirement (a core read
command and a CSP change), a second Markdown consumer (agent plans, T300+)
needs the renderer, or measurements on real documents move the thresholds.
