import type { Terminal } from '@gritty/protocol';

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
