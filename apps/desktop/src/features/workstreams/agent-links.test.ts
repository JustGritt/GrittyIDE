import type { AgentRoster, AgentSession, Workstream, WorkstreamAgentLink } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import {
  type LinkedSession,
  linkedSessionLabel,
  linkedSessions,
  linkedStatusHint,
  linkedStatusLabel,
  linkedSummary,
  linksOf,
  sameSession,
  workstreamsOfSession,
} from './agent-links';

function session(id: string, activity: AgentSession['activity'] = 'WORKING'): AgentSession {
  return {
    id,
    provider: 'CLAUDE_CODE',
    activity,
    activitySource: 'PROCESS',
    evidence: 'BOTH',
    title: `task of ${id}`,
    inWorkspace: true,
  };
}

function roster(sessions: AgentSession[]): AgentRoster {
  return {
    sessions,
    needsYou: sessions.filter((s) => s.activity === 'WAITING' || s.activity === 'BLOCKED').length,
    processEvidence: true,
  };
}

function link(workstreamId: string, sessionId: string, linkedAt = 1): WorkstreamAgentLink {
  return {
    workstreamId: workstreamId as WorkstreamAgentLink['workstreamId'],
    provider: 'CLAUDE_CODE',
    sessionId,
    linkedAt,
  };
}

function workstream(id: string, title: string): Workstream {
  return {
    id: id as Workstream['id'],
    workspaceId: 'ws-1' as Workstream['workspaceId'],
    title,
    state: 'IN_PROGRESS',
    allowedTransitions: ['BLOCKED', 'IN_REVIEW', 'READY', 'ABANDONED'],
    openDecisions: 0,
    createdAt: 1,
    updatedAt: 2,
  };
}

describe('agent links', () => {
  it('renders a link whose session has left the roster as "not running"', () => {
    const entries = linkedSessions(
      [link('w-1', 's-gone'), link('w-1', 's-live', 2)],
      roster([session('s-live')]),
      'w-1' as Workstream['id'],
    );
    expect(entries).toHaveLength(2);
    const [live, gone] = entries as [LinkedSession, LinkedSession];
    // Running first, so what can still be learned from is at the top.
    expect(live.session?.id).toBe('s-live');
    expect(live.link.sessionId).toBe('s-live');
    expect(gone.session).toBeNull();

    expect(linkedStatusLabel(gone)).toBe('Not running');
    expect(linkedStatusLabel(live)).toBe('Working');
    // Never "ended": Gritty knows the session is not in the roster it can see,
    // not that the process died.
    expect(linkedStatusHint(gone)).toContain('not in the roster');
    // And it stays identifiable enough to remove.
    expect(linkedSessionLabel(gone)).toBe('Claude Code · s-gone');
    expect(linkedSessionLabel(live)).toBe('task of s-live');
  });

  it('counts what is linked and how much of it is gone', () => {
    expect(linkedSummary([])).toBeNull();
    const one = linkedSessions([link('w-1', 's-1')], roster([session('s-1')]), 'w-1' as never);
    expect(linkedSummary(one)).toBe('1 agent');
    const mixed = linkedSessions(
      [link('w-1', 's-1'), link('w-1', 's-2')],
      roster([session('s-1')]),
      'w-1' as never,
    );
    expect(linkedSummary(mixed)).toBe('2 agents · 1 not running');
  });

  it('keeps one workstream’s links out of another’s', () => {
    const links = [link('w-1', 's-1'), link('w-2', 's-1')];
    expect(linksOf(links, 'w-1' as never)).toHaveLength(1);
    // The same session on two workstreams is allowed, and both show it.
    expect(
      workstreamsOfSession(
        links,
        [workstream('w-1', 'A'), workstream('w-2', 'B')],
        session('s-1'),
      ).map((w) => w.title),
    ).toEqual(['A', 'B']);
  });

  it('matches on the provider as well as the id', () => {
    expect(sameSession(link('w-1', 's-1'), session('s-1'))).toBe(true);
    expect(sameSession(link('w-1', 's-1'), { id: 's-1', provider: 'OTHER' as never })).toBe(false);
    expect(sameSession(link('w-1', 's-1'), session('s-2'))).toBe(false);
  });

  it('shows nothing for a workstream nobody linked', () => {
    expect(linkedSessions([], roster([session('s-1')]), 'w-1' as never)).toEqual([]);
    expect(workstreamsOfSession([], [workstream('w-1', 'A')], session('s-1'))).toEqual([]);
  });
});
