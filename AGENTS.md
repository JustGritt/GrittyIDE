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
| `docs/tasks/` | one Markdown file per task, following `docs/agent/task-template.md` | update status and completion report |
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
T200 Workstream model (Gate C) → T210 UI → T300 Agent protocol + fake agent (Gate D)
  → T310 real agent, T320 context manifest, T330 decisions → T400 worktrees, T410 attention queue
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

1. Read the task file in `docs/tasks/` plus its required reading.
2. Stay inside "Allowed scope". If the task needs something outside it, stop and
   record the need instead of expanding scope.
3. Add or update tests for domain logic and regression-prone behavior. Consider
   cancellation, invalid input, process death and filesystem errors explicitly.
4. Run `pnpm check` (and the task's manual checks) before reporting.
5. Fill the completion report in the task file and in the PR description:
   summary, files changed, tests actually executed with results, architectural
   decisions, known limitations, follow-up tasks, remaining risks.
6. Update `docs/tasks/README.md` status and any ADR/architecture doc that a
   durable decision changed.

## Definition of done

Scope and acceptance criteria satisfied; no unexplained warnings; `pnpm check`
green; failure and cancellation behavior considered for async/process work; no
renderer-blocking work added; persistence changes ship with migrations; path and
security boundaries enforced; docs/ADR updated when a durable decision changed;
feature exercised in the running app where applicable; risks named, not hidden.
