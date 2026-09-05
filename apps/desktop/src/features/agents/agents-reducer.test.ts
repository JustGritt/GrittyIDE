import type { AgentRoster, AgentSession, EventEnvelope } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import {
  agentsReducer,
  initialAgentsState,
  mostUrgent,
  runningCount,
  taskLine,
  toolLine,
} from './agents-reducer';

function session(overrides: Partial<AgentSession> = {}): AgentSession {
  return {
    id: 'claude_code:s-1',
    provider: 'CLAUDE_CODE',
    activity: 'WORKING',
    evidence: 'BOTH',
    inWorkspace: true,
    ...overrides,
  };
}

function roster(sessions: AgentSession[], overrides: Partial<AgentRoster> = {}): AgentRoster {
  return {
    sessions,
    needsYou: sessions.filter((entry) => entry.activity === 'WAITING').length,
    processEvidence: true,
    ...overrides,
  };
}

function envelope(name: string, payload: unknown): EventEnvelope {
  return { name, payload, seq: 1, actor: { type: 'SYSTEM' } } as EventEnvelope;
}

describe('agents reducer', () => {
  it('starts empty but not loaded, so "none" and "not yet" differ', () => {
    expect(initialAgentsState.loaded).toBe(false);
    expect(initialAgentsState.roster.sessions).toEqual([]);
  });

  it('replaces the roster wholesale from an event', () => {
    const first = agentsReducer(initialAgentsState, {
      type: 'loaded',
      roster: roster([session()]),
    });
    expect(first.loaded).toBe(true);

    const next = agentsReducer(
      first,
      // The session ended and a different one started: nothing is merged.
      { type: 'event', event: envelope('AgentSessionsChanged', roster([session({ id: 'b' })])) },
    );
    expect(next.roster.sessions.map((entry) => entry.id)).toEqual(['b']);
  });

  it('ignores events from other domains', () => {
    const state = agentsReducer(initialAgentsState, {
      type: 'loaded',
      roster: roster([session()]),
    });
    const after = agentsReducer(state, {
      type: 'event',
      event: envelope('TerminalOutput', { terminalId: 't-1' }),
    });
    expect(after).toBe(state);
  });

  it('counts what is alive', () => {
    const here = session({ id: 'here', inWorkspace: true });
    const away = session({ id: 'away', inWorkspace: false });
    const gone = session({ id: 'gone', inWorkspace: false, activity: 'ENDED' });

    expect(runningCount(roster([here, away, gone]))).toBe(2);
  });

  it('picks the session whose move is yours as the one the island shows', () => {
    const working = session({ id: 'working', activity: 'WORKING' });
    const waiting = session({ id: 'waiting', activity: 'WAITING' });
    // The core's ranking is respected for everything else: the first row is
    // the most relevant one it produced.
    expect(mostUrgent(roster([working, waiting]))?.id).toBe('waiting');
    expect(mostUrgent(roster([working]))?.id).toBe('working');
    expect(mostUrgent(roster([]))).toBeNull();
  });

  it('prefers the agent title, then the prompt, then the reply', () => {
    expect(taskLine(session({ title: 'A', lastPrompt: 'B', lastReply: 'C' }))).toBe('A');
    expect(taskLine(session({ lastPrompt: 'B', lastReply: 'C' }))).toBe('B');
    expect(taskLine(session({ lastReply: 'C' }))).toBe('C');
    // A process with no readable transcript says nothing rather than guessing.
    expect(taskLine(session())).toBeNull();
  });

  it('renders the tool in flight, with its detail when there is one', () => {
    expect(toolLine(session({ tool: { name: 'Bash', detail: 'cargo test' } }))).toBe(
      'Bash · cargo test',
    );
    expect(toolLine(session({ tool: { name: 'Read' } }))).toBe('Read');
    expect(toolLine(session())).toBeNull();
  });
});
