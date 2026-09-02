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

## Success criteria for the product thesis (plan §22)

Editor failures are rare and never interrupt unrelated work; basic editing feels
close enough to a conventional IDE; the state of several coding tasks is
understandable without reading transcripts; questions, blockers and review-ready
changes surface clearly; three concurrent workstreams are easier than three
windows; decisions and task state survive provider changes and restarts;
resource usage stays acceptable with agents, terminals and language tooling
running; the improvement justifies a Windows port.
