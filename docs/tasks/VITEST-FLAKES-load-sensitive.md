# Tasks MONACO-JSDOM-FLAKE and MARKDOWN-BUDGET-FLAKE — two load-sensitive tests

- **Phase:** 3
- **Status:** review
- **Depends on:** JSDOM-VITEST (the jsdom project the Monaco test runs in)

Two tickets, one file: they are the same bug in two costumes. Both tests
assert against **wall-clock time on a machine shared with other agents**, so
both measure the scheduler rather than the code under test, and both fail
exactly when the machine is busiest — during a fan-out, when a re-run is most
expensive.

## Objective

Make both tests measure what they claim to measure, so a red result means a
regression and not a busy afternoon.

## Allowed scope

- `packages/editor/src/monaco/model-eol.dom.test.ts`
- `packages/markdown/src/render.test.ts`

## Explicit non-goals

- No change to `renderer.render` or to the Monaco binding. Neither is at fault.
- No new benchmark harness — see the follow-up.
- Not `packages/editor/vitest.config.ts`: a transform-caching change is a
  separate, speculative improvement and is recorded rather than taken.

## Reproduction

Both were reproduced deterministically during this session, on a machine
carrying three other agents (load average 80-124), rather than waited for.

**Monaco.** Three runs out of three failed at load 80:

```
FAIL  |editor| src/monaco/model-eol.dom.test.ts
Tests  3 skipped (3)
Duration  11.14s (transform 8.80s, ... tests 10.01s)
```

`beforeAll` does `await import('monaco-editor/editor/editor.api')`. That module
is inlined (this package's `vitest.config.ts`: it ships ESM without
`"type": "module"` and imports `.css`, so Node cannot load it externalized and
Vite must transform it), and the transform is **8.8-9.0 s of real work on every
run**. Vitest's default hook timeout is 10 s. So the budget had about one second
of headroom and any load at all spent it. `tests 10.01s` is the timeout firing,
and the three assertions never ran.

**Markdown.** `render.test.ts` timed the render with `performance.now()` and
asserted `< 2000 ms`. Measured on this machine (8 cores), same fixture:

| | idle | 16 spinners on 8 cores | variation |
|---|---|---|---|
| wall | 324 ms | 858-1048 ms | **3.2x** |
| CPU | 623 ms | 778-822 ms | **1.3x** |

Wall time includes every millisecond the process spent descheduled waiting for
a core; the renderer's actual work does not change. A later run under the other
session's load measured **2188.7 ms wall against 815 ms CPU** — the old
assertion would have failed that run, doing identical work.

## Required behavior

1. Both tests pass under heavy load, and still fail on a real regression.
2. The performance numbers stay visible.

## Acceptance criteria

Both pass repeatedly at a load average that previously failed them.

## Commands to run

```
pnpm exec vitest run packages/editor/src/monaco/model-eol.dom.test.ts
pnpm exec vitest run packages/markdown/src/render.test.ts --reporter=verbose
pnpm exec vitest run packages/editor packages/markdown
```

## Completion report

### Summary

**Monaco: a realistic budget for the slowest setup in the repository.** The
`beforeAll` gets an explicit `60_000` timeout. Nothing was wrong with the import
— the work genuinely costs ~9 s and the default budget was simply smaller than
the work. The assertions still run under the default `testTimeout`; they are
microseconds each once the model exists.

**Markdown: assert CPU time, not wall-clock.** `process.cpuUsage()` counts only
time the process was actually on a core, so it is 2.5x more stable under
contention (1.3x vs 3.2x above) and moves only when the renderer does. Wall time
is still measured and logged, because it is the number a human cares about — it
is just not something a unit test on a shared machine can assert.

The bound moved 2000 → 4000 ms, set from the measurements rather than by feel:
~5x the worst reading under a saturated machine. CPU reads *higher* than wall
when idle, because it counts V8's background JIT and GC threads as well as the
main one; that is fine, since the bound cares about the ratio under load.

**What that bound is and is not.** It catches catastrophe — an algorithmic
regression on a 133k-character document would cost multiples — and it will not
catch a 2-3x slowdown. Detecting that reliably needs a benchmark with a recorded
baseline, which is a different tool. Said plainly in the test rather than left
for someone to discover.

### Files changed

| File | Why |
|---|---|
| `packages/editor/src/monaco/model-eol.dom.test.ts` | 60 s hook timeout, with the measurements that justify it |
| `packages/markdown/src/render.test.ts` | CPU-time assertion, 4000 ms bound, the comparison table |

### Tests actually executed

All at load averages that had just failed the originals:

```
# Monaco, at load 90 (it had failed 3/3 at load 80)
3 runs → Tests 3 passed (3), transform 9.71s / 10.82s / 10.21s
```

Transform exceeded 10 s in two of those three runs, so the old budget would
still have been failing.

```
# Markdown, verbose, under the other session's load
long.md (133878 chars) rendered in 815.0 ms CPU, 2188.7 ms wall  → passed
```

That single run is the whole ticket: 2188.7 ms wall would have failed the old
assertion; 815 ms CPU is the truth about the renderer.

```
# Both packages whole, at load 124
packages/editor    → Test Files 9 passed (9),  Tests 41 passed (41)
packages/markdown  → Test Files 6 passed (6),  Tests 54 passed (54)
```

### Known limitations

- **The Monaco transform still costs ~9 s every run.** The timeout stops it
  failing; it does not make it fast, and that cost is paid on every CI run of
  the editor package. Pre-bundling `monaco-editor` through
  `test.deps.optimizer` would likely fix it properly, but it interacts with the
  existing `server.deps.inline` rule and is a config change this ticket does not
  own. Recorded as `MONACO-PREBUNDLE`.
- `process.cpuUsage()` is process-wide, so it counts V8 background threads and
  would count sibling tests if this file ever shared a process. Vitest's default
  `forks` pool gives it its own; a future pool change would quietly widen the
  reading.
- Neither test now fails for slowness within a factor of ~5. That is deliberate
  and stated, but it does mean neither is a performance gate.

### Follow-up tasks

| Ticket | Why |
|---|---|
| `MONACO-PREBUNDLE` | pre-bundle `monaco-editor` so the 9 s transform is not paid every run |
| `MARKDOWN-BENCH` | a real benchmark with a recorded baseline, if a performance gate is actually wanted |

### Remaining risks

Both fixes were verified under load generated by other agents on this machine,
which is representative but not controlled. Neither has been seen on Linux CI,
which is dark; the Monaco transform in particular could be slower there, and
60 s is a guess above a measured 11 s rather than a measured Linux figure.
