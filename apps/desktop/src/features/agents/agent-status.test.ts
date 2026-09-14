import type { AgentActivity, AgentActivitySource } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import {
  type AgentStatusSubject,
  activitySourceLabel,
  agentStatus,
  providerLabel,
  sessionNeedsYou,
  turnEnded,
} from './agent-status';

/**
 * Every status the wire can produce today: each activity with the source that
 * carries its evidence, plus the one pair whose word depends on the source.
 * Typed, so adding a value to `AgentActivity` fails this list to compile —
 * which is the point: a status the renderer has never heard of must not reach
 * a user as a blank row.
 */
const SUBJECTS: readonly AgentStatusSubject[] = [
  { activity: 'WORKING', activitySource: 'TRANSCRIPT' },
  { activity: 'BLOCKED', activitySource: 'HOOK' },
  { activity: 'WAITING', activitySource: 'TRANSCRIPT' },
  { activity: 'WAITING', activitySource: 'PROCESS' },
  { activity: 'IDLE', activitySource: 'TRANSCRIPT' },
  { activity: 'ENDED', activitySource: 'PROCESS' },
];

const is = (activity: AgentActivity, activitySource: AgentActivitySource): AgentStatusSubject => ({
  activity,
  activitySource,
});

describe('agentStatus', () => {
  it('is total over the statuses the core produces', () => {
    for (const subject of SUBJECTS) {
      const status = agentStatus(subject);
      expect(status.label).not.toBe('');
      expect(status.tone).not.toBe('unknown');
      expect(status.hint).not.toBe('');
    }
    // One distinct word and one distinct tone per status: the marker's shape
    // lives in the tone, so two statuses sharing one would differ by hue alone.
    expect(new Set(SUBJECTS.map((subject) => agentStatus(subject).label)).size).toBe(
      SUBJECTS.length,
    );
    expect(new Set(SUBJECTS.map((subject) => agentStatus(subject).tone)).size).toBe(
      SUBJECTS.length,
    );
  });

  it('calls WAITING "Done" only when something saw the turn end', () => {
    // The wire keeps T220's name; the translation lives in one place so the
    // rail and the island cannot disagree about the word.
    expect(agentStatus(is('WAITING', 'TRANSCRIPT'))).toMatchObject({ label: 'Done', tone: 'done' });
    expect(agentStatus(is('WAITING', 'HOOK'))).toMatchObject({ label: 'Done', tone: 'done' });
    expect(agentStatus(is('WORKING', 'HOOK'))).toMatchObject({ label: 'Working', tone: 'working' });
    expect(agentStatus(is('IDLE', 'TRANSCRIPT'))).toMatchObject({ label: 'Idle', tone: 'idle' });
    expect(agentStatus(is('ENDED', 'PROCESS'))).toMatchObject({ label: 'Ended', tone: 'ended' });
  });

  it('calls a WAITING with only the process table behind it "Alive", not Done', () => {
    // AGENT-TURN-END: a live process with nothing outstanding may be a model
    // mid-thought. Done would be a claim no evidence made.
    expect(agentStatus(is('WAITING', 'PROCESS'))).toMatchObject({ label: 'Alive', tone: 'alive' });
  });

  it('draws BLOCKED as "Pending validation", the status T221 earned', () => {
    // T231 wrote the label down before any core could send the value; T221's
    // hook bridge is what makes it reachable, and this is the assertion that
    // it now travels the whole way rather than sitting unused.
    expect(agentStatus(is('BLOCKED', 'HOOK'))).toMatchObject({
      label: 'Pending validation',
      tone: 'blocked',
    });
  });

  it('shows an activity from a newer core as it arrived', () => {
    // The protocol is additive: an unrecognised value is displayed and marked
    // unrecognised, never folded into a status this build would be guessing.
    const status = agentStatus(is('SUPERVISING' as AgentActivity, 'HOOK'));
    expect(status.label).toBe('SUPERVISING');
    expect(status.tone).toBe('unknown');
  });
});

describe('sessionNeedsYou', () => {
  it('is the core rule: a prompt, or a turn seen to end', () => {
    const needing = SUBJECTS.filter(sessionNeedsYou);
    expect(needing).toEqual([is('BLOCKED', 'HOOK'), is('WAITING', 'TRANSCRIPT')]);
    expect(sessionNeedsYou(is('WAITING', 'HOOK'))).toBe(true);
    expect(sessionNeedsYou(is('WAITING', 'PROCESS'))).toBe(false);
  });

  it('separates the turn ending from the prompt', () => {
    expect(turnEnded(is('WAITING', 'HOOK'))).toBe(true);
    expect(turnEnded(is('BLOCKED', 'HOOK'))).toBe(false);
    expect(turnEnded(is('WAITING', 'PROCESS'))).toBe(false);
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
