# Task T225 — Bundle the agent hook as a sidecar

- **Phase:** 3
- **Status:** done
- **Depends on:** T221 (hook bridge — the binary, the installer and
  `resolve_hook_binary`, all of which already exist)

## Objective

`gritty-agent-hook` is built by the workspace and found at runtime, but it is
not in the installer. A packaged Gritty therefore reports
`hookAvailable: false` and offers the user nothing, while the same code in
`pnpm dev` works. This task puts the binary in the bundle.

It is T221's own follow-up, and T221's Allowed scope named
`apps/desktop/src-tauri/tauri.conf.json` for exactly this. It was recorded in a
completion report and tracked nowhere — the failure mode
`docs/testing/review-findings.md` calls "a follow-up bullet is where work goes
to be forgotten politely".

## Dependencies

Nothing new. `resolve_hook_binary`
(`apps/desktop/src-tauri/src/core.rs:226`) already resolves
`GRITTY_AGENT_HOOK` first and then **a sibling of the running executable**,
which is where Tauri places an `externalBin`. That fallback was written for
`cargo run` and `target/debug`; a bundle's `Contents/MacOS/` is the same shape.

## Required reading

- `docs/decisions/ADR-018-agent-integration-bridge.md` — what the hook is, and
  the uninstall obligation that makes a broken path worse than no path
- `docs/architecture/security.md` — the boundary this must not widen
- `apps/desktop/src-tauri/src/core.rs:211-246` — `HOOK_BINARY_ENV` and
  `resolve_hook_binary`, including the doc comment that records this gap
- `crates/agents/src/integration.rs` — the installer that writes the resolved
  path into a provider's config
- `crates/gritty-core/src/agent_session/mod.rs:1057-1094` — where
  `hook_available` is computed and where install refuses without a binary
- `AGENTS.md` hard rules 1, 11, 12 and the dependency policy

## Allowed scope

- `apps/desktop/src-tauri/tauri.conf.json`
- `apps/desktop/scripts/**` (new)
- `apps/desktop/package.json`
- `.gitignore`
- `docs/tasks/README.md`, this file

## Explicit non-goals

- **No Rust change.** If `resolve_hook_binary` needs editing, the sidecar was
  put in the wrong place. The fallback is the contract this task builds to.
- **No new Tauri capability, and no shell plugin.** Gritty never executes the
  hook — the coding agent does, out of process, from its own configuration.
  A sidecar that Gritty could spawn would widen the boundary hard rule 1 draws
  for no gain.
- **No change to the installer, the config format or the uninstall path.**
- **No code signing or notarisation work.** Out of scope, and named as a risk
  below rather than silently assumed away.
- **No claim about Linux or Windows bundles.** Implement the platform-neutral
  build step; claim only the platform whose bundle was actually built and
  opened (rule 11, rule 12).

## Required behavior

1. `pnpm build` (and `pnpm tauri build`) produces a bundle whose executable
   directory contains `gritty-agent-hook` beside `gritty-desktop`.
2. The build step compiles the hook from this workspace at the same version as
   the app. It is never a checked-in binary: `apps/desktop/src-tauri/binaries/`
   is git-ignored, like every other build output.
3. `beforeBuildCommand` produces the sidecar before Tauri bundles, so a clean
   checkout builds without a manual pre-step.
4. The step is platform-neutral: it asks `rustc` for the host triple rather than
   hardcoding one, and it must not break CI's Linux
   `pnpm tauri build --no-bundle` (`.github/workflows`, `ci:build` label).
5. A packaged app launched from a terminal reports `hookAvailable: true` and
   offers the install action, with the path it names pointing at the bundled
   binary.

## Failure behavior

- **The hook fails to compile** → the build fails loudly at the
  `beforeBuildCommand`, before any bundle exists. A bundle that silently omits
  the sidecar is worse: `hookAvailable` would be false with nothing to say why.
- **`rustc` is not on PATH** → the script exits non-zero with the reason. It
  never guesses a triple.
- **The binary is missing at runtime anyway** (moved, quarantined, stripped by
  a signing step) → unchanged, and already correct: `resolve_hook_binary`
  returns `None`, `hook_available` is false, and install refuses rather than
  writing a config entry pointing at nothing.

## Acceptance criteria

- `ls Gritty.app/Contents/MacOS/` lists `gritty-agent-hook` **and**
  `gritty-desktop`. This is observed, not inferred from the config.
- The packaged binary launched with `GRITTY_LOG=debug` does **not** log
  `no agent hook binary`. The success path logs nothing, so this criterion is
  deliberately negative; the positive one — the integration surface reporting
  the hook as available — needs the UI and is manual-pass step 6.8.
- `git status` is clean after a build — nothing generated is tracked.
- `pnpm check` is unaffected: this task adds no test-visible behaviour and no
  Rust code.

## Tests required

There is no unit test for a packaging step, and pretending otherwise would be
the claim rule 12 forbids. What is required instead:

- **Manual, executed and reported:** `pnpm tauri build --debug --bundles app`,
  then `ls` the bundle, then launch
  `Gritty.app/Contents/MacOS/gritty-desktop` with `GRITTY_LOG=debug` and read
  the hook path out of the log.
- **Manual pass:** the new step in Session 6 (packaged app) is this task's
  standing regression check.
- Anything not run — the Linux and Windows bundles — is written down as not
  run, with the reason.

## Commands to run

```sh
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
pnpm sidecar                                   # the new step, on its own
pnpm tauri build --debug --bundles app
ls target/debug/bundle/macos/Gritty.app/Contents/MacOS/
pnpm check
```

## Completion report

**Summary.** `bundle.externalBin` now names the hook, and a
`beforeBuildCommand` step compiles it and stages it under the triple-suffixed
name Tauri looks for. No Rust changed: `resolve_hook_binary`'s
sibling-of-the-executable fallback was already the right contract, and the
bundled binary lands exactly where it looks.

**Files changed.**

- `apps/desktop/scripts/prepare-sidecar.mjs` (new, 76 lines)
- `apps/desktop/src-tauri/tauri.conf.json` — `bundle.externalBin`;
  `beforeBuildCommand` is `pnpm sidecar && pnpm build`
- `apps/desktop/package.json` — the `sidecar` script
- `.gitignore` — `**/src-tauri/binaries/`
- `docs/tasks/completed/T225-hook-sidecar.md` (this file), `docs/tasks/README.md`
- `docs/testing/phase-0-manual-pass.md` — steps 0.1 and 6.8 (with
  MANUAL-PASS-COVERAGE, which owns the rest of that file)

**Tests actually executed.**

| Command | Result |
|---|---|
| `pnpm sidecar` standalone | staged the release build as `…/binaries/gritty-agent-hook-aarch64-apple-darwin` |
| `pnpm tauri build --debug --bundles app` | bundle built in 45.03 s |
| `ls Gritty.app/Contents/MacOS/` | **`gritty-agent-hook` (1 361 360 B) beside `gritty-desktop`** — triple stripped, as predicted but not assumed |
| launch the bundled binary, `GRITTY_LOG=debug` | 29 log lines, migrations 1–4, roster of 11 discovered sessions. **No `no agent hook binary` line** |
| `git status` after the build | clean; the staged binary is ignored by `.gitignore:7` |
| `pnpm check` | exit 0 — 240 files by Biome, 69 test files / 531 tests, clippy and `cargo test --workspace` clean, generated types unchanged |

**On the evidence for `hookAvailable`.** The strongest available non-GUI
evidence is negative and is stated as such: `resolve_hook_binary` logs
`no agent hook binary; the hook integration is not offered` at DEBUG when it
finds nothing, that line is absent, and the same log carries many other DEBUG
lines — so the filter was live and the not-found branch was not taken. The
success path logs nothing (only the `GRITTY_AGENT_HOOK` branch has an `info!`),
so a positive assertion would need `agents.integrationStatus`, which only the
UI calls. **That is manual-pass step 6.8**, and it is why the step exists.

**Architectural decisions.**

- **No Rust change, deliberately.** Needing one would have meant the sidecar was
  in the wrong place. `core.rs:222-225`'s doc comment — recording this exact gap
  as unbuilt — is now stale in the good way and is updated by nothing, because
  the code it describes did not change.
- **No Tauri capability and no shell plugin.** Gritty never spawns the hook; the
  coding agent does, out of process, from its own configuration. `externalBin`
  is used purely as a *placement* mechanism, and nothing in the WebView can
  reach it — hard rule 1 is untouched.
- **The triple comes from `TAURI_ENV_TARGET_TRIPLE`, else `rustc --print
  host-tuple`.** Never from `process.platform`/`arch`, which disagree with the
  triple on every cross build and would fail at bundle time with a
  missing-file error that does not say why.
- **`--target` is passed only for a real cross build.** Adding it on a host
  build changes cargo's output directory and rebuilds the world for nothing.
- **The profile follows `TAURI_ENV_DEBUG`**, so `tauri build --debug` bundles a
  debug hook and a release build bundles a release one — the sidecar always
  matches the app it ships inside.
- **The script fails loudly.** Every failure path exits non-zero with the
  reason. A bundle that silently omitted the hook would report
  `hookAvailable: false` with nothing anywhere to say why, which is the failure
  this task exists to remove.

**Known limitations.**

- **Only the macOS `app` bundle was built and opened.** No `dmg`, no `deb`, no
  Windows. The script is written to be platform-neutral and is not *claimed* to
  work there (rule 11).
- **CI has not run this.** `beforeBuildCommand` also runs for the Linux
  `pnpm tauri build --no-bundle` job, so the script executes there too — but
  GitHub Actions will not start a job at all until the account's billing is
  fixed, so no Linux evidence exists. Unverified, not assumed.
- **Code signing and notarisation are untouched.** An unsigned sidecar inside a
  signed bundle is a known packaging hazard; nothing here is signed today, so
  it is recorded rather than solved.
- **`pnpm dev` is unchanged.** Dev still relies on `target/debug/gritty-agent-hook`
  existing, which is why manual-pass step 0.1 now builds it. Putting the sidecar
  step in `beforeDevCommand` would slow every dev start for a case one command
  in Session 0 covers.

**Follow-up tasks.**

- Sign the sidecar when app signing is set up (no ticket: there is no signing
  today).
- If the Linux bundle ever ships, confirm `externalBin` placement there the same
  way this task confirmed macOS — by looking, not by inference.
