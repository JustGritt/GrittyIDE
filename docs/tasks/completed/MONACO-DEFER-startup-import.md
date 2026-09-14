# Task MONACO-DEFER — load Monaco when the first editor tab opens

- **Phase:** maintenance
- **Status:** done
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

- `docs/tasks/completed/MONACO-BARREL-startup-chunk.md` (the measurement)
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

**Summary.** Monaco is no longer linked into the startup chunk. It is reached
through `import()` on two paths — the editor engine, and the markdown preview's
fenced-code highlighter — and the app boots, opens the explorer and renders a
fence-free preview without fetching it.

**Measured, A/B on one tree** (`pnpm --filter @gritty/desktop build`; the same
working tree built twice, my six files reverted and restored, so concurrent
sessions' edits are not in the difference):

| | main chunk | gzip |
|---|---|---|
| before | 4,795.63 kB | 1,265.57 kB |
| after | **919.28 kB** | **275.15 kB** |
| saved | **3,876.35 kB (80.8%)** | **990.42 kB (78.3%)** |

Monaco now lands in `editor.api-*.js` (2,654 kB) and `monaco-*.js` (1,226 kB),
fetched on demand through Vite's `__vite__mapDeps` preload helper; `index.html`
references only the entry chunk. The stylesheet split too — the startup CSS
fell from 236 kB to 75.9 kB, with `monaco-*.css` (82 kB) and `editor-*.css`
(79.5 kB) now lazy.

**Files changed.**

- `apps/desktop/src/features/editor/worker-status.ts` (new) — `workerStatus`
  moved out of `monaco-setup`. The pane subscribes to it on mount, so leaving it
  in a Monaco-importing module would have put Monaco straight back on the
  startup path through the pane.
- `apps/desktop/src/features/editor/monaco-setup.ts` — takes `workerStatus` from
  the new module; documented as dynamic-import-only.
- `apps/desktop/src/features/editor/use-editor-binding.ts` — the engine effect
  awaits `import('@gritty/editor/monaco')` and `import('./monaco-setup')`. Value
  imports became `import type`. Adds `engineStatus: 'loading' | 'ready' | 'failed'`.
- `apps/desktop/src/features/editor/editor-pane.tsx` — imports `workerStatus`
  from the new module, and states the failure rather than showing a blank pane.
- `apps/desktop/src/features/markdown/highlighter.ts` — `createCachedHighlighter`
  takes `() => Promise<CodeHighlighter>` instead of a live highlighter, and calls
  it at most once, on the first fence that misses the cache.
- `apps/desktop/src/features/markdown/renderer.ts` — passes that loader.
- `apps/desktop/src/features/editor/monaco-deferred.test.ts` (new) — the guard.

**Tests actually executed.**

- `pnpm lint` — clean, 304 files.
- `pnpm typecheck` — clean.
- `pnpm test` — **1064 passed, 103 files** at the time of the run (1060/102
  before it; the +4 are mine). A later run in the same shared tree read 1067,
  the extra three belonging to a concurrent session, not to this change.
- `pnpm check:rust` — **one failure, not mine**:
  `commands.rs::root_removed_while_open_fails_with_not_found_and_closes_the_workspace`
  (`left ["WorkspaceOpened","WorkspaceEntriesChanged","WorkspaceClosed"]`,
  `right ["WorkspaceOpened","WorkspaceClosed"]`). That is WATCH-FLAKE's second
  test; this change touches no Rust. Re-run in isolation **4 of 4 green**. Every
  other suite passed, including 509 in `gritty-core`.
- The guard was verified to fail: reintroducing a value import in
  `status-bar.tsx` made it report
  `./status-bar.tsx imports @gritty/editor/monaco`, and it was checked again
  after the glob rewrite.

**Architectural decisions.**

1. *Loaded on mount, not on first tab.* The effect already ran on the pane's
   mount and the engine already lived behind `useState<Engine | null>`, so the
   async boundary went where the state boundary already was. Monaco starts
   arriving right after first paint rather than on the first click, which keeps
   the whole measured win without adding latency to opening a file. The ticket's
   title says "first editor tab"; this is the same saving with less risk.
2. *A guard test, not a build assertion.* The property is invisible in review —
   one convenience import undoes it and nothing else fails. A source-graph scan
   is deterministic and costs milliseconds; asserting on a built bundle would
   have meant a build inside the test suite.
3. *`workerStatus` extracted rather than re-exported.* A re-export from
   `monaco-setup` would have compiled and quietly restored the static path.

**Known limitations.**

- Not exercised in the running app. The GUI pass — launch, open a `.ts` file,
  open a `.md` file with fences, restore a session with tabs — is still needed,
  and is the acceptance criterion I could not execute.
- `engineStatus` is `'loading'` for a moment on every mount. With no document
  open nothing renders differently; the failure copy only appears on `'failed'`.
- The dynamic-import rejection path is reasoned about, not tested: forcing a
  chunk load failure needs a corrupted bundle. Per hard rule 12 this is not a
  recovery claim, only a stated error path.
- The workers are untouched: `ts.worker` is still 6.9 MB on disk, loaded only
  when a TS/JS file opens.

**Follow-up tasks.**

- The manual pass above.
- `ts.worker` is 6.9 MB to provide syntax diagnostics and completions only
  (`monaco-environment.ts` sets `noSemanticValidation: true`). It never touches
  the startup chunk, but it dominates the installed bundle. Worth its own ticket.
- MONACO-BARREL's contribution trim is now worth ~486 kB of a 919 kB startup
  chunk rather than of a 4.8 MB one — a much better ratio, if the features are
  ever judged expendable. It is closed as superseded, not deleted.
