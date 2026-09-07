// Builds `gritty-agent-hook` and puts it where Tauri expects a sidecar (T225).
//
// Tauri's `bundle.externalBin` names a path without its target triple and looks
// on disk for `<path>-<triple><ext>`; it then copies the file next to the app
// executable in the bundle, with the triple stripped. That is exactly where
// `resolve_hook_binary` (apps/desktop/src-tauri/src/core.rs) already looks — a
// sibling of the running executable — so no Rust change is needed to find it.
//
// This runs from `beforeBuildCommand`, which means it also runs for
// `tauri build --no-bundle` on CI. Keep it cheap, platform-neutral, and loud
// when it fails: a bundle that silently omits the hook reports
// `hookAvailable: false` with nothing to say why.
//
// It also runs from the repository's `postinstall` (SIDECAR-FRESH-CHECKOUT),
// because `bundle.externalBin` is checked by `tauri_build` at **compile** time:
// without a staged file, a fresh clone cannot even `cargo check -p
// gritty-desktop`, so `pnpm check`, `pnpm dev` and CI all fail on a clean
// checkout. That call passes `--debug --optional`:
//
//   --debug     a debug build is enough to satisfy a path-existence check, and
//               it is much cheaper than a release build on every install. A
//               real bundle still re-stages release from `beforeBuildCommand`.
//   --optional  a missing Rust toolchain warns and exits 0 instead of failing
//               the install. Someone without cargo cannot build the desktop app
//               either way, and breaking `pnpm install` for them would move the
//               newcomer's failure earlier rather than fix it.

import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';

const CRATE = 'gritty-agent-hook';
const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, '..', '..', '..');
const outputDir = join(here, '..', 'src-tauri', 'binaries');

function run(command, args) {
  return execFileSync(command, args, { cwd: repoRoot, encoding: 'utf8' }).trim();
}

// `--optional` downgrades every failure to a warning: see the header.
const optional = process.argv.includes('--optional');

function fail(message) {
  if (optional) {
    console.warn(`${CRATE}: ${message}`);
    console.warn(
      `${CRATE}: skipping the sidecar. \`cargo check -p gritty-desktop\` will fail until ` +
        '`pnpm --filter @gritty/desktop sidecar` succeeds.',
    );
    process.exit(0);
  }
  console.error(`${CRATE}: ${message}`);
  process.exit(1);
}

// Tauri sets these for every hook command; they are absent when this script is
// run on its own (`pnpm sidecar`), which is why each has a fallback.
const env = process.env;
const profile =
  env.TAURI_ENV_DEBUG === 'true' || process.argv.includes('--debug') ? 'debug' : 'release';

// Ask rustc rather than deriving a triple from process.platform/arch: the two
// disagree on every cross build, and a wrong name fails at bundle time with a
// missing-file error that does not say why.
let host;
try {
  host = run('rustc', ['--print', 'host-tuple']);
} catch (error) {
  fail(`cannot ask rustc for the host target triple. ${error.message}`);
}
if (!host) fail('rustc printed no host target triple.');

const target = env.TAURI_ENV_TARGET_TRIPLE || host;
const extension = target.includes('windows') ? '.exe' : '';

const cargoArgs = ['build', '-p', CRATE];
if (profile === 'release') cargoArgs.push('--release');
// Only pass --target for a real cross build: adding it on a host build moves
// cargo's output directory and rebuilds everything for no reason.
if (target !== host) cargoArgs.push('--target', target);
try {
  run('cargo', cargoArgs);
} catch (error) {
  fail(`build failed. ${error.message}`);
}

const builtDir =
  target === host ? join(repoRoot, 'target', profile) : join(repoRoot, 'target', target, profile);
const built = join(builtDir, `${CRATE}${extension}`);
const destination = join(outputDir, `${CRATE}-${target}${extension}`);
try {
  mkdirSync(outputDir, { recursive: true });
  copyFileSync(built, destination);
} catch (error) {
  fail(`could not stage ${built} as ${destination}. ${error.message}`);
}

console.log(`${CRATE}: staged the ${profile} build as ${destination}`);
