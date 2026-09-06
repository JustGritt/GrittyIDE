import type { AgentActivity, AgentActivitySource } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import { activitySourceLabel, agentStatus, providerLabel } from './agent-status';

/**
 * Every activity the wire can carry today. Typed, so adding a value to
 * `AgentActivity` fails this list to compile — which is the point: a status the
 * renderer has never heard of must not reach a user as a blank row.
 */
const ACTIVITIES: readonly AgentActivity[] = ['WORKING', 'BLOCKED', 'WAITING', 'IDLE', 'ENDED'];

describe('agentStatus', () => {
  it('is total over AgentActivity', () => {
    for (const activity of ACTIVITIES) {
      const status = agentStatus(activity);
      expect(status.label).not.toBe('');
      expect(status.tone).not.toBe('unknown');
      expect(status.hint).not.toBe('');
    }
    // One distinct word and one distinct tone per state.
    expect(new Set(ACTIVITIES.map((activity) => agentStatus(activity).label)).size).toBe(
      ACTIVITIES.length,
    );
    expect(new Set(ACTIVITIES.map((activity) => agentStatus(activity).tone)).size).toBe(
      ACTIVITIES.length,
    );
  });

  it('calls WAITING "Done", because the turn finished and it is your move', () => {
    // The wire keeps T220's name; the translation lives in one place so the
    // rail and the island cannot disagree about the word.
    expect(agentStatus('WAITING')).toMatchObject({ label: 'Done', tone: 'done' });
    expect(agentStatus('WORKING')).toMatchObject({ label: 'Working', tone: 'working' });
    expect(agentStatus('IDLE')).toMatchObject({ label: 'Idle', tone: 'idle' });
    expect(agentStatus('ENDED')).toMatchObject({ label: 'Ended', tone: 'ended' });
  });

  it('draws BLOCKED as "Pending validation", the status T221 earned', () => {
    // T231 wrote the label down before any core could send the value; T221's
    // hook bridge is what makes it reachable, and this is the assertion that
    // it now travels the whole way rather than sitting unused.
    expect(agentStatus('BLOCKED')).toMatchObject({
      label: 'Pending validation',
      tone: 'blocked',
    });
  });

  it('shows an activity from a newer core as it arrived', () => {
    // The protocol is additive: an unrecognised value is displayed and marked
    // unrecognised, never folded into a status this build would be guessing.
    const status = agentStatus('SUPERVISING' as AgentActivity);
    expect(status.label).toBe('SUPERVISING');
    expect(status.tone).toBe('unknown');
  });
});

describe('activitySourceLabel', () => {
  it('says where a verdict came from, so a BLOCKED row is attributable', () => {
    const sources: readonly AgentActivitySource[] = ['PROCESS', 'TRANSCRIPT', 'HOOK'];
    for (const source of sources) {
      expect(activitySourceLabel(source)).not.toBe('');
    }
    expect(activitySourceLabel('HOOK')).toContain('agent itself');
    expect(activitySourceLabel('ORACLE' as AgentActivitySource)).toContain('unknown');
  });
});

describe('providerLabel', () => {
  it('names the CLI, and passes an unknown one through', () => {
    expect(providerLabel('CLAUDE_CODE')).toBe('Claude Code');
    expect(providerLabel('CODEX' as 'CLAUDE_CODE')).toBe('CODEX');
  });
});
