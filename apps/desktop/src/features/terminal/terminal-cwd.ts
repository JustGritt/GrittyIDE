/**
 * Where a new terminal starts (T098).
 *
 * The workspace root is the wrong answer often enough to be annoying: someone
 * editing `crates/terminal/src/system.rs` who opens a shell wants to be in that
 * directory, not to retype the path that is already on screen. So a new
 * terminal follows the document being edited, and falls back to the root when
 * nothing is open.
 *
 * The core still resolves the directory — this only ever produces a
 * *workspace-relative* path for `SpawnTerminalRequest.cwd`, which the core
 * checks against the workspace root before anything is allocated (ADR-010).
 */

/**
 * The folder holding a workspace-relative document path.
 *
 * `''` is the workspace root, which is both what a top-level file resolves to
 * and what the core reads as "the root" — so a `README.md` tab roots a shell
 * exactly where it does today. `undefined` means there is nothing to follow.
 */
export function cwdForDocument(path: string | null | undefined): string | undefined {
  if (path === null || path === undefined) return undefined;
  // `WorkspacePath` is always '/'-separated with no leading or trailing slash,
  // so the last separator is the only thing that has to be found.
  const cut = path.lastIndexOf('/');
  return cut === -1 ? '' : path.slice(0, cut);
}
