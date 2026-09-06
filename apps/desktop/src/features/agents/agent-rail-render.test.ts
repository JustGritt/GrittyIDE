import type { AgentActivitySource, AgentSession } from '@gritty/protocol';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { AgentRail } from './agent-rail';
import type { AgentRailController } from './use-agent-rail';
import type { AgentsController } from './use-agents';

/**
 * What a rail row actually puts on the screen (T221 §24).
 *
 * §24 is a *Required behavior* item — "a row can say where its verdict came
 * from, so a `BLOCKED` row is attributable rather than magic" — and the way it
 * was missed is instructive: `activitySourceLabel` existed, was widened for an
 * unknown value, and was tested, but no component called it. A view-model test
 * cannot catch that. This renders the component.
 *
 * Static markup rather than a DOM, matching `agent-timeline-render.test.ts`:
 * `apps/desktop`'s vitest project is a node one, and a string is a stronger
 * assertion than a query against a tree anyway.
 */

function session(overrides: Partial<AgentSession> = {}): AgentSession {
  return {
    id: 'session-1',
    provider: 'CLAUDE_CODE',
    activity: 'WORKING',
    activitySource: 'TRANSCRIPT',
    evidence: 'BOTH',
    inWorkspace: true,
    title: 'refactor the ring',
    ...overrides,
  };
}

function agentsController(sessions: readonly AgentSession[]): AgentsController {
  return {
    roster: { sessions: [...sessions], needsYou: 0, processEvidence: true },
    loaded: true,
    error: null,
    clearError: () => {},
    busy: false,
    refresh: async () => {},
    answerPermission: async () => {},
    revokeGrant: async () => {},
    answering: null,
  };
}

const rail: AgentRailController = {
  open: true,
  selectedId: null,
  pendingId: null,
  timeline: null,
  openingPath: null,
  canOpenFiles: true,
  error: null,
  clearError: () => {},
  setOpen: () => {},
  toggle: () => {},
  select: () => {
    throw new Error('nothing should be selected while rendering');
  },
  openFile: () => {
    throw new Error('nothing should open while rendering');
  },
};

function render(sessions: readonly AgentSession[]): string {
  return renderToStaticMarkup(
    createElement(AgentRail, { agents: agentsController(sessions), rail, chords: false }),
  );
}

describe('a rail row says where its verdict came from', () => {
  it('attributes every source the protocol defines', () => {
    // Not one representative case: the point of §24 is that the three sources
    // can disagree, so each has to be distinguishable on the row.
    const sources: readonly AgentActivitySource[] = ['PROCESS', 'TRANSCRIPT', 'HOOK'];
    const markup = render(
      sources.map((activitySource, index) =>
        session({ id: `session-${index}`, activitySource, title: activitySource }),
      ),
    );
    expect(markup).toContain('from the process table');
    expect(markup).toContain('from the transcript');
    expect(markup).toContain('reported by the agent itself');
  });

  it('attributes a blocked row, which is the one the user has to trust', () => {
    // "Pending validation" is the exact phrase §24 calls magic without this:
    // it is the one status the user is meant to act on, so it is the one that
    // most needs to say how it was arrived at.
    const markup = render([session({ activity: 'BLOCKED', activitySource: 'HOOK' })]);
    expect(markup).toContain('Pending validation');
    expect(markup).toContain('reported by the agent itself');
  });

  it('says something about a source from a newer core rather than nothing', () => {
    // The protocol is additive. A row that silently drops its provenance is
    // worse than one that admits it does not recognise it.
    const markup = render([
      session({ activitySource: 'ORACLE' as AgentActivitySource, title: 'from later' }),
    ]);
    expect(markup).toContain('from an unknown source');
  });

  it('renders a hostile session title as text', () => {
    // The same untrusted-input rule the timeline is held to (hard rule 13):
    // every field on a row came out of a transcript.
    const markup = render([session({ title: '<script>alert(1)</script>' })]);
    expect(markup).toContain('&lt;script&gt;alert(1)&lt;/script&gt;');
    expect(markup).not.toContain('<script');
  });
});
