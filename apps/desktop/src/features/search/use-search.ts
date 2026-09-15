/**
 * The search controller (T080-U): the side effects the reducer cannot have.
 *
 * Owned by the shell, not by the panel, for the reason `ExplorerController` is:
 * the sidebar mounts only the selected section, so a controller inside the
 * panel would lose the query and the results every time the user looked at the
 * tree, and would abandon a search that is still streaming. Out here the panel
 * is a view of it, and switching tabs costs nothing.
 *
 * It owns the field text, the four options, the one run in flight and the
 * page of results the panel draws. What any of that *means* for the screen is
 * `search-model.ts`'s.
 */

import type {
  EventEnvelope,
  OperationId,
  SearchOptions,
  SearchScope,
  WorkspaceId,
} from '@gritty/protocol';
import { toCoreError } from '@gritty/protocol';
import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import { core } from '../../core/client';
import { SEARCH_PAGE } from './search-model';
import { initialSearchState, isActive, type SearchRun, searchReducer } from './search-reducer';

/**
 * How long the field waits before searching.
 *
 * Longer than quick-open's 120 ms: a content search reads every file rather
 * than ranking names, and a query per keystroke would start and cancel a whole
 * walk per character. Enter skips the wait.
 */
export const SEARCH_DEBOUNCE_MS = 250;

/** The options the panel's toggles set. `includeIgnored` is IGNORE-UI's. */
export interface SearchToggles {
  readonly caseSensitive: boolean;
  readonly wholeWord: boolean;
  readonly useRegex: boolean;
  readonly scope: SearchScope;
}

export const DEFAULT_TOGGLES: SearchToggles = {
  caseSensitive: false,
  wholeWord: false,
  useRegex: false,
  scope: 'both',
};

export interface SearchController {
  readonly workspaceId: WorkspaceId | null;
  readonly query: string;
  readonly toggles: SearchToggles;
  /** `null` before the first search, and after the field is emptied. */
  readonly run: SearchRun | null;
  /** How many results the panel draws; grows by a page on "Show more". */
  readonly limit: number;
  /** Files whose lines are folded away. */
  readonly collapsed: ReadonlySet<string>;
  /** Typing: the search follows after the debounce. */
  readonly setQuery: (query: string) => void;
  /** A toggle: the search re-runs at once. */
  readonly setToggle: <K extends keyof SearchToggles>(key: K, value: SearchToggles[K]) => void;
  /** Enter: search now, even for the query already shown — the tree may have changed. */
  readonly submit: () => void;
  readonly stop: () => void;
  readonly showMore: () => void;
  readonly toggleFile: (path: string) => void;
}

export function useSearch(workspaceId: WorkspaceId | null): SearchController {
  const [state, dispatch] = useReducer(searchReducer, initialSearchState);
  const [query, setQueryText] = useState('');
  const [toggles, setToggles] = useState<SearchToggles>(DEFAULT_TOGGLES);
  const [limit, setLimit] = useState(SEARCH_PAGE);
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(() => new Set());

  // The token of the newest run. Anything answering to an older one was
  // superseded and is cancelled rather than shown.
  const sequence = useRef(0);
  // The run the core may still be talking about, known synchronously. The
  // reducer's copy arrives a render later, and an event, a response or a Stop
  // can all land in between — so the listener, `stop` and `abandon` read this
  // rather than `state.run`.
  const live = useRef<{ token: number; id: OperationId | null } | null>(null);
  // Stop pressed while the token's response was still in flight.
  const stopWhenStarted = useRef<number | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const workspaceRef = useRef(workspaceId);
  workspaceRef.current = workspaceId;

  // Only the events a run can use, and only while one may be streaming: a
  // terminal printing output emits none of these, and a progress report for
  // someone else's listing is dropped before it costs a render.
  useEffect(() => {
    const forward = (event: EventEnvelope) => {
      if (live.current !== null) dispatch({ type: 'event', event });
    };
    const unsubscribes = [
      core.subscribe('SearchMatchesFound', forward),
      core.subscribe('OperationProgress', forward),
      core.subscribe('OperationCompleted', forward),
      core.subscribe('OperationFailed', forward),
      core.subscribe('OperationCancelled', forward),
    ];
    return () => {
      for (const unsubscribe of unsubscribes) unsubscribe();
    };
  }, []);

  // The run ended: stop forwarding. Guarded by the token, because a new run may
  // already be live while the render still shows the old one finishing.
  useEffect(() => {
    const run = state.run;
    if (run !== null && run.token === live.current?.token && !isActive(run)) live.current = null;
  }, [state.run]);

  const cancelPending = useCallback(() => {
    if (timer.current !== null) {
      clearTimeout(timer.current);
      timer.current = null;
    }
  }, []);

  /** Supersede whatever is running: no answer to it will be shown. */
  const abandon = useCallback(() => {
    sequence.current += 1;
    stopWhenStarted.current = null;
    const current = live.current;
    live.current = null;
    // Without an id the response is still in flight; it arrives under a stale
    // token and is cancelled there.
    if (current?.id != null) cancel(current.id);
  }, []);

  const execute = useCallback(
    (text: string, options: SearchToggles) => {
      cancelPending();
      abandon();
      const target = workspaceRef.current;
      if (text.trim() === '' || target === null) {
        dispatch({ type: 'clear' });
        return;
      }
      const token = sequence.current;
      const request: SearchOptions = { ...options };
      live.current = { token, id: null };
      setLimit(SEARCH_PAGE);
      setCollapsed(new Set());
      dispatch({ type: 'start', token, query: text, options: request });
      core.search.query({ workspaceId: target, query: text, options: request }).then(
        (operation) => {
          if (token !== sequence.current) {
            cancel(operation.id);
            return;
          }
          if (live.current?.token === token) live.current = { token, id: operation.id };
          dispatch({ type: 'started', token, operationId: operation.id });
          if (stopWhenStarted.current === token) {
            stopWhenStarted.current = null;
            cancel(operation.id);
          }
        },
        (error: unknown) => {
          if (token !== sequence.current) return;
          dispatch({ type: 'rejected', token, error: toCoreError(error) });
        },
      );
    },
    [abandon, cancelPending],
  );

  // Refs to the latest text and options, so a debounced call searches for what
  // the field says when it fires rather than what it said when it was set.
  const queryRef = useRef(query);
  queryRef.current = query;
  const togglesRef = useRef(toggles);
  togglesRef.current = toggles;

  const setQuery = useCallback(
    (next: string) => {
      setQueryText(next);
      queryRef.current = next;
      cancelPending();
      if (next.trim() === '') {
        // Emptying the field is an answer at once, not after a pause: the old
        // results do not describe an empty query.
        execute(next, togglesRef.current);
        return;
      }
      timer.current = setTimeout(() => {
        timer.current = null;
        execute(queryRef.current, togglesRef.current);
      }, SEARCH_DEBOUNCE_MS);
    },
    [cancelPending, execute],
  );

  const setToggle = useCallback(
    <K extends keyof SearchToggles>(key: K, value: SearchToggles[K]) => {
      const next = { ...togglesRef.current, [key]: value };
      setToggles(next);
      togglesRef.current = next;
      if (queryRef.current.trim() !== '') execute(queryRef.current, next);
    },
    [execute],
  );

  const submit = useCallback(() => {
    execute(queryRef.current, togglesRef.current);
  }, [execute]);

  const stop = useCallback(() => {
    const current = live.current;
    if (current === null) return;
    cancelPending();
    dispatch({ type: 'stop' });
    if (current.id !== null) cancel(current.id);
    else stopWhenStarted.current = current.token;
  }, [cancelPending]);

  const showMore = useCallback(() => setLimit((current) => current + SEARCH_PAGE), []);

  const toggleFile = useCallback((path: string) => {
    setCollapsed((current) => {
      const next = new Set(current);
      if (!next.delete(path)) next.add(path);
      return next;
    });
  }, []);

  // A different workspace, or none: results from the old tree answer nothing
  // here. The text stays — it is the user's, and Enter re-runs it.
  const previousWorkspace = useRef(workspaceId);
  useEffect(() => {
    if (previousWorkspace.current === workspaceId) return;
    previousWorkspace.current = workspaceId;
    cancelPending();
    abandon();
    dispatch({ type: 'clear' });
  }, [workspaceId, abandon, cancelPending]);

  // Unmounting abandons the run: nobody is left to show its results.
  useEffect(
    () => () => {
      cancelPending();
      abandon();
    },
    [abandon, cancelPending],
  );

  return useMemo(
    () => ({
      workspaceId,
      query,
      toggles,
      run: state.run,
      limit,
      collapsed,
      setQuery,
      setToggle,
      submit,
      stop,
      showMore,
      toggleFile,
    }),
    [
      workspaceId,
      query,
      toggles,
      state.run,
      limit,
      collapsed,
      setQuery,
      setToggle,
      submit,
      stop,
      showMore,
      toggleFile,
    ],
  );
}

/** Cancellation is best-effort: an operation that already ended refuses it, and that is fine. */
function cancel(id: OperationId): void {
  void core.operations.cancel(id).catch(() => undefined);
}
