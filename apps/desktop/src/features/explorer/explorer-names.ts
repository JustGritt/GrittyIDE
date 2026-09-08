/**
 * Naming rules for the explorer's inline inputs (T042).
 *
 * This is feedback, not enforcement. The core is the authority on every one of
 * these rules — `WorkspacePath`'s parser rejects the same strings before a
 * handler runs, and `files.create` refuses an existing target — and it will
 * refuse a bad name whatever this file says. What this buys is the input going
 * red as you type instead of after you press Enter.
 *
 * Pure, so the rules are tested without a DOM or a core.
 */

/** Why a name cannot be used, in words meant for the person typing it. */
export type NameProblem = string;

/** Characters no segment may contain, on any platform (ADR-010). */
const FORBIDDEN = ['/', '\\', '\0'];

/**
 * Names Windows refuses whatever the extension. Gritty is macOS-first
 * (ADR-002), so this is a *warning* the caller may show and not a refusal:
 * refusing would make a repository unusable on the platform it targets, and
 * the filesystem here will happily create `aux.ts`.
 */
const WINDOWS_RESERVED = /^(con|prn|aux|nul|com[1-9]|lpt[1-9])(\.|$)/i;

/**
 * Check one path segment — what a rename or a "New Folder" produces.
 * Returns `null` when the name is usable.
 */
export function checkName(name: string): NameProblem | null {
  if (name === '') return 'Name cannot be empty.';
  if (name === '.' || name === '..') return `"${name}" is not a name.`;
  for (const character of FORBIDDEN) {
    if (name.includes(character)) {
      const shown = character === '\0' ? 'a null character' : `"${character}"`;
      return `Name cannot contain ${shown}.`;
    }
  }
  // Leading and trailing spaces are legal on APFS and invisible in the tree,
  // which is exactly why they are worth refusing: two entries that look
  // identical are worse than a rejected keystroke.
  if (name !== name.trim()) return 'Name cannot start or end with a space.';
  if (name.endsWith('.')) return 'Name cannot end with a dot.';
  return null;
}

/**
 * Check what a "New File" input accepts, which is a whole relative path:
 * `a/b/c.ts` creates the directories on the way (`files.create` does this in
 * one call). Every segment is held to {@link checkName}.
 */
export function checkNewPath(raw: string): NameProblem | null {
  if (raw === '') return 'Name cannot be empty.';
  if (raw.startsWith('/')) return 'Enter a path relative to this folder.';
  if (raw.endsWith('/')) return 'Name cannot end with "/".';
  const segments = raw.split('/');
  for (const segment of segments) {
    if (segment === '') return 'Path cannot contain an empty segment.';
    const problem = checkName(segment);
    if (problem) return problem;
  }
  return null;
}

/** A name Windows would refuse, for a warning rather than a refusal. */
export function isWindowsReserved(name: string): boolean {
  return WINDOWS_RESERVED.test(name);
}

/**
 * Split a name at the last dot the way a file manager does: `a.tar.gz` is
 * `a.tar` + `.gz`, and a dotfile like `.gitignore` is all stem — its copy
 * should be `.gitignore copy`, not `. copy gitignore`.
 *
 * The same split the core makes (`files.rs::split_name`); duplicated rather
 * than shared because the two run in different languages, and tested on both
 * sides so they cannot drift silently.
 */
export function splitName(name: string): [stem: string, extension: string] {
  const index = name.lastIndexOf('.');
  if (index <= 0) return [name, ''];
  return [name.slice(0, index), name.slice(index)];
}

/**
 * The name a duplicate should get: `a.ts` → `a copy.ts` → `a copy 2.ts`.
 * `taken` answers whether a candidate is already in use.
 *
 * Used for the optimistic label on "Duplicate"; the core computes the real
 * destination and its answer is the one that lands.
 */
export function copyName(name: string, taken: (candidate: string) => boolean): string {
  if (!taken(name)) return name;
  const [stem, extension] = splitName(name);
  for (let attempt = 1; attempt < 1000; attempt += 1) {
    const suffix = attempt === 1 ? ' copy' : ` copy ${attempt}`;
    const candidate = `${stem}${suffix}${extension}`;
    if (!taken(candidate)) return candidate;
  }
  return `${stem} copy ${Date.now()}${extension}`;
}
