# Task JSDOM-VITEST — A jsdom vitest project, so a `.tsx` can be tested against a DOM

- **Phase:** 3
- **Status:** done
- **Depends on:** T096, T232

## Objective

Make `apps/desktop` able to test React components against a real DOM, and prove
it by writing the component test the shell has been missing.

Until now the desktop Vitest project was `environment: 'node'` with
`include: ['src/**/*.test.ts']`. No `.tsx` file was collected at all, so no
component could be rendered and asserted on. T232 worked around this by
rendering to a static string (`react-dom/server`), and recorded the gap; PR #33
shipped a launcher-visibility fix that had no test for the same reason.

## Dependencies

- T096 (terminal chrome) and T232 (timeline view) — both already merged.
- T040 is done, which retires the stale "jsdom arrives with the editor work
  (T040)" comment in the desktop Vitest config.

## Required reading

- `AGENTS.md` — hard rules 12 and 14, dependency policy
- `docs/tasks/completed/T232-timeline-view.md` §7 and its completion report — why the
  timeline settled for static markup
- `apps/desktop/src/features/session/restore-flow.ts` — the fix under test
- `apps/desktop/src/features/explorer/use-explorer.ts` — `adoptWorkspace`
- `packages/markdown/vitest.config.ts` and
  `apps/desktop/src/features/markdown/preview-links.test.ts` — the two existing
  jsdom patterns in this repository

## Allowed scope

- `apps/desktop/vitest.config.ts`
- `apps/desktop/package.json`, `pnpm-workspace.yaml`, `pnpm-lock.yaml`
- new test files under `apps/desktop/src/**`
- this task file and `docs/tasks/README.md`

## Explicit non-goals

- Changing how `packages/protocol`, `packages/editor` or `packages/markdown`
  run their tests.
- Any change to `apps/desktop/src/features/**` production code, including
  refactoring a component to make it testable.
- Any change to `packages/protocol/src/**` (leased by another session).
- A shared test-setup file or a component-testing "framework". One consumer
  does not justify one yet.

## Required behavior

1. A `.tsx` file matching `src/**/*.test.tsx` is collected by the `desktop`
   Vitest project.
2. A test file can opt into a DOM per file, leaving every other desktop test on
   `node`.
3. `@testing-library/react` is available to `apps/desktop`, versioned through
   the workspace catalog like every other shared dependency.
4. At least one real component test exists and fails if the behavior it
   describes regresses.

## Failure behavior

- A command the test did not stub rejects with a `CoreError`-shaped value
  rather than resolving empty, so a missing stub cannot make an assertion pass
  vacuously.
- The core seam is replaced with `vi.mock`, so no test reaches a Tauri runtime
  that is not there; nothing depends on `window.__TAURI_INTERNALS__`.
- The startup restore's module-level single-flight promise is reset between
  cases (`resetSessionRestore`), so one case cannot inherit another's restore.

## Acceptance criteria

- `pnpm check:ts` is green.
- The three other Vitest projects run exactly as before (same environment, same
  `include`).
- Removing `deps.adoptWorkspace(response.workspace)` from `restore-flow.ts`
  turns the new test red, and the control case stays green.

## Tests required

- `apps/desktop/src/features/session/shell-restore.dom.test.tsx` — the shell's
  launcher-versus-workspace decision after a startup restore, in both
  directions.
- Mutation check: the fix removed by hand, suite re-run, fix restored.

## Commands to run

```
pnpm install
pnpm check:ts
npx vitest run --project=desktop shell-restore
```

`pnpm check:rust` and `pnpm check:generated` were **not** run — this task makes
no Rust or protocol change, and the machine did not have the disk headroom for
a fresh-worktree cargo build. Left to the integrator.

## Completion report

### Summary

The desktop Vitest project now collects `src/**/*.test.tsx` and keeps
`environment: 'node'` as its default; component tests opt into a DOM per file
with a `/** @vitest-environment jsdom */` docblock — the pattern already used by
`preview-links.test.ts` and `packages/editor`'s `model-eol.dom.test.ts`.

The whole project was deliberately **not** flipped to jsdom. `packages/markdown`
flips because every test there needs a DOM (DOMPurify silently no-ops without
one, so a sanitizer test could pass vacuously). `apps/desktop` is the opposite
case: the great majority of its ~200 tests are pure reducer, model and flow
tests that would only get slower, and a test that never meant to touch the DOM
would silently be handed one. Per-file opt-in is also the smaller diff, and it
does not touch the other three projects at all.

The first consumer is a component test for the bug PR #33 fixed: the shell
showed the startup launcher while a workspace was open, because it waited for a
`WorkspaceOpened` event that the core can emit before the WebView has finished
subscribing. `runRestore` now adopts the workspace from the restore response
directly, and that path finally has a test.

### Files changed

| File | Change |
|---|---|
| `apps/desktop/vitest.config.ts` | `include` extended to `.tsx`; the stale "jsdom arrives with T040" comment replaced with the actual policy and why it differs from `packages/markdown` |
| `pnpm-workspace.yaml` | catalog: `@testing-library/react` 16.3.3, `@testing-library/dom` 10.4.1 |
| `apps/desktop/package.json` | both as devDependencies via `catalog:` |
| `pnpm-lock.yaml` | regenerated by `pnpm install` |
| `apps/desktop/src/features/session/shell-restore.dom.test.tsx` | new — 2 cases |
| `docs/tasks/completed/JSDOM-VITEST-component-tests.md` | new — this file |
| `docs/tasks/README.md` | status column, written by `agent-board.py render --index` |

No production file was modified. `restore-flow.ts` was edited twice during the
mutation check and reverted both times; `git diff` against it is empty.

### Dependencies added

- **`@testing-library/react` 16.3.3** — renders a React 19 component into jsdom
  and queries it the way a user would. Nothing in the workspace overlaps it:
  the only prior art is `react-dom/server` static markup, which cannot assert
  on what is *not* rendered after effects settle, which is exactly this bug.
- **`@testing-library/dom` 10.4.1** — an unbundled peer of the above
  (`^10.0.0`), not a separate choice. Adding it explicitly keeps the version in
  the catalog with everything else instead of leaving it to resolution.

### Tests actually executed

Run in this worktree, on macOS (Darwin 25.6.0), node v24.16.0, vitest 4.1.11.

- `pnpm install` — clean, no peer warnings. `Packages: +125`.
- `npx vitest run --project=desktop shell-restore` — **2 passed**, tests 346ms.
- `pnpm check:ts` — **exit 0**. Biome `Checked 248 files ... No fixes applied`;
  four typecheck projects `Done`; `Test Files 73 passed (73)`,
  `Tests 570 passed (570)` (568 before this task, +2).
- `npx vitest run` (whole suite) — repeated **3 further times**, exit 0 each,
  `73 passed` / `570 passed` every time. Four clean full-suite runs total; the
  new file is not flaky under parallel load.

**Mutation check (the part that makes the test worth having).** With
`deps.adoptWorkspace(response.workspace)` disabled in `restore-flow.ts`:

```
❯ src/features/session/shell-restore.dom.test.tsx (2 tests | 1 failed)
  × shows the workspace, not the launcher, when the restore reopened one
Tests  1 failed | 1 passed (2)
```

The regression case fails and the control case still passes — so the pair
distinguishes the two states rather than asserting something always true. The
fix was then restored and the suite re-verified green.

### Architectural decisions

1. **Per-file jsdom, not a jsdom project.** Reasons above. The config comment
   states the rule and the contrast with `packages/markdown` so the next person
   does not have to rediscover it.
2. **`vi.mock` on `apps/desktop/src/core/client.ts`.** That module is a
   singleton built on the Tauri transport, so it is replaced rather than
   reached around. This is also what the coordinator asked for, since
   `createCoreClient` is gaining an import-time `hub.start()`; the mocked
   client makes the test independent of that.
3. **A local `Transport` stub instead of `createFakeTransport`.**
   `packages/protocol`'s `exports` map publishes only `.`, and the index does
   not re-export `src/testing/fake-transport`, so it is genuinely unreachable
   from `apps/desktop` — verified: `"./testing/fake-transport" is not exported
   under the conditions [...]`. Making it reachable means editing
   `packages/protocol`, which is out of scope and leased. The stub is ~15 lines.
4. **Every stub handler declares its generated return type.** This turns a
   drifted stub into a typecheck failure instead of a render crash. It caught
   two real shape errors while writing the test (`ownership.list` is an
   `OwnershipMap`, not an array — returning `[]` took `AgentIsland` down;
   `AgentNotificationSettings` needs `supported` and `permission`).
5. **`App` imported at module scope, jsdom shims in `vi.hoisted`.** Importing
   `App` inside a test charged Monaco's ~4s module load to that test's 5s
   timeout, which made the file pass alone and fail under a parallel
   `vitest run`. Hoisting the import moved 4.3s out of the test budget: the two
   cases now take 346ms combined. The shims must be hoisted above the import
   because Monaco calls `document.queryCommandSupported` at import time.
6. **Queries scoped to each render's container, not `screen`.** When a case
   fails mid-flight its DOM can outlive the assertion; a `document.body` query
   then finds two shells and the *other* test fails too, which is how the first
   mutation run reported two failures instead of one.

### Known limitations

- **Four jsdom shims live in the test file**: `document.queryCommandSupported`,
  `ResizeObserver`, `matchMedia`, `HTMLCanvasElement.getContext`. All are APIs
  jsdom does not implement, needed by Monaco and xterm. The second component
  test to render `App` will need them too, and that is the point at which they
  should move to a shared setup file — not before.
- **The test renders the real `App`**, so it transitively boots Monaco and
  xterm. That is ~4.3s of module import per test *file* (not per test). It buys
  a genuine assertion about the real component tree; a second such file doubles
  the cost, and at three or four it is worth revisiting.
- **Only the launcher-versus-workspace decision is covered.** The restored
  *tabs* path (`applyTabs`, view modes, cursor restoration) is not: those need
  a document store and Monaco models, and `restore-flow.test.ts` already covers
  the flow with fakes.
- **The test asserts on `#launcher-title` and `.launcher`** (CSS/id selectors)
  alongside roles, because the launcher's heading text is a product string more
  likely to change than its structure. A renamed class would make it pass
  vacuously — the control case is what guards that.
- **macOS only.** Linux CI is dark until 1 Oct (`CI-LINUX-DARK`), so this has
  not run there. Nothing in it is platform-specific.

### Follow-up tasks

1. **Export `createFakeTransport` from `@gritty/protocol`** (or add a
   `./testing` entry to its `exports`) so `apps/desktop` tests stop
   hand-rolling a transport. Belongs to whoever holds `packages/protocol`.
2. **A shared jsdom setup file for `apps/desktop`** once a second component
   test needs the same shims. Config already supports it; adding it now would
   be a file with one consumer.
3. **Consider a typed stub-core helper** (`stubCore` here) promoted next to the
   fake transport, once more than one test file needs a whole healthy core.
4. **`AgentIsland` crashes on a malformed `ownership.list` response**
   (`Cannot read properties of undefined (reading 'filter')` in
   `ownership.ts:73`). The core's contract makes this shape impossible, so this
   is not a bug today and was *not* fixed here — but it is an unguarded read of
   a cross-boundary payload, and worth a look under hard rule 13.

### Remaining risks

- `pnpm check:rust` and `pnpm check:generated` were not run (no disk headroom
  for a cargo build in a fresh worktree). This task changes no Rust and no
  generated type, so the risk is low, but the integrator should confirm.
- The jsdom shim list is empirical: it is what Monaco and xterm needed today. A
  Monaco upgrade could need one more, and the failure would be an import-time
  `TypeError` in this file rather than anything subtle.
