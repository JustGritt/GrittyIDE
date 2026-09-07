# Task SIDECAR-FRESH-CHECKOUT — A fresh clone must build

- **Phase:** 3 (dogfood)
- **Status:** review
- **Depends on:** T225 (the hook sidecar this stages)

## Objective

Make a clean checkout buildable. Two agents hit this independently and both
worked around it by hand.

## The failure, reproduced

`tauri.conf.json` declares `bundle.externalBin: ["binaries/gritty-agent-hook"]`,
and `src-tauri/binaries/` is `.gitignore`d (correctly — it is a build artifact).
`tauri_build::build()` validates that path at **compile** time, so with nothing
staged:

```
$ rm -rf apps/desktop/src-tauri/binaries
$ cargo check -p gritty-desktop
resource path `binaries/gritty-agent-hook-aarch64-apple-darwin` doesn't exist
```

Only `beforeBuildCommand` staged it, and that runs for `tauri build` alone.
`beforeDevCommand` is `pnpm dev` (vite) and never staged anything.

**This was not only a newcomer problem.** CI runs `pnpm install --frozen-lockfile`
then `pnpm check`, whose `check:rust` is `cargo test --workspace` — which
includes `gritty-desktop`. So **CI could not have run the Rust half on a fresh
clone either**. It has been invisible because the self-hosted macOS runner keeps
a warm tree with a binary left over from a previous bundle build, and the Linux
job has been dark since the Actions allowance ran out.

## Allowed scope

- `apps/desktop/scripts/prepare-sidecar.mjs`
- `apps/desktop/package.json`, root `package.json`
- `apps/desktop/src-tauri/build.rs`, `.github/workflows/**` (neither needed)

## Explicit non-goals

- Committing a binary. `binaries/` stays ignored.
- Touching `tauri.conf.json` — see the rejected option below, and another
  session had it modified.
- Changing what a real bundle stages.

## Required behavior

1. `git clone && pnpm install` leaves `cargo check -p gritty-desktop` working.
2. A contributor with no Rust toolchain still gets a successful `pnpm install`.
3. `tauri build` still stages a **release** binary and still fails loudly if it
   cannot — a bundle that silently omits the hook reports `hookAvailable: false`
   with nothing to say why.

## Failure behavior

- No `rustc`/`cargo`: warn, name the command that fixes it, exit 0. The install
  survives; nothing is silently broken that was not already.
- The same script run for a bundle keeps exiting non-zero on any failure.

## Acceptance criteria

A genuine fresh clone, `pnpm install`, then `cargo check -p gritty-desktop`
succeeds — executed, not reasoned about.

## Commands to run

```
rm -rf apps/desktop/src-tauri/binaries && cargo check -p gritty-desktop   # reproduces
git clone <repo> /tmp/fresh && cd /tmp/fresh && pnpm install --frozen-lockfile
cargo check -p gritty-desktop
```

## Completion report

### Summary

A root `postinstall` stages the sidecar, so the documented first step
(`pnpm install`, in both `AGENTS.md` and `README.md`) now leaves the repository
buildable.

It stages a **debug** build via a new `sidecar:dev` script, because the thing
that was broken is a path-existence check at compile time — a debug binary
satisfies it, costs ~11 s instead of a release build, and `beforeBuildCommand`
still re-stages release for a real bundle. A useful side effect: it also
produces `target/debug/gritty-agent-hook`, which is exactly where
`resolve_hook_binary` looks in dev, so the hook integration now actually works
in `pnpm dev` on a fresh checkout instead of reporting itself unavailable.

`--optional` downgrades failure to a warning for that call only, so a
contributor without a Rust toolchain is not handed a broken `pnpm install` —
which would move the newcomer's failure earlier rather than fix it.

### Files changed

| File | Why |
|---|---|
| `package.json` | `postinstall` → the desktop package's `sidecar:dev` |
| `apps/desktop/package.json` | new `sidecar:dev` = `--debug --optional` |
| `apps/desktop/scripts/prepare-sidecar.mjs` | `--optional` mode, and the header explaining both callers |

### Tests actually executed

Reproduction, in the working tree:

```
rm -rf apps/desktop/src-tauri/binaries; cargo check -p gritty-desktop
→ resource path `binaries/gritty-agent-hook-aarch64-apple-darwin` doesn't exist
```

The fix, on a **genuine fresh clone** (`git clone --no-hardlinks` into a scratch
directory, the three changed files copied in):

```
pnpm install --frozen-lockfile
→ postinstall: ... Finished `dev` profile in 11.39s
→ postinstall: gritty-agent-hook: staged the debug build as .../binaries/gritty-agent-hook-aarch64-apple-darwin
→ Done in 13.1s

cargo check -p gritty-desktop
→ Finished `dev` profile in 51.69s     (exit 0)
```

Both failure paths, with `rustc` removed from `PATH`:

```
prepare-sidecar.mjs --debug --optional  → warns, names the fix, exit 0
prepare-sidecar.mjs                     → "cannot ask rustc for the host target triple", exit 1
```

The scratch clone and its 1.1 GB `target/` were removed afterwards.

### Architectural decisions

**Rejected: staging from `build.rs`.** It is the tidiest boundary in principle —
the Rust build is what needs the file — but it means a nested `cargo build` from
inside a build script, and the inner cargo would block on the outer build's lock
on the same `target/` directory. That is a deadlock, not a slowdown.

**Rejected for now: moving `externalBin` out of the always-applied config.** The
requirement really belongs to *bundling*, not to every compile, and Tauri
supports a bundle-time `--config` overlay. That is the better long-term shape,
but it changes packaging, which `pnpm check` does not exercise — claiming it
works without running a real bundle is exactly what rule 12 forbids, and a full
macOS bundle was not something this session could verify. It also means editing
`tauri.conf.json`, which another live session had modified. Recorded as
`SIDECAR-BUNDLE-CONFIG`.

**Chosen: `postinstall`,** because it matches the documented flow exactly. Both
`AGENTS.md` and `README.md` open with `pnpm install`, and CI runs it before
`pnpm check`, so the one step everybody already takes is the one that fixes it.

### Known limitations

- `pnpm install` now compiles a Rust crate (~11 s cold, near-zero warm). On CI
  this lands after `Swatinem/rust-cache` restores, so it is cached.
- pnpm skips lifecycle scripts when an install is a complete no-op ("Already up
  to date"). That is harmless — the binary is only absent when there was a real
  install to do — but it means `pnpm install` is not a way to *repair* a
  deleted `binaries/`. `pnpm --filter @gritty/desktop sidecar` is.
- Verified on macOS/aarch64 only. The script already asks `rustc` for the host
  triple rather than deriving one, so nothing here is platform-specific, but
  Linux CI is dark and Windows has not been run.

### Follow-up tasks

| Ticket | Why |
|---|---|
| `SIDECAR-BUNDLE-CONFIG` | move `externalBin` to a bundle-time config overlay so a compile never needs a staged binary; needs a real bundle run to verify |

### Remaining risks

CI still cannot prove any of this: the Linux job is dark until the Actions
allowance resets, and the macOS runner's tree is warm, which is what hid the bug
in the first place. The fresh-clone test above was run by hand and is the only
evidence.
