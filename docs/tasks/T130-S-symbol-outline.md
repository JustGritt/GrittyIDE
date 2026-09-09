# Task T130-S — `@` symbols: a regex outline, honestly named

- **Phase:** 2
- **Status:** doing
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

- `docs/tasks/T130-U-quick-open-bar.md` — the parser's `symbols` arm and the row
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

*(filled on completion)*
