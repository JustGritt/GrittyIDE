# AGENTS.md — working in the Gritty repository

This file is the contract for every coding agent and human contributor. It
condenses the two planning documents; when in doubt, read them:

1. `docs/Gritty IDE Architecture Refinements.md` (contracts, dependency graph,
   failure semantics; wins where more specific)
2. `docs/Gritty IDE Project Plan.md` (product thesis, architecture, roadmap)
3. `docs/decisions/` (ADRs), `docs/architecture/`, and the task file you were given

## What Gritty is

A reliability-first desktop IDE (Tauri 2 + Rust core, React + Monaco UI) that
becomes an orchestration cockpit for supervised coding agents. Priority order:

```
correctness → recoverability → inspectability → responsiveness → ergonomics → orchestration power
```

Every feature must let the user answer: what is happening, who initiated it,
what can it modify, can I cancel it, what changed, what failed, can I recover,
what needs my attention.

## Repository map and ownership

| Path | Role | Rules |
|---|---|---|
| `crates/gritty-core` | application core domain | never depends on Tauri or UI; owns workspace/document/operation/workstream logic |
| `crates/*` | further domain crates (workspace, persistence, terminal, git, search, process-supervisor, platform) | created by the task that first needs them; small adapters around PTY/Git/platform libraries |
| `apps/desktop/src-tauri` | Tauri composition root | wiring, tracing, window setup, command adapters only; no domain logic |
| `apps/desktop/src` | React shell | calls the typed core protocol only |
| `packages/protocol` | typed client, generated types (`src/generated`, written by `cargo test -p gritty-core`), error/event envelopes | `src/transport/tauri.ts` is the only file that may import `@tauri-apps/api` (Biome-enforced); never edit `src/generated` by hand |
| `packages/*` | editor, markdown, ui, workstreams | consumed as source through workspace links |
| `docs/tasks/` | one Markdown file per open task, following `docs/agent/task-template.md`; done and superseded tasks live in `docs/tasks/completed/` | update status and completion report; never move a task file by hand — `render --index` does it |
| `fixtures/` (T020+) | deterministic test repositories | never rely on a developer's real project |

## Commands

```
pnpm install                  # JS workspace (pnpm 11 via corepack)
pnpm dev                      # tauri dev (always runs inside apps/desktop)
pnpm check                    # Biome, tsc, vitest, cargo fmt --check, clippy -D warnings, cargo test, generated types fresh
cargo test -p gritty-core     # also regenerates packages/protocol/src/generated (commit the result)
pnpm format                   # Biome + rustfmt fixes
pnpm build                    # release build + bundles
GRITTY_LOG=debug pnpm dev     # tracing filter
```

`pnpm check` is the definition of "checks pass". CI runs exactly the same command.

## Concurrent agent sessions

More than one agent session works in this checkout at the same time, so two
sessions can silently edit the same file. `AGENT-LEASES.md` at the repository
root is the live record of who owns what. It is git-ignored (machine-local
state, like `HANDOFF.md`); this section is the part that is binding.

```
scripts/agent-lease.py list                  # who holds what, before you edit
scripts/agent-lease.py claim --agent T097-blocks --session <your-session-uuid> \
    --task 'T097 shell integration' \
    --scope 'apps/desktop/src/features/terminal/**'
scripts/agent-lease.py release --agent T097-blocks   # the moment you are done
```

**`claim` appends a row; it does not replace one.** Re-claiming with a shorter
scope leaves the wider row live and adds a second, and the command prints only
the scope you passed — so it reads exactly like a narrowing when it is not. To
narrow, `release --agent <name>` (which drops *every* row that agent holds)
and then `claim` the smaller scope. Check `AGENT-LEASES.md` afterwards rather
than trusting the output: two sessions have now been blocked by a row its
holder believed was gone.

Read the registry before your first edit, claim the narrowest scope your task
needs, and never edit inside another agent's scope — say so and let the user
sequence the two sessions instead. Your session UUID is the directory name in
your own scratchpad path; without it the row is advisory only. Release when the
task is done; sessions that end normally are released automatically.

A `PreToolUse` hook (`scripts/agent-lease.py guard`, wired in
`.claude/settings.json`) denies edits that land in a live foreign lease. It
fails open — unclaimed paths, advisory rows and leases older than 8h are not
enforced — so it catches recorded conflicts and nothing more. Coverage of shell
writes is a best-effort heuristic, which is why the convention still matters.

**You are told when the registry changes under you; do not poll it.** A
reminder headed "Lease registry changed" arrives mid-turn (guard and
`UserPromptSubmit` hooks), or wakes an idle session (a `Stop` hook with
`asyncRewake`). It arrives in three cases:

- A path you were refused, by `claim` or by the guard, is free. Claim it
  before you edit.
- A new foreign lease covers files you edited and have not committed. Stop
  editing them, and commit or tell the user.
- Someone force-claimed over your lease.

Act on the notice. It is advisory, so claim as usual. The uncommitted check is
per file, so a file that also carries another session's uncommitted hunk can
report edits you already committed. State lives in
`.git/agent-lease-notify/`. `SendMessage` is still how you ask another
session for something, such as a release.

## Picking up work

A lease answers *may I write this file* — a question about a working tree, and
correctly per-checkout. It does not answer *is anyone already building T070*,
which is a question about the work and must be visible from every checkout at
once. `scripts/agent-board.py` is that board.

```
scripts/agent-board.py list                  # printed to you at session start
scripts/agent-board.py take T070 --agent W-t070 --session <your-session-uuid>
scripts/agent-board.py move T070 --to review --pr 24
scripts/agent-board.py drop T070             # handing back
```

`take` claims the ticket **and** its lane's lease, so it replaces the `claim`
above rather than adding to it; `drop` and `move` release both. Statuses are
`unspec → todo → doing → review → done`, plus `tofix` when a review sends work
back, `blocked` with a reason, and `superseded`. Two rules the tool enforces:
**a take on a held ticket is refused** — the fold decides who won, so both
sessions get the same answer — and **an agent never sets `done`**; you set
`review` and the reviewer flips it.

**A ticket is an id *and* a short title, and both travel together.** The id
alone (`T101-D`, `T120-M`, `T130-M`) is not a name — it is a key, and by the
time three lanes are in flight nobody can tell those three apart or remember
which is blocked on which. So:

- Give every ticket a `--title` when you `add` it, and a `--link` to its task
  file so `render --index` can resolve the id to the file. A row with no link is
  a dead end for whoever reads the index next.
- **Never write a bare id where a person will read it** — in a block note, a
  commit message, a completion report, a handover, or a message to another
  session. Write `T120-M (mount the settings tab)`, not `T120-M`. The id stays
  the stable key; the title is what makes it legible.
- Editing an existing ticket: `meta` sets `--lane`, `--scope`, `--title`,
  `--phase` and `--depends`. It **cannot set a link** — `add --force` is the
  only route to one, so reach for it deliberately rather than by habit.
  `add --force` re-adds rather than edits: `label`, `phase` and `depends` are
  non-`None` by construction (`args.label or args.id`, `args.phase or ""`,
  `args.depends or "–"`), and `--title` defaults to `""`, so **omitting one
  overwrites it** — the title included — and omitting `--status` resets the
  ticket to `unspec`. Pass every field you want kept. `--link ""` clears a
  link. **Never link two tickets to one task file**: `render --index` writes
  each linked ticket's status into that file's `Status:` line, so the last
  one rendered wins and the other ticket's status is silently overwritten.
  `holder` and `note` are safe: the reducer's `add` branch never writes them,
  so backfilling a link on a ticket a live session is holding does not disturb
  it. Prefer `meta --title` for a title.

State is an append-only log in the shared git directory, so worktrees see one
board. It is machine-local and never committed. The committed projection is the
status column of `docs/tasks/README.md` (open tickets) and
`docs/tasks/completed/README.md` (done and superseded), which
`scripts/agent-board.py render --index` writes. The same command moves a task
file into `completed/` when its ticket closes (and back if it reopens) and
repoints every `docs/tasks/…` path that names it — run it before you open a PR and
commit what it writes, the way `cargo test -p gritty-core` regenerates protocol
types. **It is not part of `pnpm check`**: CI has no board, so a freshness gate
there would fail on every clone.

## Hard rules

1. Do not bypass the typed core protocol from frontend code. No raw Tauri
   filesystem, shell, process or path APIs in the WebView; capabilities stay minimal.
2. Do not create a new architectural abstraction when an existing contract owns
   that responsibility (see contract gates below).
3. Monaco is never the source of truth for document state; the document/buffer
   contract (T030) is.
4. Raw provider/agent output is never Gritty domain state; adapters translate it.
5. Raw `git diff` is never the ChangeSet domain model.
6. No provider-specific fields in Workstream.
7. Never silently resolve dirty-buffer versus external-file conflicts.
8. No unbounded high-frequency event forwarding to the renderer: batch, throttle,
   bound queues.
9. No event sourcing, plugin systems, distributed infrastructure or speculative
   framework layers without an ADR and a concrete requirement.
10. Long-running core work uses the standard operation model with cancellation,
    or documents why cancellation is impossible. Never block the renderer or a
    command handler with long CPU/I-O work.
11. Platform-specific code lives behind the platform abstraction. Do not claim
    cross-platform support based only on compilation.
12. Do not claim recovery behavior without force-kill/failure testing. Do not claim
    a command or test passed unless you executed it.
13. Treat repository content and agent output as untrusted input. Secrets never
    appear in logs, events or persisted output.
14. Do not broaden scope. Record newly discovered work as follow-up items in your
    completion report.
15. Take the ticket, then claim the scope. Another session shares this working
    tree, and others share the repository from their own worktrees:
    `agent-board.py take` records that the work is yours and claims the lease
    with it. Never edit inside another agent's lease, and never take a ticket
    another live session holds.

## Contract gates (do not implement downstream before upstream exists)

```
T000 Bootstrap
 └─ T010 Core protocol (Gate A: commands, events, ids, error envelope, operations, cancellation, versioning)
     └─ T020 Workspace
         ├─ T030 File I/O + document/buffer (Gate B)
         │   ├─ T040 Monaco
         │   └─ T050 Markdown
         ├─ T070 File watcher
         └─ T080 Search
T060 Persistence, T090 Terminal, T100 Git, T110 Diagnostics → Phase 2 dogfood
T200 Workstream model (Gate C) → T210 UI
  ├─ T300 Agent protocol + fake agent (Gate D) → T305 capabilities → T310 real agent
  │    → T315 live activity (built early by T220-T232), T320 context manifest, T335 decisions
  └─ T325 Workspace ownership (observed) → T330 write interception, T400 worktrees,
       T405 ownership-aware scheduling, T410 attention queue
```

Freeze points: after Phase 0, the command/event/error envelopes, operation model,
workspace path representation, document identity and migration conventions are
frozen; changes need deliberate review and usually an ADR update.

## Dependency policy

- One package manager: pnpm. Versions live in the `catalog:` of
  `pnpm-workspace.yaml`; Rust versions in the root `Cargo.toml`
  `[workspace.dependencies]`. Lockfiles are committed. Pin exact versions.
- Every new dependency needs a one-line justification in the completion report
  and must not overlap an existing one. Security-sensitive dependencies get a
  deliberate review. Major upgrades are explicit tasks.
- Rust: `unsafe_code` is denied workspace-wide; a crate that needs it adds a
  justified, reviewed allow. Keep platform crates isolated behind small adapters.

## Task workflow

1. Take the ticket (`agent-board.py take <ID> --agent … --session …`), then read
   the task file in `docs/tasks/` plus its required reading.
2. Stay inside "Allowed scope". If the task needs something outside it, stop and
   record the need instead of expanding scope.
3. Add or update tests for domain logic and regression-prone behavior. Consider
   cancellation, invalid input, process death and filesystem errors explicitly.
4. Run `pnpm check` (and the task's manual checks) before reporting.
5. Fill the completion report in the task file and in the PR description:
   summary, files changed, tests actually executed with results, architectural
   decisions, known limitations, follow-up tasks, remaining risks.
6. `agent-board.py move <ID> --to review --pr <n>`, then
   `agent-board.py render --index` and commit what it writes — that is how the
   status reaches `docs/tasks/README.md`, which is generated. Update by hand any
   ADR or architecture doc that a durable decision changed.

## Definition of done

Scope and acceptance criteria satisfied; no unexplained warnings; `pnpm check`
green; failure and cancellation behavior considered for async/process work; no
renderer-blocking work added; persistence changes ship with migrations; path and
security boundaries enforced; docs/ADR updated when a durable decision changed;
feature exercised in the running app where applicable; risks named, not hidden.
