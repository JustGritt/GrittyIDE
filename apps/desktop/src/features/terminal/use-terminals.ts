import {
  type CoreError,
  type Terminal,
  type TerminalId,
  toCoreError,
  type WorkspaceId,
} from '@gritty/protocol';
import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import { core } from '../../core/client';
import { createOutputBus } from './output-bus';
import {
  initialTerminalsState,
  selectActive,
  selectTerminals,
  terminalsReducer,
} from './terminal-reducer';

/**
 * Terminal sessions for the open workspace.
 *
 * Output does not go through this hook's state: `TerminalOutput` is routed to
 * the output bus, which hands it straight to an xterm instance. Only lifecycle
 * (which sessions exist, which ended) is React state, so a shell printing at
 * full speed re-renders nothing.
 */
export function useTerminals() {
  const [state, dispatch] = useReducer(terminalsReducer, initialTerminalsState);
  const [error, setError] = useState<CoreError | null>(null);
  const bus = useMemo(createOutputBus, []);
  // Read inside the event handler, which is registered once.
  const busRef = useRef(bus);
  busRef.current = bus;

  useEffect(() => {
    // Subscribe before asking for the current workspace so nothing emitted in
    // between is lost.
    const unsubscribe = core.subscribeAll((event) => {
      if (event.name === 'TerminalOutput') {
        busRef.current.deliver(event.payload.terminalId, {
          data: event.payload.data,
          truncatedBytes: event.payload.truncatedBytes,
        });
        return;
      }
      dispatch({ type: 'event', event });
    });
    let active = true;
    core.workspace.current().then(
      (workspace) => {
        if (!active) return;
        dispatch({ type: 'workspace', workspaceId: workspace?.id ?? null });
        // After a renderer reload the core still holds the sessions; adopt
        // them rather than leaving orphaned shells running invisibly.
        if (workspace) {
          core.terminal.list().then(
            (terminals) => {
              if (active) dispatch({ type: 'rehydrate', terminals });
            },
            (cause: unknown) => {
              if (active) setError(toCoreError(cause));
            },
          );
        }
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

  const spawn = useCallback(async (workspaceId: WorkspaceId) => {
    try {
      const terminal = await core.terminal.spawn({ workspaceId });
      dispatch({ type: 'opened', terminal });
      setError(null);
      return terminal;
    } catch (cause: unknown) {
      setError(toCoreError(cause));
      return null;
    }
  }, []);

  /**
   * Close a tab: terminate the process tree, then forget the session. The
   * core keeps an ended session listed so its status can still be read; once
   * the user closes the tab it is the UI that stops showing it.
   */
  const close = useCallback(
    async (terminalId: TerminalId) => {
      try {
        await core.terminal.kill({ terminalId });
      } catch (cause: unknown) {
        setError(toCoreError(cause));
      }
      bus.forget(terminalId);
      dispatch({ type: 'closed', terminalId });
    },
    [bus],
  );

  /** Terminate without closing the tab, so the exit status stays visible. */
  const kill = useCallback(async (terminalId: TerminalId) => {
    try {
      const terminal = await core.terminal.kill({ terminalId });
      dispatch({ type: 'snapshot', terminal });
    } catch (cause: unknown) {
      setError(toCoreError(cause));
    }
  }, []);

  const activate = useCallback((terminalId: TerminalId) => {
    dispatch({ type: 'activate', terminalId });
  }, []);

  const write = useCallback((terminalId: TerminalId, data: string) => {
    // Keystrokes are fire-and-forget: awaiting each one would serialize typing
    // behind the IPC round trip. A failure means the session is gone, which
    // the exit event reports anyway.
    void core.terminal.write({ terminalId, data }).catch(() => {});
  }, []);

  const resize = useCallback((terminalId: TerminalId, cols: number, rows: number) => {
    void core.terminal.resize({ terminalId, cols, rows }).catch(() => {});
  }, []);

  const clearError = useCallback(() => setError(null), []);

  const terminals: Terminal[] = selectTerminals(state);
  return {
    terminals,
    active: selectActive(state),
    workspaceId: state.workspaceId,
    bus,
    error,
    spawn,
    close,
    kill,
    activate,
    write,
    resize,
    clearError,
  };
}

export type TerminalsController = ReturnType<typeof useTerminals>;
