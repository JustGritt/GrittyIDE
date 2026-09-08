import type { AgentRunState } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import { runStatus } from './run-status';

/**
 * Every state the wire can carry today. Typed, so adding a value to
 * `AgentRunState` fails this list to compile — which is the point: a state the
 * renderer has never heard of must not reach a user as a blank row.
 */
const STATES: readonly AgentRunState[] = [
  'STARTING',
  'RUNNING',
  'WAITING',
  'NEEDS_DECISION',
  'BLOCKED',
  'REVIEW',
  'FINISHED',
  'FAILED',
  'CANCELLED',
  'INTERRUPTED',
];

describe('runStatus', () => {
  it('is total over AgentRunState', () => {
    for (const state of STATES) {
      const status = runStatus(state);
      expect(status.label).not.toBe('');
      expect(status.tone).not.toBe('unknown');
      expect(status.hint).not.toBe('');
    }
    // One distinct word per state. Tones deliberately collapse — ten states
    // onto six tones — but two states must never read as the same thing.
    expect(new Set(STATES.map((state) => runStatus(state).label)).size).toBe(STATES.length);
  });

  it('separates the three answers a terminal run can give', () => {
    // "The agent broke", "we stopped being able to watch it" and "someone
    // asked it to stop" are three different answers, and a user deciding
    // whether to retry needs to know which.
    expect(runStatus('FAILED').label).toBe('Failed');
    expect(runStatus('INTERRUPTED').label).toBe('Interrupted');
    expect(runStatus('CANCELLED').label).toBe('Cancelled');
  });

  it('does not draw a failure in the grey a session that merely ended gets', () => {
    // A session has no failure state, so `ended` is as far as `AgentStatusTone`
    // goes. Reusing it here would downgrade the one outcome the user most
    // needs to see.
    expect(runStatus('FAILED').tone).toBe('failed');
    expect(runStatus('INTERRUPTED').tone).toBe('failed');
    expect(runStatus('FINISHED').tone).toBe('ended');
    expect(runStatus('CANCELLED').tone).toBe('ended');
  });

  it('keeps STARTING distinct from RUNNING, in words', () => {
    // The state exists precisely to say that a process being alive is not the
    // same as the run being usable (Phase 0 §D). Collapsing the two labels
    // would throw away the only reason it is on the wire.
    expect(runStatus('STARTING').label).not.toBe(runStatus('RUNNING').label);
    expect(runStatus('STARTING').tone).toBe('working');
    expect(runStatus('RUNNING').tone).toBe('working');
  });

  it('tones the three states that hold for you as blocked, and REVIEW as done', () => {
    expect(runStatus('WAITING').tone).toBe('blocked');
    expect(runStatus('NEEDS_DECISION').tone).toBe('blocked');
    expect(runStatus('BLOCKED').tone).toBe('blocked');
    // Nothing is being withheld from a run in REVIEW; it finished a unit of
    // work, which is what a session's WAITING means and is drawn the same way.
    expect(runStatus('REVIEW').tone).toBe('done');
  });

  it('shows a state from a newer core as it arrived, rather than inventing one', () => {
    const status = runStatus('TELEPORTING' as AgentRunState);
    expect(status.label).toBe('TELEPORTING');
    expect(status.tone).toBe('unknown');
    expect(status.hint).toContain('newer core');
  });
});
