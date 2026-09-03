# Fixtures

Deterministic repositories for tests (architecture addendum §10). Tests must
never depend on a developer's real project.

| Fixture | Purpose | Source |
|---|---|---|
| `small-repo/` | fast integration and explorer tests: TypeScript, Rust, JSON, Markdown, nested directories, a hidden file and directory, a file name with spaces, a symlink | committed |
| `large-repo/` | performance and cancellation tests: 10,500 files, a 24-level deep chain, `bulk/` with 5,200 entries (more than one listing page), ignored directories (`.git`, `node_modules`, `target`, `dist`) | generated, git-ignored |
| `weird-files/` | document edge cases (addendum §10.3): see the inventory below | committed, plus a git-ignored `generated/` |
| `markdown/` | Markdown preview (T050): GFM features, XSS attempts, link resolution cases, a long document, instruction files; see the inventory below | committed |

## Generating `large-repo`

```
cargo run -p gritty-workspace --example generate_large_repo            # writes fixtures/large-repo
cargo run -p gritty-workspace --example generate_large_repo -- <dir>   # elsewhere
```

The generator (`crates/workspace/src/fixtures.rs`) is deterministic: same
layout and content on every run. `cargo test -p gritty-workspace` generates its
own copy under `target/fixtures/large-repo` and reuses it while the layout
version in `.gritty-fixture` matches.

## Symlinks on Windows

`small-repo/link-to-src` is committed as a symlink (mode `120000`). On a
checkout with `core.symlinks=false` (the Git for Windows default) it appears
as a plain file containing `src`; the Rust tests create real symlinks in a
temporary copy so their assertions do not depend on the checkout mode.
Creating symlinks on Windows needs Developer Mode or elevation.

## `weird-files` (T030)

Bytes are the test data: `.gitattributes` marks the directory `-text` so Git
never converts line endings. Do not edit the files; tests compare hashes.

| Path | Case |
|---|---|
| `readme.md`, `empty.txt`, `no-trailing-newline.txt` | plain text, zero bytes, no final newline |
| `.hidden-file`, `.hidden-dir/inside.txt` | hidden entries |
| `unicode/日本語.md`, `unicode/héllo wörld.txt`, `unicode/emoji 🎉.txt` | Unicode names and content |
| `spaces/file with spaces.txt`, `spaces/two  spaces.txt` | spaces (including consecutive) in names |
| `long-lines/one-line-100k.txt`, `long-lines/minified.txt` | a 100,000-character line without newline; a 20,000-character line |
| `binary/pixel.png`, `binary/nul.bin` | binary (NUL bytes) |
| `encoding/invalid-utf8.txt` | Latin-1 bytes: reported as binary |
| `encoding/utf8-bom.txt`, `encoding/crlf.txt`, `encoding/mixed-eol.txt` | BOM preserved on save; CRLF and mixed EOL kept verbatim |
| `encoding/utf16le.txt` | UTF-16 with BOM: reported as binary (unsupported encoding) |
| `deep/a/b/c/d/e/f/g/h/i/j/leaf.txt` | 10-level path |
| `links/to-readme`, `links/to-unicode`, `links/broken` | symlink to a file, to a directory, dangling |
| `generated/large-text.txt` | 12 MiB text, above the large-file threshold (generated) |
| `generated/denied.txt` | mode `000` (generated, Unix only): reads must fail with `PermissionDenied` |

```
cargo run -p gritty-workspace --example generate_weird_files            # writes fixtures/weird-files/generated
cargo test -p gritty-workspace                                          # stages its own copy under target/fixtures/weird-files
```

Tests never write into the committed fixture: `stage_weird_files` copies it
(recreating the symlinks from a manifest, so Windows checkouts without
symlinks behave the same) and adds the generated members.

## `markdown` (T050)

Rendering tests snapshot these files; the XSS file is asserted on, never
"fixed". `long.md` is generated deterministically (400 sections, ~134 KB) and
committed; regenerate it only with the same script in the T050 report.

| Path | Case |
|---|---|
| `gfm.md` | headings, emphasis, lists, task lists, tables with alignment, fenced code (ts, rust, json, none, unknown), indented code, quotes, rules, links of every kind, a relative and a `data:` image, duplicate and punctuated headings |
| `xss.md` | raw `<script>`, `onerror`, `javascript:` / `vbscript:` / `data:text/html` / `file:` links, entity and percent-encoded schemes, SVG data image, `<iframe>` `<object>` `<embed>` `<style>` `<svg>` `<math>` `<form>`, `id="root"` clobbering, `style` overlays, `target`, benign content that must survive |
| `links.md` | relative, `./`, `../` (inside and escaping), root-relative, `path:line`, `path:line:col`, anchors, percent-encoded spaces and Unicode, query strings, directories, external, `mailto:`, `ftp:`, drive letters, backslashes, a missing target, an empty link |
| `long.md` | ~134 KB, 400 sections with lists, tasks, fences and tables: render-time budget |
| `README.md`, `AGENTS.md` | recognized as instruction files (badge only) |
| `docs/target.ts`, `docs/nested/deep.md` | link targets (`target.ts:3:5` is a known position; `deep.md#section` an anchor) |

Planned (addendum §10.4): Git fixtures arrive with T100.
