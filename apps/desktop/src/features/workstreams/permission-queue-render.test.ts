import type {
  AgentPermissionAnswer,
  AgentRun,
  AgentRunId,
  AgentSession,
  OwnershipMap,
  Workstream,
  WorkstreamId,
} from '@gritty/protocol';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import type { AgentsController } from '../agents/use-agents';
import type { RunsController } from '../runs/use-runs';
import { AttentionQueue } from './attention-queue';
import type { WorkstreamsController } from './use-workstreams';

/**
 * The queue draws a permission ask, and the ask can be answered from it
 * (T335, artboard 12b).
 *
 * The same guard `agent-timeline-render.test.ts` carries, for the same reason:
 * the review that sent T210, T221 and T232 back found a correct view model with
 * no caller three times, and only a render catches that. Here the risk is
 * specific — `attentionItems` gained a fourth kind, and a queue that filtered
 * it, counted it and never drew it would look entirely healthy in a unit test.
 *
 * Static markup, no DOM: `apps/desktop`'s vitest project is a node one.
 */

const NO_OWNERSHIP: OwnershipMap = { leases: [], conflicts: [] };

/** Everything an attacker-controlled tool input might try. */
const HOSTILE = '<script>alert(1)</script><img src=x onerror=alert(2)>';

function controller(workstreams: Workstream[] = []): WorkstreamsController {
  const refuse = () => {
    throw new Error('no command should run while rendering');
  };
  return {
    workstreams,
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
  } as WorkstreamsController;
}

function asking(): AgentSession {
  return {
    id: 'claude_code:s1',
    provider: 'CLAUDE_CODE',
    activity: 'BLOCKED',
    activitySource: 'HOOK',
    evidence: 'BOTH',
    inWorkspace: true,
    title: 'workstream leases',
    root: '~/wt/t080',
    pending: {
      askId: 'ask-1',
      sessionId: 'claude_code:s1',
      verb: 'RUN_COMMAND',
      target: `cargo publish --dry-run ${HOSTILE}`,
      detail: 'runs a command that reaches the network',
      diff: [] as string[],
      added: 0,
      removed: 0,
      outsideRoot: false,
      sensitive: false,
      askedAt: 0,
      suggestedScope: 'cargo',
    },
  } as AgentSession;
}

function agentsWith(
  sessions: readonly AgentSession[],
  recentAnswers: readonly AgentPermissionAnswer[] = [],
): AgentsController {
  return {
    roster: {
      sessions: [...sessions],
      needsYou: sessions.length,
      processEvidence: true,
      recentAnswers: [...recentAnswers],
    },
    loaded: true,
    error: null,
    clearError: () => {},
    busy: false,
    refresh: async () => {},
    answerPermission: async () => {},
    revokeGrant: async () => {},
    answering: null,
  } as AgentsController;
}

function runsWith(runs: readonly AgentRun[]): RunsController {
  return {
    runs: [...runs],
    loaded: true,
    error: null,
    answering: null,
    answerPermission: async () => {
      throw new Error('no command should run while rendering');
    },
  } as unknown as RunsController;
}

function render(
  agents: AgentsController,
  runs: RunsController = runsWith([]),
  workstreams: Workstream[] = [],
): string {
  return renderToStaticMarkup(
    createElement(AttentionQueue, {
      controller: controller(workstreams),
      agents,
      runs,
      onClose: () => {
        throw new Error('nothing should close while rendering');
      },
    }),
  );
}

describe('the queue answers a background agent’s ask', () => {
  const markup = render(agentsWith([asking()]));

  it('lists the ask, with its verb, target and blast radius on the row', () => {
    // 12b's whole claim: you can judge it without opening the console first.
    expect(markup).toContain('Permission');
    expect(markup).toContain('run command');
    expect(markup).toContain('cargo publish --dry-run');
    expect(markup).toContain('reaches the network');
  });

  it('draws the answers, which is the part a unit test cannot see', () => {
    expect(markup).toContain('Allow once');
    expect(markup).toContain('Allow cargo for this run');
    expect(markup).toContain('Deny');
  });

  it('gives permissions their own filter tab', () => {
    expect(markup).toContain('Permissions');
  });

  it('says the prompt is also live in the agent’s own terminal', () => {
    // ADR-020: both answer paths are live at once, and hiding that would make
    // a prompt that vanishes on its own look like a bug.
    expect(markup).toContain('whichever comes first wins');
  });

  it('renders the agent’s own tool input as text', () => {
    for (const tag of markup.match(/<[^>]*>/g) ?? []) {
      expect(tag).not.toMatch(/^<\/?(script|img|iframe|object|embed)\b/i);
      expect(tag).not.toMatch(/\son\w+=/i);
    }
    expect(markup).toContain('&lt;script&gt;');
  });

  it('offers to take back a standing rule, and calls it that', () => {
    const answered = render(
      agentsWith(
        [],
        [
          {
            askId: 'ask-0',
            sessionId: 'claude_code:s1',
            verb: 'WRITE_FILE',
            target: '~/p/src/pty.rs',
            verdict: 'ALLOW_FOR_RUN',
            answeredAt: 0,
            grantId: 'grant-1',
          } as AgentPermissionAnswer,
        ],
      ),
    );
    expect(answered).toContain('Answered just now');
    expect(answered).toContain('allowed for this run');
    // Not "Undo": the tool has already run, and only the standing part can go.
    expect(answered).toContain('Undo the standing rule');
  });

  it('offers no undo for an answer that granted nothing', () => {
    const denied = render(
      agentsWith(
        [],
        [
          {
            askId: 'ask-0',
            sessionId: 'claude_code:s1',
            verb: 'RUN_COMMAND',
            target: 'rm -rf build',
            verdict: 'DENY',
            answeredAt: 0,
          } as AgentPermissionAnswer,
        ],
      ),
    );
    expect(denied).toContain('denied');
    expect(denied).not.toContain('Undo the standing rule');
  });
});

/**
 * An owned run's ask joins the same queue and the same card (T310-ANSWER). A
 * user should not have to learn which lane an ask came from to answer it — but
 * the one thing that does differ, that nothing else is holding this prompt open,
 * has to be said.
 */
describe('the queue answers a run Gritty owns', () => {
  const ship: Workstream = {
    id: 'w-ship' as WorkstreamId,
    workspaceId: 'ws-1' as Workstream['workspaceId'],
    title: 'Ship the permission verdict',
    state: 'IN_PROGRESS',
    allowedTransitions: [],
    openDecisions: 0,
    createdAt: 1,
    updatedAt: 1,
  };
  const blocked: AgentRun = {
    id: 'run-1' as AgentRunId,
    workstreamId: ship.id,
    workspaceId: 'ws-1' as AgentRun['workspaceId'],
    provider: 'CLAUDE_CODE',
    state: 'BLOCKED',
    startedAt: 0,
    updatedAt: 0,
    cancellationRequested: false,
    correlationId: 'c1' as AgentRun['correlationId'],
    pending: {
      askId: 'run-ask-1',
      tool: 'Bash',
      verb: 'RUN_COMMAND',
      target: `touch probe.txt ${HOSTILE}`,
      askedAt: 0,
    },
  };
  const markup = render(agentsWith([]), runsWith([blocked]), [ship]);

  it('lists the ask under its workstream, with its verb and target', () => {
    expect(markup).toContain('Permission');
    expect(markup).toContain('Ship the permission verdict');
    expect(markup).toContain('run command');
    expect(markup).toContain('touch probe.txt');
  });

  it('draws Allow once and Deny, and no answer that would remember', () => {
    expect(markup).toContain('Allow once');
    expect(markup).toContain('Deny');
    expect(markup).not.toContain('for this run');
  });

  it('says the run is held on this answer alone', () => {
    expect(markup).toContain('held until you answer');
    // The session's line would be false here: a run's child has no terminal.
    expect(markup).not.toContain('whichever comes first wins');
  });

  it('renders the agent’s own tool input as text', () => {
    for (const tag of markup.match(/<[^>]*>/g) ?? []) {
      expect(tag).not.toMatch(/^<\/?(script|img|iframe|object|embed)\b/i);
      expect(tag).not.toMatch(/\son\w+=/i);
    }
    expect(markup).toContain('&lt;script&gt;');
  });

  it('counts the run’s ask beside a session’s under one Permissions filter', () => {
    const both = render(agentsWith([asking()]), runsWith([blocked]), [ship]);
    expect(both).toMatch(/Permissions<span class="queue__filter-count">2</);
  });
});
