# Gritty

Agent-native desktop development environment. A dependable code editor first; an
orchestration cockpit for supervised coding agents when they are available, and
still fully useful when they are not.

**Status:** Phase 0 (architecture spike). Nothing here is usable as an editor yet.
See `docs/plans/roadmap.md` for where this is going and `docs/plans/phase-0-spike.md`
for the current checklist.

## Stack

| Layer | Technology | Location |
|---|---|---|
| Desktop shell | Tauri 2 (Rust) | `apps/desktop/src-tauri` (thin composition root) |
| UI | React 19 + TypeScript + Vite 8, Monaco from T040 | `apps/desktop/src`, `packages/*` |
| Core domain | Rust workspace crates, UI-agnostic | `crates/*` |
| Platforms | macOS first (dogfood), Linux continuously buildable in CI, Windows deferred | |

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
| `pnpm check` | Every gate: Biome, tsc, vitest, rustfmt, clippy `-D warnings`, cargo test |
| `pnpm format` | Apply Biome and rustfmt fixes |
| `pnpm build` | Release build and bundles under `target/release/bundle/` |
| `pnpm tauri <args>` | Any Tauri CLI command, executed inside the app package |

Logging: `GRITTY_LOG=debug pnpm dev` (a `tracing` env filter).

## Repository map

```
apps/desktop/          Tauri app: React frontend (src/) + Rust shell (src-tauri/)
crates/gritty-core/    Application core; never depends on Tauri
crates/*               Further domain crates arrive with their tasks (workspace, persistence, terminal, ...)
packages/*             Frontend packages arrive with their tasks (protocol, editor, markdown, ...)
docs/                  Planning docs, architecture, ADRs, plans, task specs, test matrices
.github/workflows/     CI (Linux always, macOS gated)
```

## Documentation

- `docs/Gritty IDE Project Plan.md`: product definition, architecture, delivery plan.
- `docs/Gritty IDE Architecture Refinements.md`: contracts, dependency graph and
  failure semantics; wins over the plan where it is more specific.
- `docs/architecture/`: runtime boundaries and the security model.
- `docs/decisions/`: architecture decision records.
- `docs/plans/`: roadmap and the executable Phase 0 checklist.
- `docs/tasks/`: task specs for coding agents, one file per task.
- `AGENTS.md`: the rules every coding agent (and human) follows in this repo.

## License

MIT. See `LICENSE`.
