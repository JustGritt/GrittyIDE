# Markdown preview

Status: fixed by T050 (ADR-013). `packages/markdown` renders and sanitizes;
`apps/desktop/src/features/markdown` shows the result next to Monaco and
handles links; `gritty_core::shell` opens external URLs.

## Pipeline

```
model text ──(debounce)──▶ markdown-it (html: false, plugins) ──▶ DOMPurify allowlist ──▶ innerHTML
                                   │ fenced code the cache cannot colorize
                                   └──▶ Monaco colorize (async) ──▶ DOMPurify ──▶ <code> in the current root
```

- Plugins (`packages/markdown/src/plugins`): `source-lines` (`data-line`,
  `data-line-end`), `heading-ids` (`md-` slugs, `env.headings`), `task-lists`
  (`task_checkbox` token → disabled checkbox), `tables` (`style` → `align`),
  `images` (inline `data:image/*` only, placeholders otherwise), `links`
  (classification into `data-md-*`, unsupported → inert span).
- `render(text, { path, highlight?, brokenTargets? })` never throws; a parser
  failure yields the escaped source and `failed: true`.

## Sanitizer policy

| Subject | Allowed |
|---|---|
| tags | `p h1-h6 a img ul ol li table thead tbody tr th td pre code em strong s del blockquote hr br input span div sup sub` |
| attributes | `href src alt title class id align start type checked disabled`, `data-*` |
| URIs (`href`) | `http:`, `https:`, `#fragment`, relative paths; nothing else |
| `img[src]` | `data:image/(png\|jpeg\|gif\|webp);base64,…` ≤ 2 MiB |
| `input` | only `type="checkbox"` with `disabled` |
| `id` | only values starting with `md-` |
| `align` / `start` | cells with `left\|center\|right` / `ol` with a number |
| never | `style`, `target`, event handlers, `script iframe object embed style svg math form` |

`createSanitizer(window)` throws when DOMPurify is unsupported. Test files
that sanitize run under jsdom (`packages/markdown/vitest.config.ts`).

## Link resolution (`resolveLink(href, fromPath)`)

| Input | Result |
|---|---|
| `#Some Heading` | `anchor` (`md-some-heading`) |
| `https://…`, `http://…` | `external` |
| `x.md`, `./x.md`, `../x.md`, `sub/x.ts` | `document`, joined to the document's directory, normalized |
| `/docs/x.md` | `document`, root-relative |
| `x.ts:12`, `x.ts:12:3`, `x.md#frag`, `x.md?q=1` | `document` with `line`, `column`, `fragment`; query dropped |
| `file%20name.md`, `h%C3%A9llo.md` | percent-decoded |
| `../../escape.md` from the root, `/../x` | `unsupported` (outside the workspace) |
| `mailto:`, `ftp:`, `file:`, `javascript:`, `C:/x`, `%6Aavascript:` | `unsupported` (scheme) |
| `docs/`, `./`, `a\b`, empty | `unsupported` |

Outputs satisfy the ADR-010 wire grammar, so `documents.open` never answers
`InvalidInput` for a link the resolver accepted. Whether a target exists is
known only at click time: `NotFound` marks the link (`data-md-broken`) and
shows a notice.

## Preview behavior (desktop)

| Concern | Behavior |
|---|---|
| text source | the live Monaco model (`EditorBinding.readText`), read-only |
| debounce | `clamp(3 × lastRenderMs, 50, 500)` ms after the last change |
| pause | text > 512 KiB, last render > 200 ms, or large-file mode: banner + Refresh |
| refuse | text > 4 MiB |
| highlighting | LRU cache (500 entries) in front of `monaco.editor.colorize`; async pass capped at 100 blocks / 200 KB, aborted by the next render |
| scroll sync | source → preview: first visible line → interpolation over `[data-line]` offsets; bottom follows bottom; suspended 1 s after an anchor jump |
| clicks | every link: `preventDefault` on click, middle click and context menu; document → `documents.open` (+ reveal `line:col`), anchor → scroll, external → `shell.openExternal`, unsupported → notice |
| navigation guard | `on_navigation` plugin hook in `apps/desktop/src-tauri/src/lib.rs`: only `tauri://localhost`, `http://tauri.localhost` and the dev server in debug builds |
| view modes | `source \| split \| preview` per tab, Markdown defaults to `split`, in memory |
| instruction files | `README.md`, `AGENTS.md`: badge in explorer, tab and toolbar |

## Shell commands

| Command | Request | Returns | Notes |
|---|---|---|---|
| `shell.openExternal` | `{ url }` | – | `InvalidInput` (`details.field: "url"`) unless `http`/`https`, ≤ 2048 bytes, no whitespace/control characters, a host, no credentials; `Io` when the system handler fails; only scheme and host are logged |

No events. Actor `USER`.
