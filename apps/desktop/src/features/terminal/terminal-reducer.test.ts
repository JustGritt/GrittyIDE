import type { EventEnvelope, Terminal, TerminalId, WorkspaceId } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import {
  describeTerminal,
  initialTerminalsState,
  selectActive,
  selectTerminals,
  type TerminalsState,
  terminalsReducer,
} from './terminal-reducer';

const workspaceId = 'ws-1' as WorkspaceId;
const other = 'ws-2' as WorkspaceId;

function terminal(id: string, overrides: Partial<Terminal> = {}): Terminal {
  return {
    id: id as TerminalId,
    workspaceId,
    cwd: '' as Terminal['cwd'],
    shell: '$SHELL',
    pid: 100,
    status: 'RUNNING',
    cols: 80,
    rows: 24,
    createdAt: 1,
    ...overrides,
  };
}

function opened(...ids: string[]): TerminalsState {
  return ids.reduce(
    (state, id) => terminalsReducer(state, { type: 'opened', terminal: terminal(id) }),
    terminalsReducer(initialTerminalsState, { type: 'workspace', workspaceId }),
  );
}

function event(envelope: Partial<EventEnvelope> & { name: string; payload: unknown }) {
  return {
    type: 'event' as const,
    event: { seq: 1, actor: { type: 'SYSTEM' }, ...envelope } as EventEnvelope,
  };
}

describe('terminals reducer', () => {
  it('keeps sessions in creation order and activates the newest', () => {
    const state = opened('a', 'b', 'c');
    expect(selectTerminals(state).map((t) => t.id)).toEqual(['a', 'b', 'c']);
    expect(state.activeId).toBe('c');
  });

  it('activates an existing session and ignores an unknown one', () => {
    const state = opened('a', 'b');
    const activated = terminalsReducer(state, { type: 'activate', terminalId: 'a' as TerminalId });
    expect(activated.activeId).toBe('a');
    expect(
      terminalsReducer(activated, { type: 'activate', terminalId: 'ghost' as TerminalId }),
    ).toBe(activated);
  });

  it('marks a session exited from its event without removing it', () => {
    const state = opened('a');
    const exited = terminalsReducer(
      state,
      event({
        name: 'TerminalExited',
        payload: {
          terminal: terminal('a', { status: 'EXITED', pid: undefined, exitCode: 3 }),
          code: 3,
        },
      }),
    );
    const session = selectTerminals(exited)[0];
    expect(session?.status).toBe('EXITED');
    expect(session?.exitCode).toBe(3);
    expect(exited.order).toEqual(['a']);
    expect(exited.activeId).toBe('a');
  });

  it('ignores an exit for a session the UI already closed', () => {
    const state = opened('a');
    const closed = terminalsReducer(state, { type: 'closed', terminalId: 'a' as TerminalId });
    const after = terminalsReducer(
      closed,
      event({
        name: 'TerminalExited',
        payload: { terminal: terminal('a', { status: 'EXITED' }), code: 0 },
      }),
    );
    expect(selectTerminals(after)).toEqual([]);
  });

  it('closing the active session activates the previous one', () => {
    const state = opened('a', 'b', 'c');
    const closed = terminalsReducer(state, { type: 'closed', terminalId: 'c' as TerminalId });
    expect(closed.activeId).toBe('b');
    expect(selectActive(closed)?.id).toBe('b');

    const empty = ['b', 'a'].reduce(
      (acc, id) => terminalsReducer(acc, { type: 'closed', terminalId: id as TerminalId }),
      closed,
    );
    expect(empty.activeId).toBeNull();
    expect(selectActive(empty)).toBeNull();
  });

  it('a snapshot never resurrects a closed session', () => {
    const state = opened('a');
    const closed = terminalsReducer(state, { type: 'closed', terminalId: 'a' as TerminalId });
    expect(terminalsReducer(closed, { type: 'snapshot', terminal: terminal('a') })).toBe(closed);
  });

  it('rehydrate replaces the list and keeps a still-present active session', () => {
    const state = opened('a', 'b');
    const activated = terminalsReducer(state, { type: 'activate', terminalId: 'a' as TerminalId });
    const rehydrated = terminalsReducer(activated, {
      type: 'rehydrate',
      terminals: [terminal('a'), terminal('z')],
    });
    expect(rehydrated.order).toEqual(['a', 'z']);
    expect(rehydrated.activeId).toBe('a');

    // When the active session is gone, the newest takes over.
    const withoutActive = terminalsReducer(activated, {
      type: 'rehydrate',
      terminals: [terminal('z')],
    });
    expect(withoutActive.activeId).toBe('z');
  });

  it('keeps running sessions when the workspace changes or closes', () => {
    // The core does not kill a session because a workspace closed. Dropping
    // them here would leave the user with shells they can neither see nor
    // terminate until the application exits.
    const state = opened('a', 'b');

    const switched = terminalsReducer(state, {
      type: 'event',
      event: {
        seq: 2,
        actor: { type: 'USER' },
        name: 'WorkspaceOpened',
        payload: { workspace: { id: other, root: '/tmp/other', name: 'other', openedAt: 2 } },
      } as EventEnvelope,
    });
    expect(selectTerminals(switched).map((session) => session.id)).toEqual(['a', 'b']);
    expect(switched.workspaceId).toBe(other);

    const closed = terminalsReducer(
      state,
      event({
        name: 'WorkspaceClosed',
        payload: { workspaceId, root: '/tmp/ws', reason: 'REPLACED' },
      }),
    );
    expect(selectTerminals(closed).map((session) => session.id)).toEqual(['a', 'b']);
    expect(closed.workspaceId).toBeNull();
  });

  it('tracks a session that belongs to no workspace', () => {
    const homeless = terminal('h', { workspaceId: undefined, cwd: undefined });
    const state = terminalsReducer(initialTerminalsState, {
      type: 'opened',
      terminal: homeless,
    });
    expect(selectTerminals(state)).toEqual([homeless]);
    // Opening a workspace afterwards must not disturb it.
    const withWorkspace = terminalsReducer(state, { type: 'workspace', workspaceId });
    expect(selectTerminals(withWorkspace)).toEqual([homeless]);
  });

  it('reopening the same workspace keeps the sessions', () => {
    const state = opened('a');
    const same = terminalsReducer(state, { type: 'workspace', workspaceId });
    expect(same).toBe(state);
  });

  it('describes what happened to a session', () => {
    expect(describeTerminal(terminal('a'))).toBe('running');
    expect(describeTerminal(terminal('a', { status: 'EXITED', exitCode: 0 }))).toBe('exited');
    expect(describeTerminal(terminal('a', { status: 'EXITED', exitCode: 130 }))).toBe(
      'exited (130)',
    );
    expect(describeTerminal(terminal('a', { status: 'EXITED', exitSignal: 'SIGKILL' }))).toBe(
      'killed (SIGKILL)',
    );
  });
});
