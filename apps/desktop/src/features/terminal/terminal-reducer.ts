import type { EventEnvelope, Terminal, TerminalId, WorkspaceId } from '@gritty/protocol';

/**
 * Terminal sessions as the UI knows them. Lifecycle only: the bytes a shell
 * prints never reach React state (see `output-bus.ts`). Pure, so it is
 * unit-tested without React or a PTY.
 */
export interface TerminalsState {
  readonly byId: Readonly<Record<string, Terminal>>;
  readonly order: readonly TerminalId[];
  readonly activeId: TerminalId | null;
  /**
   * The open workspace, which is where a new terminal is rooted. Sessions are
   * *not* dropped when it changes: a shell the core is still running is one
   * the user must be able to see and kill (see the `workspace` case).
   */
  readonly workspaceId: WorkspaceId | null;
}

export const initialTerminalsState: TerminalsState = {
  byId: {},
  order: [],
  activeId: null,
  workspaceId: null,
};

export type TerminalsAction =
  | { type: 'workspace'; workspaceId: WorkspaceId | null }
  | { type: 'rehydrate'; terminals: Terminal[] }
  | { type: 'opened'; terminal: Terminal }
  | { type: 'snapshot'; terminal: Terminal }
  | { type: 'closed'; terminalId: TerminalId }
  | { type: 'activate'; terminalId: TerminalId }
  | { type: 'event'; event: EventEnvelope };

function put(state: TerminalsState, terminal: Terminal): TerminalsState {
  const known = state.byId[terminal.id] !== undefined;
  return {
    ...state,
    byId: { ...state.byId, [terminal.id]: terminal },
    order: known ? state.order : [...state.order, terminal.id],
  };
}

function remove(state: TerminalsState, terminalId: TerminalId): TerminalsState {
  if (state.byId[terminalId] === undefined) return state;
  const { [terminalId]: _removed, ...byId } = state.byId;
  const order = state.order.filter((id) => id !== terminalId);
  const activeId =
    state.activeId === terminalId ? (order[order.length - 1] ?? null) : state.activeId;
  return { ...state, byId, order, activeId };
}

export function terminalsReducer(state: TerminalsState, action: TerminalsAction): TerminalsState {
  switch (action.type) {
    case 'workspace': {
      if (action.workspaceId === state.workspaceId) return state;
      // Only the rooting for *new* terminals changes. Forgetting the running
      // ones would not stop them: the core keeps every session until it is
      // killed or the application exits, so a UI that hides them leaves the
      // user with shells they can neither see nor terminate.
      return { ...state, workspaceId: action.workspaceId };
    }
    case 'rehydrate': {
      const next = action.terminals.reduce(put, {
        ...state,
        byId: {},
        order: [],
      });
      const activeId =
        state.activeId && next.byId[state.activeId]
          ? state.activeId
          : (next.order[next.order.length - 1] ?? null);
      return { ...next, activeId };
    }
    case 'opened': {
      const next = put(state, action.terminal);
      return { ...next, activeId: action.terminal.id };
    }
    case 'snapshot':
      // A snapshot for a session the UI has already closed must not resurrect it.
      return state.byId[action.terminal.id] === undefined ? state : put(state, action.terminal);
    case 'closed':
      return remove(state, action.terminalId);
    case 'activate':
      return state.byId[action.terminalId] === undefined
        ? state
        : { ...state, activeId: action.terminalId };
    case 'event': {
      const { event } = action;
      switch (event.name) {
        case 'TerminalExited':
          return state.byId[event.payload.terminal.id] === undefined
            ? state
            : put(state, event.payload.terminal);
        case 'WorkspaceOpened':
          return { ...state, workspaceId: event.payload.workspace.id };
        case 'WorkspaceClosed':
          return { ...state, workspaceId: null };
      }
      return state;
    }
  }
}

/** Sessions in creation order. */
export function selectTerminals(state: TerminalsState): Terminal[] {
  return state.order.map((id) => state.byId[id]).filter((terminal) => terminal !== undefined);
}

export function selectActive(state: TerminalsState): Terminal | null {
  return state.activeId ? (state.byId[state.activeId] ?? null) : null;
}

/** How a tab labels a session: its exit status once it has one. */
export function describeTerminal(terminal: Terminal): string {
  if (terminal.status === 'RUNNING') return 'running';
  if (terminal.exitSignal) return `killed (${terminal.exitSignal})`;
  if (terminal.exitCode === 0) return 'exited';
  return `exited (${terminal.exitCode ?? '?'})`;
}
