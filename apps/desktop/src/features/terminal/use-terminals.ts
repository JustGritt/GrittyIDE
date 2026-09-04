import {
  type CoreError,
  type SpawnTerminalRequest,
  type Terminal,
  type TerminalId,
  type TerminalProfile,
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
export function useTerminals(customShells: readonly string[] = []) {
  const [state, dispatch] = useReducer(terminalsReducer, initialTerminalsState);
  const [error, setError] = useState<CoreError | null>(null);
  const [profiles, setProfiles] = useState<TerminalProfile[]>([]);
  // Both first answers from the core: which sessions already exist, and which
  // shells there are. Until both have landed the UI does not know enough to
  // start a terminal on the user's behalf (see `auto-start.ts`).
  const [adopted, setAdopted] = useState(false);
  const [profiled, setProfiled] = useState(false);
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
        if (active) dispatch({ type: 'workspace', workspaceId: workspace?.id ?? null });
      },
      (cause: unknown) => {
        if (active) setError(toCoreError(cause));
      },
    );
    // After a renderer reload the core still holds the sessions; adopt them
    // rather than leaving orphaned shells running invisibly. Not conditional
    // on a workspace: a session does not need one.
    core.terminal.list().then(
      (terminals) => {
        if (!active) return;
        dispatch({ type: 'rehydrate', terminals });
        setAdopted(true);
      },
      (cause: unknown) => {
        if (!active) return;
        setError(toCoreError(cause));
        // Settled, not successful. A core that cannot list its sessions will
        // not spawn one either; leaving the UI waiting forever is worse than
        // letting the attempt fail visibly.
        setAdopted(true);
      },
    );
    return () => {
      active = false;
      unsubscribe();
    };
  }, []);

  // Refetched whenever the configured shells change, so adding one in the
  // settings puts it in the picker without a restart. Keyed on the serialized
  // list rather than the array, which is a fresh object on every render;
  // JSON round-trips a path exactly, including one containing a newline.
  const customKey = JSON.stringify(customShells);
  useEffect(() => {
    let active = true;
    const custom = JSON.parse(customKey) as string[];
    // Which shells exist does not depend on a workspace, and a failure here
    // must not stop terminals working: the picker just shows the default.
    core.terminal.profiles({ custom }).then(
      (found) => {
        if (!active) return;
        setProfiles(found);
        setProfiled(true);
      },
      (cause: unknown) => {
        if (!active) return;
        setError(toCoreError(cause));
        // As above: the picker falls back to the core's own default shell.
        setProfiled(true);
      },
    );
    return () => {
      active = false;
    };
  }, [customKey]);

  /**
   * Start a shell. `shell` is an absolute path from the profile picker;
   * omitting it lets the core resolve the user's login shell (T090). Without a
   * `workspaceId` the core starts it in the home directory (T095) — the
   * renderer never names a directory outside a workspace.
   */
  const spawn = useCallback(async (workspaceId: WorkspaceId | null, shell?: string) => {
    try {
      const request: SpawnTerminalRequest = {};
      if (workspaceId !== null) request.workspaceId = workspaceId;
      if (shell !== undefined) request.shell = shell;
      const terminal = await core.terminal.spawn(request);
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
    profiles,
    /** The core has answered both opening questions; see `auto-start.ts`. */
    ready: adopted && profiled,
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
