# Roadmap

Phases and exit criteria from the project plan (§15) with the refined sequence
from the architecture addendum (§13). Effort figures are indicative.

This file is the *shape* of the work and goes stale between edits. For what any
task's status actually is, and who is building it right now, read the board:
`scripts/agent-board.py list`. Its committed projection is the table in
`docs/tasks/README.md`; a status written here is prose, not a record.

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

## Phases 1–2 and Gate D, specified ahead of implementation

Six task files were written in one batch on 2026-09-06 — T070, T080, T100,
T110, T300 and T326 — taking the board from fourteen unspecified tasks to
eight. None of them is started. Writing a spec is not starting the task, and
the sentence above still holds: T070, T080, T100 and T110 wait on the Phase 0
runtime decision, and their `Depends on` rows say `ADR-001` for that reason.
T300 and T326 sit in Phase 4 behind Gate C rather than behind the runtime
decision.

Three of the six raise a question the spec deliberately does not answer:

- **T080** recommends the in-process `grep`/`ignore` library family over
  spawning `ripgrep`: cancellation through the existing operation model
  instead of a second process-tree kill path, and no bundled binary whose
  support would be claimed on a platform nobody has run. It needs an ADR.
- **T100** recommends `gix` over shelling out to `git` or wrapping `git2`.
  Repository content is untrusted input (rule 13), and parsing another
  program's porcelain while inheriting the user's Git configuration as
  ambient authority is the worse of the three. It needs an ADR.
- **T300** proposes no persistence for an owned `AgentRun`, which sits
  awkwardly beside ADR-005's "state and results are persisted" and the
  recoverability priority. A fake agent has nothing to lose across a restart
  and a real one does, so the question is due before T310, not before T300.

T300 also settles a naming collision the coordination plan carries into the
code: §§5–7 call things `AgentSession` and `AgentActivity`, and both names
already mean something else in `agent_session/wire.rs`. Everything an owned
run brings is `AgentRun`-prefixed, so Lane A and Lane O cannot come to share a
type by accident.

None of the six claims a `PROTOCOL_VERSION`. The per-lane reservations in
`execution-board.md` are a planning hint that has already been overtaken once
— 2.11 was reserved for T070 and spent by T223 — so each spec takes the next
free value from `docs/merge-queue.md`, which is the record, immediately before
its PR opens.

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

## Phases 4–5, renumbered (ADR-019)

`docs/plans/Gritty Agent Runtime Coordination.md` §36 sequences the
orchestration work more finely than the plan's original T300/T310/T320/T330/
T400/T410 did, and ADR-019 adopts it:

```
T300 Agent event protocol + deterministic fake agent   (Gate D)
     ↳ split into T300-C · T300-FAKE · T300-CORE · T300-CMD (PLAN-FANOUT)
T305 Agent capability model
T310 First real adapter
T315 Live agent activity        ← already built, by T220/T230/T231/T232
T320 Context manifest
T325 Workspace ownership registry                      ✔ (this branch)
T330 Write interception         ← needs an amendment to ADR-018
T335 Decision interruption      ← what older documents call T330
T400 Worktree isolation
T405 Ownership-aware scheduler
T410 Attention queue
```

**The sequence above is unchanged; the granularity is not.** `PLAN-FANOUT`
(2026-09-06) split T300 and T222 into per-crate tickets behind a wire contract,
and added the lanes to build them in parallel — `docs/plans/execution-board.md`
§ Waves is the record, and the task files carry a pointer at the top. Nothing
here was resequenced: T300 is still Gate D and still gates T305/T310/T320.

Two things this changes, and one it does not. **`T330` means write
interception now**; decision interruption is `T335`, and live references were
retargeted. **`T315` is already shipped** by the agent-observability tasks, so
its row says superseded rather than re-specifying work that exists. Completion
reports keep the numbers they were written with — history is not edited to
match a later decision.

**T325 does not gate on T300.** A lease is held by a workstream (T200), not by
a run, so ownership is usable — and testable — before Gritty owns a single
agent. What it deliberately does *not* do is stop a write: that is T330, and it
is an ADR amendment before it is code.

## Success criteria for the product thesis (plan §22)

Editor failures are rare and never interrupt unrelated work; basic editing feels
close enough to a conventional IDE; the state of several coding tasks is
understandable without reading transcripts; questions, blockers and review-ready
changes surface clearly; three concurrent workstreams are easier than three
windows; decisions and task state survive provider changes and restarts;
resource usage stays acceptable with agents, terminals and language tooling
running; the improvement justifies a Windows port.
