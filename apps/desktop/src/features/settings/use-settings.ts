import {
  type AgentNotificationSettings,
  type CoreError,
  type JsonValue,
  type SettingsSchema,
  type SettingsSnapshot,
  toCoreError,
} from '@gritty/protocol';
import { useCallback, useEffect, useRef, useState } from 'react';
import { core } from '../../core/client';
import { planSeed, type SeedSource } from './settings-seed';

export interface SettingsController {
  /** The registry, fetched once: it is a build constant (ADR-031 §3). */
  readonly schema: SettingsSchema | null;
  /** What is in force. Replaced by every answer and every `SettingsChanged`. */
  readonly snapshot: SettingsSnapshot | null;
  /** False until both have landed, so "no settings" and "not yet" differ. */
  readonly loaded: boolean;
  /** The last refusal, until it is dismissed or a write succeeds. */
  readonly error: CoreError | null;
  clearError(): void;
  /** True while a write is in flight; controls disable rather than queue. */
  readonly busy: boolean;
  /**
   * Write one setting. Resolves `true` when the core accepted it.
   *
   * The answer matters rather than the snapshot: a control holding a draft has
   * to know to put the stored value back, and a fully controlled input that
   * silently refuses to update cannot be typed into.
   */
  set(key: string, value: JsonValue): Promise<boolean>;
  /** Remove keys from the file so their defaults apply again. */
  reset(keys?: readonly string[]): Promise<boolean>;
  /** Show `settings.json` in the system file manager. */
  reveal(): Promise<void>;
  /**
   * The platform's answer for `agents.notifications`, or `null` until it lands.
   */
  readonly notifications: AgentNotificationSettings | null;
  /**
   * Turn attention notifications on or off: the platform first, then the file.
   */
  setNotifications(enabled: boolean): Promise<boolean>;
}

/**
 * The settings surface's one connection to the core (T120-U, ADR-031).
 *
 * `describe` is fetched once and `get` once; from then on the snapshot is
 * whatever the last answer or `SettingsChanged` carried. Nothing here polls and
 * nothing watches a file: ADR-031 §6 re-reads on window-focus gain in the core
 * and emits the event, precisely so the renderer does not have to.
 *
 * **Nothing optimistic.** Every write converges on the snapshot the core
 * answered with, so a refused value is never briefly on screen as though it
 * had taken. That is also what makes the number input's draft the only local
 * state on the page.
 *
 * Constructed in `App.tsx` rather than inside the page (T120-M), for two
 * reasons: the seeding has to run whether or not anyone opens settings, and a
 * tab that is closed and reopened must not re-fetch a build constant.
 */
export function useSettings(seed: SeedSource | null = null): SettingsController {
  const [schema, setSchema] = useState<SettingsSchema | null>(null);
  const [snapshot, setSnapshot] = useState<SettingsSnapshot | null>(null);
  const [notifications, setNotificationState] = useState<AgentNotificationSettings | null>(null);
  const [error, setError] = useState<CoreError | null>(null);
  const [inFlight, setInFlight] = useState(0);

  useEffect(() => {
    let active = true;
    // Subscribed before anything is fetched, so a change landing between the
    // two is not lost. A later event always wins over the initial `get`
    // because the fetch only ever sets state it started before.
    const unsubscribe = core.subscribe('SettingsChanged', (event) => {
      if (active) setSnapshot(event.payload.snapshot);
    });
    Promise.all([core.settings.describe(), core.settings.get()]).then(
      ([described, current]) => {
        if (!active) return;
        setSchema(described);
        setSnapshot((held) => held ?? current);
      },
      (cause: unknown) => {
        if (active) setError(toCoreError(cause));
      },
    );
    return () => {
      active = false;
      unsubscribe();
    };
  }, []);

  useEffect(() => {
    let active = true;
    // Reading the platform's answer never prompts — `notification_settings`
    // calls `permission()`, not `request_permission()` — so opening the app is
    // safe. Only the control the user touches may ask (T223 §2).
    core.agents.notifications().then(
      (answer) => {
        if (active) setNotificationState(answer);
      },
      (cause: unknown) => {
        if (active) setError(toCoreError(cause));
      },
    );
    return () => {
      active = false;
    };
  }, []);

  const run = useCallback(async <T>(work: () => Promise<T>): Promise<T | null> => {
    setInFlight((count) => count + 1);
    try {
      const answer = await work();
      setError(null);
      return answer;
    } catch (cause: unknown) {
      setError(toCoreError(cause));
      return null;
    } finally {
      setInFlight((count) => count - 1);
    }
  }, []);

  const set = useCallback(
    async (key: string, value: JsonValue) => {
      const answer = await run(() => core.settings.set({ values: { [key]: value } }));
      if (answer === null) return false;
      setSnapshot(answer);
      return true;
    },
    [run],
  );

  const reset = useCallback(
    async (keys?: readonly string[]) => {
      const answer = await run(() =>
        // `undefined` resets every known key, which is what "reset all" means.
        core.settings.reset(keys === undefined ? {} : { keys: [...keys] }),
      );
      if (answer === null) return false;
      setSnapshot(answer);
      return true;
    },
    [run],
  );

  const reveal = useCallback(async () => {
    await run(() => core.settings.reveal());
  }, [run]);

  const setNotifications = useCallback(
    async (enabled: boolean) => {
      // The platform first. This is the one call in Gritty that can raise a
      // notification permission prompt, and it must be reachable only from the
      // control the user touched.
      const answer = await run(() => core.agents.setNotifications({ enabled }));
      if (answer === null) return false;
      setNotificationState(answer);
      // Then the file, carrying what the platform actually did rather than what
      // was asked. Persisting `true` after a refusal would leave a modified
      // mark on a setting that is not in force, and would arm nothing at the
      // next launch either — `arm_notifications` re-reads the permission.
      return await set('agents.notifications', answer.enabled);
    },
    [run, set],
  );

  // The one-time seeding (ADR-031 §8). Attempted once per session: a failure
  // leaves the blobs intact and is safe to retry on the next launch, but
  // retrying inside one session would just fail the same way and bury the
  // user's own refusal under it.
  const seeded = useRef(false);
  useEffect(() => {
    if (seeded.current || seed === null || schema === null || snapshot === null) return;
    // A write is refused outright while storage is unwritable, and a seed is
    // not worth reporting that with: the page already says why, in words that
    // say what to do.
    if (!snapshot.storage.writable) return;
    seeded.current = true;
    const patch = planSeed(schema, snapshot, seed);
    if (Object.keys(patch).length === 0) return;
    // One `set`, so it is one atomic file write: the seeding lands whole or
    // not at all.
    core.settings.set({ values: patch }).then(
      (answer) => setSnapshot(answer),
      (cause: unknown) => setError(toCoreError(cause)),
    );
  }, [seed, schema, snapshot]);

  return {
    schema,
    snapshot,
    loaded: schema !== null && snapshot !== null,
    error,
    clearError: useCallback(() => setError(null), []),
    busy: inFlight > 0,
    set,
    reset,
    reveal,
    notifications,
    setNotifications,
  };
}
