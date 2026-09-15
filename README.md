# Gritty

Agent-native desktop development environment. A dependable code editor first; an
orchestration cockpit for supervised coding agents when they are available, and
still fully useful when they are not.

**Status:** pre-release, built and run from source on macOS. Unsigned, with no
release build published. On `main` today:

- a Monaco editor with tabs, split panes and crash recovery for unsaved buffers;
- Markdown preview;
- a file explorer with create, rename, move and delete to the Trash;
- quick open;
- a terminal dock;
- Git status, history, staging and commit;
- a settings page;
- workstreams with decisions and an attention queue;
- a view of the Claude Code sessions already running on the machine, with
  their timelines and permission prompts.

Some of this is still at review. Runs that Gritty starts itself are available
only in development builds. Linux is unverified while its CI is dark.

The goal is parity with Orca on the core loop (task as worktree, agent in a
terminal, review, pull request). Where this is going, and in what order:
`docs/plans/project-plan.md` and ADR-035. What is being built right now:
`scripts/agent-board.py list`.

## Stack

| Layer | Technology | Location |
|---|---|---|
| Desktop shell | Tauri 2 (Rust) | `apps/desktop/src-tauri` (thin composition root) |
| UI | React 19 + TypeScript + Vite 8, Monaco, xterm | `apps/desktop/src`, `packages/*` |
| Core domain | Rust workspace crates, UI-agnostic | `crates/*` |
| Platforms | macOS first; Linux built in CI when CI runs (dark until 1 Oct 2026); Windows deferred (ADR-002) | |

## Prerequisites

- macOS 14+ with Xcode Command Line Tools. Linux: the apt packages listed in
  `.github/workflows/ci.yml`.
- Node 24 (`.nvmrc`) with corepack: `corepack enable pnpm`
  (fallback: `npm i -g pnpm@11.25.0`).
- Rust via rustup. `rust-toolchain.toml` pins the exact version and rustup installs
  it on the first `cargo` call. If Rust was installed through Homebrew's `rust`
  formula, uninstall it and put `/opt/homebrew/opt/rustup/bin` first on `PATH`,
  otherwise the pin is silently ignored.

## Commands

| Command | What it does |
|---|---|
| `pnpm install` | Install the JS workspace (lockfile is committed) |
| `pnpm dev` | Run the desktop app with hot reload (`tauri dev`) |
| `pnpm web` | Run only the Vite frontend in a browser (no core, health call fails by design) |
| `pnpm check` | Every gate: Biome, tsc, vitest, rustfmt, clippy `-D warnings`, cargo test, generated protocol types fresh |
| `pnpm format` | Apply Biome and rustfmt fixes |
| `pnpm build` | Release build and bundles under `target/release/bundle/` |
| `pnpm tauri <args>` | Any Tauri CLI command, executed inside the app package |

Logging: `GRITTY_LOG=debug pnpm dev` (a `tracing` env filter).

## Repository map

```
apps/desktop/          Tauri app: React frontend (src/) + Rust shell (src-tauri/)
crates/gritty-core/    Application core; never depends on Tauri
crates/*               Domain crates: workspace, persistence, terminal, git, git-write, search,
                       settings, agents, agent-hook, agent-fake, agent-claude
packages/protocol/     Typed core client; src/generated is written by `cargo test -p gritty-core`
packages/editor/       Monaco binding
packages/markdown/     Markdown rendering and link resolution
fixtures/              Deterministic test repositories
docs/                  Plan of record, architecture, ADRs, task specs, test matrices
.github/workflows/     CI (Linux on every push; macOS on a self-hosted runner, on demand)
```

## Documentation

- `docs/plans/project-plan.md`: the plan of record. Product definition,
  architecture as built, contract gates, phase status, dependency graph and open
  decisions.
- `docs/decisions/`: architecture decision records; an accepted ADR wins over the
  plan where it is more specific (ADR-035 owns the forward sequence).
- `docs/architecture/`: runtime boundaries, domain models and the security model.
- `docs/Gritty IDE Project Plan.md`, `docs/Gritty IDE Architecture Refinements.md`:
  the founding documents of 2 September 2026, frozen as history.
- `docs/tasks/`: task specs for coding agents, one file per task.
- `AGENTS.md`: the rules every coding agent (and human) follows in this repo.

## License

MIT. See `LICENSE`.
