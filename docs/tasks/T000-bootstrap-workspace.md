# Task T000 — Bootstrap workspace

- **Phase:** 0
- **Status:** review
- **Depends on:** –

## Objective

Create the repository structure, the Tauri 2 desktop app with a React/TypeScript
frontend, the Rust workspace, and the formatting/lint/test commands so that the
app launches and one command runs every check.

## Dependencies

None. Establishes the ground every later task builds on.

## Required reading

- `docs/Gritty IDE Project Plan.md` §6 (repository shape, runtime boundaries), §17 (T000 row), §18
- `docs/Gritty IDE Architecture Refinements.md` §9 (dependency policy), §15, §16
- ADR-002, ADR-003, ADR-008

## Allowed scope

Whole repository (bootstrap). Later tasks are scoped.

## Explicit non-goals

Core protocol (T010), workspace/file commands (T020/T030), Monaco (T040),
Markdown (T050), SQLite (T060), PTY (T090), fixtures, any plugin exposing
filesystem/shell/path to the WebView, signing/notarization, Windows.

## Required behavior

- pnpm workspace + Cargo workspace at the repo root; versions pinned in one place each.
- `apps/desktop`: Vite + React shell, thin Tauri composition root, strict CSP,
  minimal capabilities, one `health` command proving the WebView ↔ core IPC path.
- `crates/gritty-core`: UI-agnostic core with a health report and tests.
- `pnpm check` runs Biome, tsc, vitest, rustfmt, clippy `-D warnings`, cargo test.
- CI builds on Linux (always) and macOS (gated).
- Repository docs: README, AGENTS.md, architecture overview/security, ADRs,
  roadmap, Phase 0 checklist, task template, task specs T010–T090, platform matrix.

## Failure behavior

- A failing gate fails `pnpm check` (fail-fast, sequential).
- The frontend renders the invoke error if the core is unreachable (`pnpm web`).

## Acceptance criteria

- App launches via `pnpm dev`; the Rust log shows the `health` call from the WebView.
- `pnpm check` exits 0 on a clean checkout.
- `pnpm build` produces a launchable macOS bundle.
- CI green on Linux.

## Tests required

`pnpm check`; manual launch; release build and launch; CI.

## Commands to run

`pnpm install`, `pnpm check`, `pnpm dev`, `pnpm tauri build --bundles app`.

## Completion report

### Summary

Bootstrapped the monorepo on branch `t000-bootstrap`: pnpm 11 (corepack) with a
version catalog, Biome 2, Vitest 4, TypeScript 6.0.3, Cargo workspace pinned to
Rust 1.98.0, `crates/gritty-core`, `apps/desktop` (React 19 + Vite 8 + Tauri 2.11)
with a strict CSP and a `health` IPC round-trip, GitHub Actions CI, and the
documentation set including task specs for T010–T090.

### Files changed

- Root: `package.json`, `pnpm-workspace.yaml`, `pnpm-lock.yaml`, `Cargo.toml`,
  `Cargo.lock`, `rust-toolchain.toml`, `rustfmt.toml`, `biome.json`,
  `tsconfig.base.json`, `tsconfig.json`, `vitest.config.ts`, `.editorconfig`,
  `.gitignore`, `.nvmrc`, `README.md`, `AGENTS.md`, `CLAUDE.md`
- `crates/gritty-core/` (Cargo.toml, `src/lib.rs`)
- `apps/desktop/` (package.json, index.html, vite/vitest/tsconfig files,
  `src/{main.tsx,App.tsx,app.css,vite-env.d.ts}`, `src/core/{bridge.ts,health.ts,health.test.ts}`,
  `src-tauri/{Cargo.toml,build.rs,tauri.conf.json,capabilities/default.json,icons/,src/{main.rs,lib.rs,commands.rs}}`)
- `.github/workflows/ci.yml`, `.github/PULL_REQUEST_TEMPLATE.md`
- `docs/architecture/{overview,security}.md`, `docs/decisions/ADR-001..008 + README`,
  `docs/plans/{roadmap,phase-0-spike}.md`, `docs/agent/task-template.md`,
  `docs/tasks/README.md + T000/T010/T020/T030/T040/T050/T060/T090`,
  `docs/testing/platform-matrix.md`

### Tests actually executed (2026-09-02, macOS 26.6 arm64, Node 24.16, Rust 1.98.0)

| Command | Result |
|---|---|
| `pnpm install` | 101 packages resolved, lockfile created, no ignored build scripts |
| `cargo test -p gritty-core` | 2 passed |
| `pnpm check` | exit 0: Biome 20 files clean; tsc (root, desktop ×2) clean; vitest 1 file / 2 tests; `cargo fmt --check` clean; `cargo clippy --workspace --all-targets -- -D warnings` clean (after removing one unused import it caught); `cargo test --workspace` 2 passed |
| `pnpm dev` (scripted) | Vite on 1420; Rust log `health requested by webview` 7 s after start under the dev CSP; WebKit `21624.4.5.11.5`; HMR update observed 2 s after editing `App.tsx`; clean shutdown, no leftover processes |
| `pnpm tauri build --bundles app` | release build 1 m 34 s; `target/release/bundle/macos/Gritty.app` (6.1 MB) |
| release binary launched directly | `health requested by webview` after 3 s under the production CSP with `debug=false` |
| CI | pending: runs on the pull request (Linux always, macOS via the `ci:macos` label) |

Not executed: the negative UI check (renaming the command to see the error
envelope rendered) because the WebView cannot be observed from the CLI; DMG
bundling; manual clipboard/menu/IME checks (tracked in the platform matrix).

### Architectural decisions

- Hand-written app instead of an in-place `create-tauri-app` scaffold; the icon set
  is generated from a placeholder PNG with `pnpm tauri icon`.
- TypeScript 6.0.3 (bridge release) rather than 7.0.2; tsconfigs are 7-safe (ADR-008).
- Release profile keeps `panic = "unwind"` (panic containment later); `lto = "thin"`.
- Desktop crate builds only the default `rlib` (no mobile crate types), no opener plugin.
- Capabilities are an explicit subset of `core:default` (app/event/window/webview).
- CSP enabled in dev and release, already allowing `worker-src blob:` for Monaco.
- `version` omitted from `tauri.conf.json`; Tauri reads it from Cargo workspace
  inheritance (confirmed: health reports `appVersion 0.1.0`).
- Bundle identifier `io.github.justgritt.gritty` (permanent).
- `serde_json` is only a dev-dependency of the core; `thiserror`/`tokio`/`ts-rs` arrive with T010.

### Known limitations

- `health` is invoked twice in dev because of React StrictMode; harmless.
- Placeholder icon; no DMG; no signing.
- `pnpm web` runs the UI without a core, so the health call fails by design.
- The macOS CI job costs ~10× Linux minutes on this private repo; it is gated.

### Follow-up tasks

- T010 relocates `apps/desktop/src/core/bridge.ts` into `packages/protocol` and adds a
  lint forbidding `@tauri-apps/api` imports elsewhere.
- T020 creates `fixtures/` (small-repo, large-repo generator); T030 creates `weird-files`.
- Explicit later task: TypeScript 7.x upgrade (one catalog line).
- Real Gritty icon; DMG/signing/notarization in Phase 7.
- Machine note: uninstalling Homebrew's `rust` formula auto-removed its orphaned
  dependencies (`libgit2`, `llhttp`, `libssh2`); nothing else used them.
