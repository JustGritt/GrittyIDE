import type { OwnershipMap, Workstream, WorkstreamId } from '@gritty/protocol';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import type { AgentsController } from '../agents/use-agents';
import type { RunsController } from '../runs/use-runs';
import { AgentIsland } from './island';
import { RaiseQuestion } from './raise-question';
import type { WorkstreamsController } from './use-workstreams';

/**
 * The form that closes T210's largest gap, rendered (T210 follow-up 1).
 *
 * A view-model test could not have caught the defect this fixes: `raise` was
 * correct, exported and reachable from the controller, and no component called
 * it. So these assertions are about markup — that the affordance exists, and
 * that the fields are the kind of field the island's ⌥ chords tolerate.
 *
 * Static markup rather than a DOM, matching `agent-timeline-render.test.ts`:
 * `apps/desktop`'s vitest project is a node one.
 */

const NO_OWNERSHIP: OwnershipMap = { leases: [], conflicts: [] };

function workstream(patch: Partial<Workstream> = {}): Workstream {
  return {
    id: 'w-1' as WorkstreamId,
    workspaceId: 'ws-1' as Workstream['workspaceId'],
    title: 'refactor the ring',
    state: 'IN_PROGRESS',
    allowedTransitions: ['BLOCKED', 'IN_REVIEW', 'READY', 'ABANDONED'],
    openDecisions: 0,
    createdAt: 1,
    updatedAt: 1,
    ...patch,
  };
}

function controller(workstreams: readonly Workstream[]): WorkstreamsController {
  const refuse = () => {
    throw new Error('no command should run while rendering');
  };
  return {
    workstreams: [...workstreams],
    decisions: [],
    loaded: true,
    error: null,
    clearError: () => {},
    busy: false,
    create: refuse,
    update: refuse,
    transition: refuse,
    remove: refuse,
    raise: refuse,
    resolve: refuse,
    dismiss: refuse,
    ownership: NO_OWNERSHIP,
    ownershipLoaded: true,
    acquire: refuse,
    release: refuse,
    checkOwnership: refuse,
    resolveConflict: refuse,
    agentLinks: [],
    agentLinksLoaded: true,
    linkAgent: refuse,
    unlinkAgent: refuse,
  };
}

const agents: AgentsController = {
  roster: { sessions: [], needsYou: 0, processEvidence: true },
  loaded: true,
  error: null,
  clearError: () => {},
  busy: false,
  refresh: async () => {},
  answerPermission: async () => {},
  revokeGrant: async () => {},
  answering: null,
};

/** No runs: the island only needs them for the permission asks it counts. */
const runs = {
  runs: [],
  loaded: true,
  error: null,
  answering: null,
} as unknown as RunsController;

function renderForm(): string {
  return renderToStaticMarkup(
    createElement(RaiseQuestion, {
      workstreamId: 'w-1' as WorkstreamId,
      controller: controller([]),
    }),
  );
}

describe('the raise-a-question form', () => {
  const markup = renderForm();

  it('offers a question field and a way to send it', () => {
    expect(markup).toContain('Ask a question');
    expect(markup).toContain('What do you need decided?');
    expect(markup).toContain('Add an option');
  });

  it('uses inputs, never a textarea', () => {
    // `isTypingInField` in island.tsx tests `tagName === 'INPUT'` and
    // deliberately does not exclude Monaco's hidden textarea, because ⌥A from
    // the editor is the point of that shortcut. A textarea here would let ⌥A
    // and ⌥1…9 fire mid-sentence.
    expect(markup).not.toContain('<textarea');
    expect(markup).toContain('<input');
  });

  it('bounds the question at the core’s own limit', () => {
    // Not a second opinion on what is legal — a cheaper refusal than a round
    // trip. The core stays the authority and its message is what is shown.
    expect(markup).toContain('maxLength="4000"');
  });

  it('says what a question with no options means', () => {
    // The difference between the two shapes is a core rule the user did not
    // write: options become the island's ⌥1…9 answers, none means a text reply.
    expect(markup).toContain('answered in words');
  });

  it('cannot be sent empty', () => {
    expect(markup).toMatch(/<button[^>]*type="submit"[^>]*disabled/);
  });
});

describe('the island roster offers the form', () => {
  it('puts an Ask control on every row', () => {
    // The gap this closes: before it, `raise()` had no component caller at all,
    // so manual-pass step 7.7 could not be performed.
    const markup = renderToStaticMarkup(
      createElement(AgentIsland, {
        controller: controller([workstream()]),
        agents,
        runs,
        open: true,
        onOpenChange: () => {},
        onOpenQueue: () => {},
      }),
    );
    expect(markup).toContain('refactor the ring');
    expect(markup).toMatch(/<button[^>]*aria-expanded="false"[^>]*>Ask</);
  });

  it('does not open a form until it is asked for', () => {
    const markup = renderToStaticMarkup(
      createElement(AgentIsland, {
        controller: controller([workstream()]),
        agents,
        runs,
        open: true,
        onOpenChange: () => {},
        onOpenQueue: () => {},
      }),
    );
    // A form open on every row would bury the roster it belongs to.
    expect(markup).not.toContain('What do you need decided?');
  });
});
