# Task MONACO-DEFER — load Monaco when the first editor tab opens

- **Phase:** maintenance
- **Status:** todo
- **Depends on:** T040 (the Monaco shell). Queues behind T310-U / T101-D / T120-M for `App.tsx`.

## Objective

Monaco is ~4.2 MB of the desktop app's 4.8 MB startup chunk and is parsed on
every launch, even though the app boots with no editor —
`editor-pane.tsx:300` renders *"No document open. Click a file in the
explorer."* Load it on first use instead.

Measured in MONACO-BARREL (see `MONACO-BARREL-startup-chunk.md`): Monaco's own
closure is 4.2 MB (esbuild, minified); the app and React are the remaining
~0.6 MB. Deferring it is worth **~87% of the startup chunk and costs no
feature**, against ~10% for the contribution trim MONACO-BARREL proposed.

## Dependencies

Nothing in the core. This is entirely a renderer-side import-graph change.

## Required reading

- `docs/tasks/MONACO-BARREL-startup-chunk.md` (the measurement)
- `apps/desktop/src/features/editor/monaco-setup.ts` — the static `?worker` imports
- `apps/desktop/src/features/editor/use-editor-binding.ts`
- `apps/desktop/src/features/markdown/renderer.ts` — `createMonacoHighlighter`,
  the second static path into Monaco; markdown preview must not drag it back in
- `apps/desktop/src/features/editor/editor-pane.tsx`

## Allowed scope

`apps/desktop/src/features/editor/**`, `apps/desktop/src/features/markdown/**`,
`packages/editor/**`, and the single mount point in `App.tsx` if one is needed.

## Explicit non-goals

- Trimming Monaco contributions or languages. That is MONACO-BARREL, and it is
  worth ~10% of what this is worth.
- Touching the language workers.

## Required behavior

- The startup chunk does not contain Monaco. Opening the app with no document
  open must not fetch or parse it.
- Opening the first file loads it, and the editor appears without a visible
  regression in behavior. A loading state is acceptable; a flash of unstyled or
  jumping layout is not.
- Markdown preview's fenced-code highlighting still works, and still shares the
  one Monaco instance rather than loading a second copy.
- `installMonacoEnvironment` stays idempotent and still runs before the first
  model is created.

## Failure behavior

If the dynamic import rejects (a corrupt bundle, an offline asset), the editor
pane must show a stated error rather than an empty pane, and the rest of the
app — explorer, terminal, git — must keep working. The existing `WorkerStatus`
covers worker failure, not module-load failure; this needs its own path.

## Acceptance criteria

- `pnpm --filter @gritty/desktop build` shows a main chunk without Monaco, and
  the before/after numbers are in the completion report.
- No `import` of `@gritty/editor/monaco` or `monaco-editor` remains on a static
  path reachable from the app entry.
- `pnpm check` green.

## Tests required

`pnpm check`, plus a manual pass: launch, confirm no Monaco chunk is requested
before a file is opened; open a `.ts` file; open a `.md` file and check fenced
code is highlighted; reopen the app with a restored session (T060) that has tabs.

## Commands to run

```
pnpm --filter @gritty/desktop build
pnpm check
```

## Completion report

_Not started._
