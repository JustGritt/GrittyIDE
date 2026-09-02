# ADR-008: Repository tooling and Rust/TypeScript type sharing

- **Status:** Accepted
- **Date:** 2026-09-02

## Context

The dependency policy asks for mature tooling, one package manager, committed
lockfiles and pinned versions. Independent coding agents need one command that
runs every gate and a single way to share protocol types.

## Decision

- **Package manager:** pnpm 11 through corepack (`packageManager` in
  `package.json`); shared versions in the `catalog:` of `pnpm-workspace.yaml`,
  `saveExact`, no install scripts unless approved (`allowBuilds`).
- **Lint/format (TS, JSON, CSS):** Biome 2 with a single `biome.json`.
- **Tests:** Vitest 4 (root `test.projects`) and `cargo test`.
- **Rust:** exact toolchain pin in `rust-toolchain.toml`; workspace lints
  (`unsafe_code = "deny"`, clippy `all` warn, CI `-D warnings`); release profile
  keeps `panic = "unwind"` so in-process worker panics can be contained.
- **TypeScript:** 6.0.3, the bridge release with TypeScript 7 semantics. Moving
  to 7.x is a deliberate later task (one catalog line; tsconfigs are already
  7-safe).
- **Type sharing:** `ts-rs` exports Rust protocol types into
  `packages/protocol`; commands and events are wrapped by a hand-written typed
  client (T010). `tauri-specta` was rejected because it is still a release
  candidate.
- **Identity:** product name `Gritty`, bundle identifier
  `io.github.justgritt.gritty` (permanent: it names the app-data directory).
- **CI:** GitHub Actions; the Linux job always runs `pnpm check` plus a
  no-bundle build; the macOS job is gated (main, manual dispatch, PR label
  `ci:macos`) because macOS minutes are expensive on a private repository.
- **Single gate:** `pnpm check` = Biome, tsc, vitest, rustfmt, clippy, cargo test.

## Consequences

- Agents never need to choose tooling; they run `pnpm check`.
- Dependency changes are visible in two files (`pnpm-workspace.yaml`, `Cargo.toml`).

## Revisit when

TypeScript 7 tooling is mature in editors, or `tauri-specta` reaches a stable release.
