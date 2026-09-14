# Task T130-S — `@` symbols: a regex outline, honestly named

- **Phase:** 2
- **Status:** done
- **Depends on:** T130-U (the bar, the parser and the row union it plugs into)

## Objective

The `@` mode: a list of the current file's headings and top-level definitions,
extracted by pattern from the open buffer, so `@` jumps within a file the way
`⌘P` jumps between them.

## Motivation

Alexis asked for `@` after being told it was the weak mode, so it ships — but it
must ship under a name that is true. There is no language server: the project
plan defers LSP until "the editing substrate is stable"
(`Gritty IDE Project Plan.md:317`), and Monaco's `quickOutline` reads a
`DocumentSymbolProvider` registry that nothing populates. `@` therefore cannot
mean *symbols* in the sense an IDE user expects.

What it can honestly be is an **outline by pattern** over the buffer already in
memory: Markdown headings, and the definition keywords of the languages Gritty
highlights. That is useful in a Markdown-heavy repository — `T050:54` asked for
exactly this, "quick-open, with no vendor-specific semantics" — and it is
cheap, synchronous and testable. It is isolated in one module precisely so it
can be swapped for real symbols later, **or rejected in review without blocking
the palette**.

## Required reading

- `docs/tasks/completed/T130-U-quick-open-bar.md` — the parser's `symbols` arm and the row
  union this fills in
- `packages/markdown/src/slug.ts` and `packages/markdown/src/plugins/heading-ids.ts`
  — heading extraction and slugging that already exist; do not write a second one
- `crates/workspace/src/detect.rs` — the VS Code language vocabulary
  `document.languageId` carries
- `packages/editor/src/languages.ts` — `resolveLanguageId`, and that an unknown
  language renders as plaintext rather than failing
- `AGENTS.md` hard rules 13, 14

## Allowed scope

- `apps/desktop/src/features/quick-open/outline.ts`
- `apps/desktop/src/features/quick-open/outline.test.ts`
- the `symbols` arm of the existing row model in the same directory

## Explicit non-goals

- **No LSP, no language server, no `DocumentSymbolProvider` registration.** That
  is a separate, larger decision the project plan has already sequenced.
- **No parser and no dependency.** Not tree-sitter, not a TS AST. A regex
  outline that is honest about being one costs a file; a real parser is a
  dependency, an ADR and a maintenance surface.
- **No cross-file symbols.** Current buffer only. Workspace-wide symbols without
  an index or a server would be a walk that opens every file per keystroke.
- **No go-to-definition, no references, no rename.**
- **No new protocol.** The buffer is already in the renderer.

## Required behavior

1. **One pure function.** `outlineOf(languageId, text) -> OutlineEntry[]`, where
   `OutlineEntry { name, kind, line, depth }`. No React, no core, no DOM.
2. **Markdown is the good case, and it reuses what exists.** ATX and setext
   headings, `depth` from the level, ordered by position. Take the heading
   walk and the slug from `packages/markdown` rather than writing a second
   heading parser — two of them drifting is how an anchor stops matching a link.
3. **A small, stated set of languages beyond Markdown**: Rust (`fn`, `struct`,
   `enum`, `trait`, `impl`, `mod`), TypeScript/JavaScript/TSX (`function`,
   `class`, `interface`, `type`, `const`/`let` bound to an arrow function,
   `export default`). Anything else returns an empty outline — which the row
   union must present as *"no outline for this language"*, not as "no matches".
   The difference matters: one is a limit of the tool, the other a fact about
   the file.
4. **Fenced code is not outlined in Markdown**, and a `#` inside a fence is not
   a heading. This is the one case a naive line scan gets wrong and a user
   notices immediately.
5. **The term filters the outline in memory**, reusing the same ranking the
   files mode uses if it is exported, or plain case-insensitive subsequence
   otherwise. No round trip: the buffer is already here.
6. **Enter reveals the line in the active editor** and does not open a tab —
   `@` is navigation within the file that is already open.
7. **Bounded.** Stop after 1,000 entries and say the outline is truncated. A
   generated 200k-line file must not freeze the field.
8. **Never claim more than it is.** The empty-language row, the mode's
   placeholder text and the completion report all say "outline", not "symbols".

## Failure behavior

- No editor open, or a binary/unsupported buffer → the stated empty state.
- A language with no rules → "no outline for this language", distinct from "no
  matches", per §3.
- A pathological regex input (very long single line) → the 1,000-entry bound and
  a linear scan; no backtracking construct that can blow up on adversarial
  content. Repository content is untrusted input (rule 13).
- A heading containing markup → escaped like any other untrusted text, never
  interpolated.

## Acceptance criteria

- `@` in a Markdown file lists its headings in order, nested by level, skipping
  fenced code.
- `@` in a `.rs` file lists its `fn`/`struct`/`impl` items.
- `@` in a language with no rules says so, and does not read as an empty file.
- Enter moves the cursor and opens no tab.
- `pnpm check` green.

## Tests required

- **`outline.test.ts` (node, pure)** — table tests: ATX and setext headings;
  a `#` inside a fenced block ignored; Rust and TS items; an unknown language
  returning empty; the 1,000-entry bound; a heading containing `<script>`
  surviving as text.
- The row-union test in T130-U extended so the `symbols` arm's two empty states
  are both named and distinguishable.

## Commands to run

```
pnpm vitest run --project desktop outline
pnpm check
```

## Completion report

### Summary

`outline.ts`: `outlineOf(languageId, text) -> OutlineResult` and
`hasOutlineRules(languageId)`, plus `OUTLINE_LIMIT = 1000` and a 4 MiB markdown
input cap. Markdown reuses `@gritty/markdown`'s block parser, so there is no
second heading parser; Rust and TS/JS are a bounded, index-based line walk.

### Files changed

| File | Why |
|---|---|
| `apps/desktop/src/features/quick-open/outline.ts` | New. The extractor. |
| `apps/desktop/src/features/quick-open/outline.test.ts` | New. 23 table tests. |
| `apps/desktop/src/features/quick-open/quick-open-types.ts` | Shared seam, written before the parallel work started. |
| `apps/desktop/src/features/quick-open/quick-open-model.ts` | `OutlineState.ready` gained `truncated` — see *the defect* below. |

### Tests actually executed

`pnpm exec vitest run --project desktop apps/desktop/src/features/quick-open` —
**102 passed, 5 files** (23 of them this module's). `biome check` on the
directory clean; `pnpm --filter @gritty/desktop typecheck` clean. The whole JS
gate is green including this work: biome 302 files, **vitest 1060 passed**.

`pnpm check` overall is red at `cargo fmt --all --check` on another lane's
uncommitted `crates/gritty-core/src/settings/**`, which is inside their live
lease and not touched here.

### Architectural decisions

**Markdown's block parser, with the inline chain switched off.** Reuse was the
instruction, and it paid twice: fences, `~~~`, indented code and setext edge
cases are markdown-it's, so the outline and the preview cannot disagree. But the
inline tokenizer is superlinear on adversarial input, which is hard rule 13
territory for repository content. Measured:

| input | full parse | block-only |
|---|---|---|
| 1 MB of `![` | **5,528 ms** | **7 ms** |
| 1 MB of `[` | 2,800 ms | 5 ms |
| 8 MB / 200k-line doc | 3,145 ms | 183 ms |

So blocks give structure, and the reused `inlineText` runs only over each
heading's own source, capped at 512 chars. Two timing guards in the suite fail
if the inline chain is ever switched back on.

**`OutlineResult`, not a bare array.** `{ supported, truncated, entries }` plus
`hasOutlineRules`, because "this language has no rules" and "this file has no
entries" are different facts and the row model must say different things. The
suite asserts `result.supported === hasOutlineRules(languageId)`.

### The defect the split created, and the fix

Each agent was correctly scoped to its own files, and the bug lived exactly
between them: `OutlineState`'s `ready` arm had nowhere to carry `truncated`, and
the symbols branch hardcoded `capped('symbols', rows, false)`. An outline cut
short at `OUTLINE_LIMIT` would have rendered as complete — requirement 4 of
T130-U (the quick-open bar) is not scoped to the files mode. Fixed by adding
`truncated` to the ready arm and passing it through; the type change made the
two stale test fixtures a compile error rather than a silent pass, which is why
it was caught.

### Known limitations

- **Nothing here knows what a name means.** A keyword inside a string literal or
  a block comment is an entry; only a line *starting* `//`, `/*` or `*` is
  skipped.
- **Declarations split across lines are missed** — notably
  `const f = (\n  a,\n) => …`, since the arrow test is same-line.
- Macro-generated Rust items, associated consts/types inside an `impl`, object
  literal methods and class members are not listed. Code entries are flat
  (`depth: 0`); only Markdown nests.
- Markdown beyond 4 MiB is parsed to the last line boundary before the cut;
  later headings are absent and `truncated` says so.
- Two markdown-it instances are memoised at module scope for the process
  lifetime — the preview does the same.

### Follow-up tasks

| Ticket | Why |
|---|---|
| LSP symbols replacing this | The honest ceiling of a regex outline. This module is one file precisely so it can be swapped out. |
| Code-splitting note | Quick open now imports `@gritty/markdown`, hence markdown-it and transitively dompurify. The desktop bundle already carries both for the preview, so nothing is added today — but splitting the palette from the preview would pull that chunk along. |
