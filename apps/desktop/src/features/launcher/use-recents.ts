import { type CoreError, type RecentWorkspace, toCoreError } from '@gritty/protocol';
import { useCallback, useEffect, useState } from 'react';
import { core } from '../../core/client';

/** Asked for at once: five are shown, the rest are behind "View all". */
const LIMIT = 20;

export interface RecentsController {
  readonly items: readonly RecentWorkspace[];
  readonly loading: boolean;
  /** A failed read, or a failed Forget. Never a persistence outage — that is
   *  an empty history plus a session notice. */
  readonly error: CoreError | null;
  forget(root: string): void;
  reload(): void;
  clearError(): void;
}

/**
 * The workspace history behind the launcher (T049).
 *
 * A snapshot taken when the launcher mounts, not a subscription: the launcher
 * only exists while no workspace is open, and the one thing that changes the
 * history — opening a workspace — unmounts it. `reload` covers the rest (a
 * Forget, or a failed open that proved an entry stale).
 */
export function useRecents(): RecentsController {
  const [items, setItems] = useState<readonly RecentWorkspace[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<CoreError | null>(null);
  const [generation, setGeneration] = useState(0);

  // biome-ignore lint/correctness/useExhaustiveDependencies: the effect does not read `generation`; bumping it is how `reload` asks for a refetch
  useEffect(() => {
    let active = true;
    setLoading(true);
    core.workspace.recents(LIMIT).then(
      (recents) => {
        if (!active) return;
        setItems(recents);
        setLoading(false);
      },
      (cause: unknown) => {
        if (!active) return;
        setError(toCoreError(cause));
        setLoading(false);
      },
    );
    return () => {
      active = false;
    };
  }, [generation]);

  const reload = useCallback(() => setGeneration((n) => n + 1), []);

  const forget = useCallback(
    (root: string) => {
      // Removed from the list first: the user pressed Forget, and a row that
      // lingers until a round trip finishes reads as a button that did nothing.
      // The reload below is what makes the optimism honest.
      setItems((current) => current.filter((item) => item.root !== root));
      core.workspace.forget(root).then(
        () => setError(null),
        (cause: unknown) => {
          setError(toCoreError(cause));
          reload();
        },
      );
    },
    [reload],
  );

  const clearError = useCallback(() => setError(null), []);

  return { items, loading, error, forget, reload, clearError };
}
