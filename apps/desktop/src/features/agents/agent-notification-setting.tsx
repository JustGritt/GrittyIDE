import { type AgentNotificationSettings, type CoreError, toCoreError } from '@gritty/protocol';
import { useCallback, useEffect, useState } from 'react';
import { ErrorEnvelope } from '../../components/error-envelope';
import { core } from '../../core/client';

/**
 * The attention notification setting (T223 §2).
 *
 * **Off by default, every run**, and this is the only control that turns it
 * on — which matters, because turning it on is the only thing in Gritty that
 * can produce a notification permission prompt. Nothing about launching the
 * application arms it.
 *
 * Reading the setting never prompts, so this can read on mount: it mounts with
 * the island's sheet, and the answer is what the switch shows. A build that
 * cannot raise notifications reports `supported: false` and this renders
 * nothing at all — the setting is absent rather than a switch that would
 * silently do nothing (rule 11).
 *
 * What it buys is one thing: being told a session went to `BLOCKED` or
 * `WAITING` while you were looking somewhere else. The notification is raised
 * by the core, edge-triggered on the transition, throttled per session, capped
 * globally, and silent while this window has focus — because when it does, the
 * island is the notification.
 */
export function AgentNotificationSetting() {
  const [settings, setSettings] = useState<AgentNotificationSettings | null>(null);
  const [error, setError] = useState<CoreError | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let active = true;
    core.agents.notifications().then(
      (answer) => {
        if (active) setSettings(answer);
      },
      (cause: unknown) => {
        if (active) setError(toCoreError(cause));
      },
    );
    return () => {
      active = false;
    };
  }, []);

  const toggle = useCallback(async (enabled: boolean) => {
    setBusy(true);
    try {
      // Nothing optimistic (T210): the switch follows the platform's answer,
      // so a refusal leaves it visibly off rather than briefly on.
      setSettings(await core.agents.setNotifications({ enabled }));
      setError(null);
    } catch (cause: unknown) {
      setError(toCoreError(cause));
    } finally {
      setBusy(false);
    }
  }, []);

  // Not yet read, or this build cannot raise one. Either way there is nothing
  // honest to draw.
  if (settings !== null && !settings.supported) return null;

  return (
    <div className="agents__notifications">
      <label className="agents__notifications-row">
        <input
          type="checkbox"
          checked={settings?.enabled ?? false}
          disabled={settings === null || busy}
          onChange={(event) => void toggle(event.target.checked)}
        />
        <span>Notify me when an agent needs me</span>
      </label>
      <p className="agents__hint">
        A system notification when a session stops at a permission prompt or finishes its turn.
        Never while this window has focus — the island is already saying so — at most one per
        session at a time, and never one per poll.
      </p>
      {settings?.permission === 'DENIED' && (
        <p className="agents__note">
          The system refused permission, so nothing can be shown. Grant it to Gritty in your
          notification settings; Gritty will not ask again on its own.
        </p>
      )}
      {error && <ErrorEnvelope error={error} onDismiss={() => setError(null)} />}
    </div>
  );
}
