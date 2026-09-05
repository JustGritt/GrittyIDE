import type { LastSession, Workspace } from '@gritty/protocol';
import { useCallback, useEffect, useState } from 'react';
import { core } from '../../core/client';

export interface LastSessionController {
  /** What the previous window had open, or `null` when there is nothing to offer. */
  readonly value: LastSession | null;
  /** Hide the rail for the rest of this run. Nothing persisted is deleted. */
  dismiss(): void;
}

/**
 * The last session behind the launcher's resume rail (T051, artboard 3b).
 *
 * Asked for whenever no workspace is open — which is exactly when the launcher
 * is on screen. That covers all three ways of getting there: the last root was
 * missing at startup, persistence was unavailable, or the user closed the
 * workspace during this run and the startup restore will never run again.
 *
 * The core answers `null` unless there is something the recents list does not
 * already say, so this hook never has to decide whether the rail is worth
 * showing: if there is a value, it is worth showing.
 *
 * A failure is not surfaced. The rail is an offer, not a feature: if the core
 * cannot answer, the launcher is exactly as usable without it, and the session
 * notices already explain a persistence outage.
 */
export function useLastSession(workspace: Workspace | null): LastSessionController {
  const [value, setValue] = useState<LastSession | null>(null);
  const [dismissed, setDismissed] = useState(false);

  useEffect(() => {
    if (workspace) {
      // Opening one ends the offer, and re-arms it for the next close.
      setValue(null);
      setDismissed(false);
      return;
    }
    let active = true;
    core.session.lastSession().then(
      (last) => {
        if (active) setValue(last);
      },
      (cause: unknown) => {
        if (active) console.warn('[gritty launcher] no last session to offer', cause);
      },
    );
    return () => {
      active = false;
    };
  }, [workspace]);

  return {
    value: dismissed ? null : value,
    dismiss: useCallback(() => setDismissed(true), []),
  };
}
