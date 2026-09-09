# Task MONACO-BARREL — packages/editor imports Monaco whole

- **Phase:** maintenance (found by the ADR-032 gate)
- **Status:** doing
- **Depends on:** T040 (the Monaco shell)

> **Respecified after measurement.** The ticket as originally filed rests on two
> premises that were tested and failed; the implementation decision is open and
> is stated under *Recommendation*. Nothing has been implemented.

## Objective

`packages/editor/src/monaco/*.ts` imports `monaco-editor` as a barrel, so the
desktop app's startup chunk is 4.6 MB. Decide, on measured evidence, what to do
about it.

The ticket was filed with a hypothesis — that the barrel drags in "every
language contribution and editor feature", and that narrowing the import is
"the single largest payload lever in the app". **Both halves were measured and
neither survives.** What follows replaces the original note.

## Measurement

Monaco 0.56.0, `pnpm --filter @gritty/desktop build` (rolldown), `apps/desktop/dist`.
Each row is a real build; the numbers are the main chunk.

| Build | Main chunk | gzip |
|---|---|---|
| Baseline (`import * as monaco from 'monaco-editor'`) | 4,795.5 kB | 1,265.5 kB |
| `editor.api` + all 74 contributions | 4,740.9 kB | 1,251.5 kB |
| `editor.api` + 44 of 74 contributions | 4,309.9 kB | 1,137.8 kB |
| `editor.api`, **zero** contributions (floor) | 3,547.4 kB | 948.7 kB |

Which decomposes the chunk as:

- **editor core, 3,547 kB** — irreducible while Monaco is the editor.
- **the 74 editor contributions, 1,194 kB** — the whole opportunity.
- **81 language stubs + 4 language services + the lsp client, 55 kB** — nothing.

### The two premises that failed

1. **The languages are already lazy.** Every
   `esm/vs/languages/definitions/<lang>/register.js` registers metadata with a
   `loader: () => import('./<lang>.js')`. The grammars are already 86 separate
   chunks totalling **443 kB**, fetched only when a file of that language is
   opened. Pruning them cannot move the main chunk — they contribute ~55 kB of
   stubs, and the core only detects 32 language ids
   (`crates/workspace/src/detect.rs`), of which Monaco ships grammars for 24.
2. **The suggested import path does not resolve.** The note proposed
   `monaco-editor/esm/vs/editor/editor.api`. Monaco 0.56's `exports` map is
   `"./*": "./esm/vs/*.js"`, so that specifier resolves to
   `esm/vs/esm/vs/editor/editor.api.js`, which does not exist. The working
   specifier is **`monaco-editor/editor/editor.api`** — already used by
   `model-eol.dom.test.ts:39`.

### What the ticket is actually worth

The ceiling is 1,248 kB (26%), and only with an editor that has no find, no
folding, no context menu, no suggestions and no bracket matching. The 44-contribution
build above — which already gives up rename, go-to-definition, sticky scroll,
inline completions, semantic tokens and the colour picker — saves **486 kB, 10.1%**.
Every kB of it is an IDE feature, and the cost is a hand-maintained contribution
list that must be reconciled against `editor.main.js` at each Monaco upgrade.

### The lever the ticket missed

Monaco is imported **statically** on every path into it (`monaco-setup.ts`,
`use-editor-binding.ts`, `markdown/renderer.ts`), and there is no dynamic
`import()` of it anywhere in `apps/desktop/src`. So all of Monaco is parsed at
startup even though the app boots to `editor__empty` — *"No document open. Click
a file in the explorer."* (`editor-pane.tsx:300`).

Measured with esbuild, Monaco's own closure is **4.2 MB** of the 4.8 MB chunk
(`editor.api` alone is 2.5 MB); the app and React are the remaining ~0.6 MB.

**Deferring Monaco behind a dynamic import removes ~4.2 MB — ~87% — from the
startup chunk and costs no feature at all.** That is an order of magnitude more
than trimming contributions, and it is the claim the original note made for the
wrong change. Filed as MONACO-DEFER (load Monaco when the first editor tab opens).

## Recommendation

Do **not** implement the contribution trim as specified. Either:

- **close this as not-worth-doing** and put the effort into MONACO-DEFER; or
- **keep only the uncontroversial half** — move the four imports to
  `monaco-editor/editor/editor.api` and re-add contributions explicitly — which
  buys ~55 kB now but makes the contribution list a reviewable artefact, so a
  later trim is a one-line diff rather than an investigation. Note this needs
  `monaco.typescript.*` in `monaco-environment.ts:52-53` to import from
  `monaco-editor/languages/features/typescript/register` — `editor.api` does not
  export it.

Either way the decision — *which editor features does Gritty ship* — is durable
and belongs in an ADR. ADR-033 is the next free number.

## Allowed scope

`packages/editor/**`, `apps/desktop/src/features/editor/**`.

## Explicit non-goals

- The 6.6 MB `ts.worker` (and the 1.0 MB `css.worker`). They are separate worker
  chunks, loaded only when a file of that language opens; they do not touch the
  startup chunk. `monaco-environment.ts` sets `noSemanticValidation: true`, so
  the TypeScript worker is carrying a full compiler to provide syntax
  diagnostics and completions only — worth its own ticket, not this one.
- Deferring Monaco. That is MONACO-DEFER; it touches `App.tsx`, which is contended.

## Acceptance criteria

Whatever is implemented: `pnpm check` green, the editor opens a file, markdown
fenced-code highlighting still resolves aliases, and the before/after main-chunk
size is recorded in the completion report.

## Tests required

`pnpm check`, plus a manual pass: open a TS file (worker completions), a
markdown file (fenced highlighting), and confirm find/folding/context menu still
work if contributions were touched.

## Commands to run

```
pnpm --filter @gritty/desktop build   # then read the main-chunk line
pnpm check
```

## Completion report

Not implemented — respecified. The measurement above is the deliverable; the
implementation decision is open. Experiment builds were reverted; the working
tree carries no change from this investigation.
