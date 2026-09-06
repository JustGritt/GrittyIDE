# Roadmap

Phases and exit criteria from the project plan (§15) with the refined sequence
from the architecture addendum (§13). Effort figures are indicative.

| Phase | Effort | Scope | Exit criterion |
|---|---|---|---|
| 0 — Architecture and runtime spike | 1–2 weeks | Tauri/WebView viability, Monaco, filesystem boundary, typed protocol, PTY, Markdown, persistence fundamentals, failure containment, Linux build, performance baseline | ADR-001 go/no-go with measurements (`phase-0-spike.md`) |
| 1A — Workspace + document foundation | part of 2–4 weeks | workspace lifecycle, explorer, document/buffer model, safe read/write, dirty buffers, Monaco, tabs, external modification handling | Gritty is safe for basic code editing |
| 1B — Markdown + persistence + recovery | | Markdown source/preview, session restoration, dirty-buffer recovery, cursor/layout state | force-kill/restart does not make routine editing feel unsafe |
| 2 — Developer essentials | 2–4 weeks | search, terminal, Git status/diff, watcher hardening, diagnostics, settings | usable for meaningful daily repository work (dogfood) |
| 3 — Workstreams | 2–3 weeks | persistent workstream/decision/change model with no AI dependency | a workstream can be managed manually from goal to review/done |
| 4A — Agent protocol + fake agent | part of 2–4 weeks | provider-neutral protocol, run lifecycle, normalized events, capabilities, context manifest, cancellation, deterministic fake backend | orchestration testable completely offline |
| 4B — First real agent | | one backend through an adapter | one bounded task delegated, interrupted, resumed, cancelled and reviewed without blocking the UI |
| 5 — Parallel orchestration | 3–5 weeks | branches/worktrees, concurrent runs, attention queue, conflicts, review workflow | multiple workstreams materially easier to supervise than separate windows |
| 6 — Language intelligence | as needed | LSP lifecycle and selected servers | navigation/diagnostics improve without an extension system |
| 7 — Linux hardening | parallel + focused pass | packaging, WebView/PTY/filesystem validation, distro policy | supported Linux release |
| 8 — Windows evaluation/port | after validation | WebView2, ConPTY, path/shell semantics, installer/signing, QA | only if Gritty has proven its workflow value |

## Immediate execution order

```
T000 Bootstrap workspace                         ✔ (this branch)
T010 Core protocol + operation/event/error conventions
T020 Workspace open/list (+ fixtures)
T030 File I/O + document/buffer contract
T040 Monaco shell
T050 Markdown
T060 Persistence/recovery spike
T090 PTY spike
Phase 0 failure/performance/platform tests        (docs/plans/phase-0-spike.md)
ADR-001 confirm or reject Tauri
```

T070 (watcher), T080 (search) and the rest of Phases 1–2 start only after the
Phase 0 runtime decision is recorded. Task specs live in `docs/tasks/`.

## Agent observability (T221 and T230–T232 built; T222–T224 specified only)

T220 landed read-only discovery of the agent sessions already running on the
machine. The rest finishes that surface without crossing into orchestration.

The landing that mattered first was the **agent rail**, and it has landed:
T230 (a transcript follower and a provider-neutral timeline, no UI), T221 (the
hook bridge, which earns `BLOCKED` — the one status the transcript cannot
produce), T231 (a third region beside the editor, sessions coloured by status)
and T232 (the feed, and opening a file the agent touched). Together they answer
*what is this agent doing* next to the code it is changing. All four are at
`review`: built, `pnpm check` green, and never once driven by a pointer.

Still specified only: T222 (Codex), T223 (notifications plus a
session↔workstream link) and T224 (reveal the terminal).

They sit in Phase 3 beside T210 and **do not gate on T300**, because none of
them owns a run. This section used to say implementation waited for ADR-001.
It did not: the four above were built while the runtime decision is still
formally `Proposed`. Nothing in them rests on it — no new runtime assumption,
and the Phase 0 measurements are untouched — but the gate is real, and an
exception to it belongs in writing rather than in the commit log.

The comparison that motivated them is in ADR-018's context: Herdr ships the
runtime layer (a detached daemon, 15+ agents, a socket API) and explicitly
refuses worktree isolation, task graphs, approval policies and durable event
history. That refusal list is Phases 3–5 here. Gritty should not rebuild the
multiplexer; it should be the layer Herdr says it will not be.

## Success criteria for the product thesis (plan §22)

Editor failures are rare and never interrupt unrelated work; basic editing feels
close enough to a conventional IDE; the state of several coding tasks is
understandable without reading transcripts; questions, blockers and review-ready
changes surface clearly; three concurrent workstreams are easier than three
windows; decisions and task state survive provider changes and restarts;
resource usage stays acceptable with agents, terminals and language tooling
running; the improvement justifies a Windows port.
