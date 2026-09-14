import { core } from '../../core/client';
import { type ThemeSignal, themeChoiceOf, themeSignal } from './theme-signal';

/**
 * `workbench.theme`, pushed into the signal (T120-T, ADR-031).
 *
 * Kept out of `theme-signal.ts` on purpose: the signal is a DOM object with no
 * idea where a choice comes from, which is what lets it be tested against a
 * fake root and a fake media query. This is the half that talks to the core,
 * and it is the whole of the coupling between the theme and settings.
 */

/**
 * Read the theme once and follow it.
 *
 * Subscribed before the read, so a change landing between the two is not lost.
 * A refusal is *not* an envelope: there is no surface to render one on at this
 * point, and the honest fallback is the behaviour that predates this ticket —
 * `system`, which the signal already holds. A theme must never fail closed to
 * an unreadable window.
 */
export function followThemeSetting(
  signal: ThemeSignal = themeSignal,
  client: Pick<typeof core, 'settings' | 'subscribe'> = core,
): () => void {
  const unsubscribe = client.subscribe('SettingsChanged', (event) => {
    // `changed` rides alongside the snapshot precisely so a consumer can ignore
    // an event that does not concern it, and this one cares about one key.
    if (!event.payload.changed.includes('workbench.theme')) return;
    signal.setChoice(themeChoiceOf(event.payload.snapshot.values['workbench.theme']));
  });
  client.settings.get().then(
    (snapshot) => signal.setChoice(themeChoiceOf(snapshot.values['workbench.theme'])),
    () => {
      // Left on `system`. See above.
    },
  );
  return unsubscribe;
}

let started = false;

/**
 * The application's theme signal, following the setting.
 *
 * Idempotent, and called by each surface that needs the theme rather than wired
 * once in `App.tsx`: the two consumers — the Monaco engine's lifetime and a
 * live xterm's palette effect — are not React state and are constructed from
 * places that never see the app's providers. Starting the follow here keeps
 * the coupling to exactly those two call sites.
 */
export function themeFollowingSettings(): ThemeSignal {
  if (!started) {
    started = true;
    followThemeSetting();
  }
  return themeSignal;
}
