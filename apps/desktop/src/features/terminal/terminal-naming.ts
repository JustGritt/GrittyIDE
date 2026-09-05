import type { Terminal, WorkspaceId } from '@gritty/protocol';

/**
 * What a session says about itself: the label it carries (T047), where it is
 * and how it ended (T096, design artboard 8a).
 *
 * All of it is here rather than in the pill component because this app's
 * vitest project runs in `node` and matches `.test.ts` only — `.tsx` is not
 * collected, so logic worth a test has to live in a `.ts` module.
 */

/** Matches the core's `MAX_TERMINAL_NAME_CHARS`. */
export const MAX_TERMINAL_NAME_CHARS = 64;

/** Which shell is running in a session, for a tab that has no name. */
export function shellName(shell: string): string {
  const name = shell.split('/').pop();
  return name && name !== '' ? name : 'shell';
}

/**
 * What a tab says.
 *
 * The shell's own name is the fallback, not the answer: once sessions hold an
 * agent, a dev server and a build, three tabs reading "zsh" identify none of
 * them. A name the user gave wins.
 */
export function terminalLabel(session: Pick<Terminal, 'name' | 'shell'>): string {
  return session.name ?? shellName(session.shell);
}

/**
 * Check a typed name the way the core will, so the field can refuse before a
 * round trip and say the same thing the core would.
 *
 * Blank is not an error: emptying the field means "clear the label", which is
 * what `null` asks the core for.
 */
export function normalizeTerminalName(raw: string): { name: string | null } | { error: string } {
  const trimmed = raw.trim();
  if (trimmed === '') return { name: null };
  // The label is drawn straight into the tab strip, so a control character is
  // refused rather than rendered. Tested by code point rather than by a regex
  // that would have to contain the very characters it rejects; the range is
  // C0, DEL and C1, matching Rust's `char::is_control` on the other side.
  if ([...trimmed].some((character) => isControl(character))) {
    return { error: 'A name cannot contain control characters.' };
  }
  // Spread, not `.length`: the bound counts what is drawn, and an emoji is one
  // character on screen however many UTF-16 units it costs.
  if ([...trimmed].length > MAX_TERMINAL_NAME_CHARS) {
    return { error: `At most ${MAX_TERMINAL_NAME_CHARS} characters.` };
  }
  return { name: trimmed };
}

function isControl(character: string): boolean {
  const code = character.codePointAt(0) ?? 0;
  return code < 0x20 || (code >= 0x7f && code <= 0x9f);
}

/** The open workspace, which is the only one whose name the renderer knows. */
export interface PillWorkspace {
  readonly id: WorkspaceId;
  readonly name: string;
}

/**
 * The dot's colour, which is the first thing read on a pill.
 *
 * `idle` is a shell that ended the way it was asked to; `failed` is one that
 * ended with something to explain. The artboard also draws agent-owned pills in
 * blue and amber — there is no data behind those yet (T220 is read-only and
 * T410 owns the badge), so no variant is invented for them.
 */
export type PillTone = 'running' | 'idle' | 'failed';

export function pillTone(session: Pick<Terminal, 'status' | 'exitCode' | 'exitSignal'>): PillTone {
  if (session.status === 'RUNNING') return 'running';
  if (session.exitSignal) return 'failed';
  return session.exitCode === 0 || session.exitCode === undefined ? 'idle' : 'failed';
}

/**
 * Where the session is, in the few characters a pill can spare.
 *
 * The directory is the answer for the sessions nobody has named. A home shell
 * is `~`; the workspace root is the workspace's own name, which is what the
 * artboard draws; anything deeper is the workspace-relative path.
 *
 * `null` when a session was rooted in a workspace that is no longer the open
 * one: its name is not on the wire, and inventing one would be worse than the
 * tooltip, which still says the session has a workspace root.
 */
export function pillLocation(
  session: Pick<Terminal, 'cwd' | 'workspaceId'>,
  workspace: PillWorkspace | null,
): string | null {
  if (session.cwd === undefined) return '~';
  if (session.cwd !== '') return session.cwd;
  return workspace && workspace.id === session.workspaceId ? workspace.name : null;
}

/**
 * How a session ended, short enough to sit on a pill.
 *
 * `null` while it is running: a pill that says "running" spends its width
 * repeating what the green dot already said.
 */
export function pillStatus(
  session: Pick<Terminal, 'status' | 'exitCode' | 'exitSignal'>,
): string | null {
  if (session.status === 'RUNNING') return null;
  if (session.exitSignal) return `killed ${session.exitSignal}`;
  if (session.exitCode !== undefined && session.exitCode !== 0) return `exit ${session.exitCode}`;
  return 'exited';
}

/** Where the session started, spelled out for its tooltip. */
export function describeCwd(session: Pick<Terminal, 'cwd'>): string {
  if (session.cwd === undefined) return 'your home directory';
  return session.cwd === '' ? 'the workspace root' : session.cwd;
}
