import type { TerminalProfile } from '@gritty/protocol';

/**
 * Which shell a new terminal should start (T095).
 *
 * The user's choice is remembered as a path, and a path can go stale: a shell
 * gets uninstalled, Homebrew moves a prefix, the session file comes from
 * another machine. Rather than spawning something that no longer exists and
 * showing an error the user cannot act on, the picker falls back to the
 * machine's default — the behaviour they had before choosing anything.
 *
 * Returns `null` when nothing is known, which `terminal.spawn` reads as "the
 * login shell": the core resolves the default, not the UI.
 */
export function selectedProfile(
  profiles: readonly TerminalProfile[],
  remembered: string | null,
): TerminalProfile | null {
  const chosen = remembered ? profiles.find((profile) => profile.path === remembered) : undefined;
  // An unavailable profile is still shown in the settings, so the user can see
  // why it broke, but it must never be what a new terminal tries to start.
  if (chosen?.available) return chosen;
  return profiles.find((profile) => profile.isDefault) ?? null;
}

/**
 * The `shell` argument for `terminal.spawn`. The default profile is sent as
 * "no shell" so the core applies its own resolution order (`$SHELL`, then the
 * platform default, then `/bin/sh`) instead of the UI pinning a path that was
 * correct when the list was fetched.
 */
export function spawnShell(profile: TerminalProfile | null): string | undefined {
  if (!profile || profile.isDefault) return undefined;
  return profile.path;
}

/**
 * Add a shell the user typed, or explain why it cannot be added.
 *
 * The core does the real check — it is the only side that can stat the file —
 * but the two mistakes a text field invites (a relative path, one already in
 * the list) are worth answering immediately rather than after a round trip.
 */
export function addCustomShell(
  existing: readonly string[],
  raw: string,
  limit: number,
): { shells: readonly string[] } | { error: string } {
  const path = raw.trim();
  if (path === '') return { error: 'Enter the absolute path of a shell.' };
  if (!path.startsWith('/')) return { error: 'Use an absolute path, starting with “/”.' };
  if (existing.includes(path)) return { error: 'That shell is already in the list.' };
  if (existing.length >= limit) return { error: `At most ${limit} custom shells.` };
  return { shells: [...existing, path] };
}
