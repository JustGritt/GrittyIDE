import type { AgentRun, AgentRunId, AgentRunPermissionAsk } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import { permissionVerdictFor } from '../agents/permission-chords';
import { blastRadius, scopeLabel, verbLabel } from '../agents/permission-model';
import { askingRuns, runAskCard, runVerdict } from './run-permission';

/**
 * A run's ask, phrased for the card an observed session's ask already gets
 * (T310-ANSWER). Pure, so the one mapping that decides what a click means is a
 * test rather than a click.
 */

function ask(overrides: Partial<AgentRunPermissionAsk> = {}): AgentRunPermissionAsk {
  return {
    askId: 'ask-1',
    tool: 'Bash',
    verb: 'RUN_COMMAND',
    target: 'cargo test',
    askedAt: 10,
    ...overrides,
  };
}

function run(id: string, pending?: AgentRunPermissionAsk): AgentRun {
  return {
    id: id as AgentRunId,
    workstreamId: 'w1' as AgentRun['workstreamId'],
    workspaceId: 'ws1' as AgentRun['workspaceId'],
    provider: 'CLAUDE_CODE',
    state: pending ? 'BLOCKED' : 'RUNNING',
    startedAt: 0,
    updatedAt: 0,
    cancellationRequested: false,
    correlationId: 'c1' as AgentRun['correlationId'],
    ...(pending ? { pending } : {}),
  };
}

describe('a run’s ask on the card', () => {
  it('keeps the verb and the target the core bounded', () => {
    const card = runAskCard(ask({ detail: 'runs the whole suite' }));
    expect(verbLabel(card.verb)).toBe('run command');
    expect(card.target).toBe('cargo test');
    expect(blastRadius(card)).toEqual(['runs the whole suite']);
  });

  it('offers no remembered answer, so ⌘⏎ cannot quietly allow for the run', () => {
    const card = runAskCard(ask());
    expect(scopeLabel(card)).toBeNull();
    const chord = { code: 'Enter', metaKey: true, ctrlKey: false, altKey: false, shiftKey: false };
    expect(permissionVerdictFor(chord, scopeLabel(card) !== null)).toBeNull();
  });

  it('names the tool when nothing more specific is known about the call', () => {
    const card = runAskCard(ask({ verb: 'OTHER', tool: 'Read', target: '/etc/hosts' }));
    expect(verbLabel(card.verb)).toBe('use tool');
    expect(blastRadius(card)).toEqual(['Read']);
    const described = runAskCard(ask({ verb: 'OTHER', tool: 'mcp__db__query', detail: 'Query' }));
    expect(blastRadius(described)).toEqual(['mcp__db__query · Query']);
  });

  it('claims nothing it cannot know: no diff, nothing outside, nothing redacted', () => {
    const card = runAskCard(ask({ verb: 'WRITE_FILE', target: '/w/.env' }));
    expect(card.diff).toEqual([]);
    expect(card.outsideRoot).toBe(false);
    expect(card.sensitive).toBe(false);
    expect(card.added + card.removed).toBe(0);
  });
});

describe('what an answer means to a run', () => {
  it('maps the card’s two answers a run takes, and refuses the third', () => {
    expect(runVerdict('ALLOW_ONCE')).toBe('ALLOW_ONCE');
    expect(runVerdict('DENY')).toBe('DENY');
    // Not either of the others: that would be a decision nobody made.
    expect(runVerdict('ALLOW_FOR_RUN')).toBeNull();
  });
});

describe('which runs are asking', () => {
  it('lists only runs with an ask, oldest ask first', () => {
    const asking = askingRuns([
      run('late', ask({ askId: 'b', askedAt: 30 })),
      run('busy'),
      run('early', ask({ askId: 'a', askedAt: 10 })),
    ]);
    expect(asking.map((r) => r.id)).toEqual(['early', 'late']);
  });
});
