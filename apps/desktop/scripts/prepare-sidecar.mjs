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

function fail(message) {
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
