import type { AgentRoster, AgentSession, EventEnvelope } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import {
  activityLabel,
  activityTone,
  agentsReducer,
  initialAgentsState,
  runningCount,
  sessionsElsewhere,
  sessionsHere,
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

  it('splits the roster by workspace and counts what is alive', () => {
    const here = session({ id: 'here', inWorkspace: true });
    const away = session({ id: 'away', inWorkspace: false });
    const gone = session({ id: 'gone', inWorkspace: false, activity: 'ENDED' });
    const all = roster([here, away, gone]);

    expect(sessionsHere(all).map((entry) => entry.id)).toEqual(['here']);
    expect(sessionsElsewhere(all).map((entry) => entry.id)).toEqual(['away', 'gone']);
    expect(runningCount(all)).toBe(2);
  });

  it('prefers the agent title, then the prompt, then the reply', () => {
    expect(taskLine(session({ title: 'A', lastPrompt: 'B', lastReply: 'C' }))).toBe('A');
    expect(taskLine(session({ lastPrompt: 'B', lastReply: 'C' }))).toBe('B');
    expect(taskLine(session({ lastReply: 'C' }))).toBe('C');
    // A process with no readable transcript says nothing rather than guessing.
    expect(taskLine(session())).toBeNull();
  });

  it('labels every activity and tones it', () => {
    expect(activityLabel('WORKING')).toBe('Working');
    expect(activityLabel('WAITING')).toBe('Your move');
    expect(activityLabel('IDLE')).toBe('Idle');
    expect(activityLabel('ENDED')).toBe('Ended');
    expect(activityTone('WORKING')).toBe('active');
    expect(activityTone('WAITING')).toBe('waiting');
  });

  it('survives an activity a newer core invented', () => {
    const future = 'WAITING_FOR_APPROVAL' as never;
    expect(activityLabel(future)).toBe('WAITING_FOR_APPROVAL');
    expect(activityTone(future)).toBe('done');
  });

  it('renders the tool in flight, with its detail when there is one', () => {
    expect(toolLine(session({ tool: { name: 'Bash', detail: 'cargo test' } }))).toBe(
      'Bash · cargo test',
    );
    expect(toolLine(session({ tool: { name: 'Read' } }))).toBe('Read');
    expect(toolLine(session())).toBeNull();
  });
});
