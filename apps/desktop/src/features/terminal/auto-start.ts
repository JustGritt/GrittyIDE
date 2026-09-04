/**
 * Whether opening the terminal dock should start a shell.
 *
 * The panel's job is to show a terminal, so arriving at an empty one and
 * having to ask for a shell is a step that exists only because the code
 * needed it. The conditions are all about *not* being annoying:
 *
 * - `ready` — the core has been asked what sessions it already has and which
 *   shells exist. Starting before that answers arrives would duplicate a
 *   session that survived a renderer reload, or start the wrong shell because
 *   the user's chosen profile had not loaded yet.
 * - `sessions` — never a second one. Opening a dock that already has tabs
 *   changes nothing.
 * - `startedForThisOpening` — once per opening, so closing the last tab leaves
 *   the dock empty instead of immediately growing another one, and a shell
 *   that fails to start is not retried in a loop.
 */
export function shouldStartFirstTerminal(state: {
  readonly open: boolean;
  readonly ready: boolean;
  readonly sessions: number;
  readonly startedForThisOpening: boolean;
}): boolean {
  if (!state.open || !state.ready) return false;
  if (state.startedForThisOpening) return false;
  return state.sessions === 0;
}
