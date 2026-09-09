import type {
  AgentActivitySource,
  AgentRoster,
  AgentRun,
  AgentRunId,
  AgentSession,
  Workstream,
  WorkstreamId,
} from '@gritty/protocol';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { AgentConsole, agentTabItems } from './agent-console';
import { parseConsoleKey } from './agent-tabs-reducer';
import { rosterRows } from './roster-model';
import type { AgentConsoleController } from './use-agent-console';
import type { AgentsController } from './use-agents';

/**
 * What the console actually puts on the screen (T233-C).
 *
 * The §24 half of this file is inherited from `agent-rail-render.test.ts`, and
 * inheriting it is the point. §24 is a *Required behavior* item — "a row can
 * say where its verdict came from, so a `BLOCKED` row is attributable rather
 * than magic" — and `agent-rail.tsx` was its only caller in the app. Deleting
 * the rail without moving these assertions would have un-shipped it with no
 * failing test, which is exactly the defect this style of test exists for:
 * `activitySourceLabel` was written, widened and unit-tested, and none of that
 * would have noticed that nothing called it.
 *
 * Static markup rather than a DOM: `apps/desktop`'s vitest project is a node
 * one, and a string is a stronger assertion than a query against a tree anyway.
 */

/**
 * The tags, with their attribute *values* blanked.
 *
 * `agent-timeline-render.test.ts` scans whole tags, which works because every
 * agent string there lands in a text node. The console puts one in an
 * attribute — the full working directory rides a `title`, because the header
 * truncates it — and `title="&lt;img src=x …&gt;"` is agent text a browser will
 * never act on. Scanning the raw tag would flag that `src=` and fail, which is
 * the same false positive the timeline's test warns about in its own terms:
 * escaped content is the safe outcome, not the dangerous one. So the values go
 * and the attribute names stay, which is what the assertions are actually about.
 */
function tagsOf(markup: string): readonly string[] {
  return (markup.match(/<[^>]*>/g) ?? []).map((tag) => tag.replace(/"[^"]*"/g, '""'));
}

function session(overrides: Partial<AgentSession> = {}): AgentSession {
  return {
    id: 'session-1',
    provider: 'CLAUDE_CODE',
    activity: 'WORKING',
    activitySource: 'TRANSCRIPT',
    evidence: 'BOTH',
    inWorkspace: true,
    rootName: 'gritty',
    root: '~/Desktop/CurrentProjects/GrittyIDE',
    title: 'refactor the ring',
    ...overrides,
  };
}

const refuse = (what: string) => () => {
  throw new Error(`nothing should ${what} while rendering`);
};

const controller: AgentConsoleController = {
  openTabs: ['session:session-1'],
  previewId: null,
  activeId: 'session:session-1',
  activeTarget: { kind: 'session', sessionId: 'session-1' },
  pendingId: null,
  timeline: null,
  openingPath: null,
  canOpenFiles: true,
  error: null,
  clearError: () => {},
  show: refuse('be shown'),
  keep: refuse('be kept'),
  close: refuse('be closed'),
  showDocuments: refuse('switch to the documents'),
  openFile: refuse('open a file'),
};

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

function render(overrides: Partial<AgentSession> = {}): string {
  return renderToStaticMarkup(
    createElement(AgentConsole, { session: session(overrides), controller, agents }),
  );
}

describe('the console says where its verdict came from (T221 §24)', () => {
  it('attributes every source the protocol defines', () => {
    // Not one representative case: the point of §24 is that the three sources
    // can disagree, so each has to be distinguishable.
    const sources: readonly AgentActivitySource[] = ['PROCESS', 'TRANSCRIPT', 'HOOK'];
    const seen = sources.map((activitySource) => render({ activitySource }));
    expect(seen[0]).toContain('from the process table');
    expect(seen[1]).toContain('from the transcript');
    expect(seen[2]).toContain('reported by the agent itself');
    // Three sources, three distinct sentences.
    expect(new Set(seen).size).toBe(3);
  });

  it('says so for a source a newer core invented, rather than silently omitting it', () => {
    expect(render({ activitySource: 'ORACLE' as AgentActivitySource })).toContain(
      'from an unknown source',
    );
  });

  it('attributes a BLOCKED session, which is the case the rule exists for', () => {
    const markup = render({ activity: 'BLOCKED', activitySource: 'HOOK' });
    expect(markup).toContain('Pending validation');
    expect(markup).toContain('reported by the agent itself');
  });
});

describe('the console header', () => {
  it('names the provider, the directory and the branch', () => {
    const markup = render({ gitBranch: 't090/terminal-pty' });
    expect(markup).toContain('Claude Code');
    expect(markup).toContain('GrittyIDE');
    expect(markup).toContain('t090/terminal-pty');
  });

  it('says the directory is unknown rather than leaving a gap', () => {
    expect(render({ root: undefined })).toContain('directory unknown');
  });

  it('says there is no transcript rather than drawing an empty line', () => {
    expect(render({ title: undefined, lastPrompt: undefined, lastReply: undefined })).toContain(
      'No transcript yet',
    );
  });

  it('says the status in words as well as in a tone', () => {
    const markup = render({ activity: 'WAITING' });
    expect(markup).toContain('Done');
    expect(markup).toContain('agent-status--done');
  });
});

describe('agent text is text', () => {
  // Every string here came out of a transcript. T232 §7 is the rule the console
  // inherits whole: never `innerHTML`, never Markdown, never a resolved link.
  const nasty = '<script>alert(1)</script><img src=x onerror=alert(2)>';
  const markup = render({ title: nasty, root: nasty, gitBranch: nasty });

  it('renders no tag an agent supplied', () => {
    for (const tag of tagsOf(markup)) {
      expect(tag).not.toMatch(/^<\/?(script|img|iframe|object|embed|style|a)[\s/>]/i);
    }
  });

  it('renders nothing that navigates or runs', () => {
    for (const tag of tagsOf(markup)) {
      expect(tag).not.toMatch(/\shref/i);
      expect(tag).not.toMatch(/\ssrc=/i);
      expect(tag).not.toMatch(/\son[a-z]+=/i);
    }
  });

  it('still shows what the agent wrote, escaped', () => {
    expect(markup).toContain('&lt;script&gt;');
  });
});

describe('the console has no prompt box (T310-U §9)', () => {
  it('draws no way to speak to a session another terminal owns', () => {
    // "No prompt box **and no disabled one either**": there is nothing to
    // enable, and a greyed-out control implies a permission the user could
    // acquire. Speaking to an agent is `RunConsole`'s, in the other lane.
    const markup = render();
    expect(markup).not.toContain('<textarea');
    expect(markup).not.toContain('agent-prompt');
  });
});

/**
 * The tab strip is handed rows, not sessions (T310-U).
 *
 * `rosterRows` is what builds them — the same call `App` makes — rather than
 * hand-written literals, because the assertion that actually matters is that
 * the key a row carries and the key a tab is opened with are one string. A
 * fixture spelling `run:…` itself would pass while the two drifted apart.
 */
function rows(runs: readonly AgentRun[], sessions: readonly AgentSession[]) {
  const roster: AgentRoster = { sessions: [...sessions], needsYou: 0, processEvidence: true };
  const workstreams: readonly Workstream[] = [
    {
      id: 'w1' as WorkstreamId,
      workspaceId: 'ws1' as Workstream['workspaceId'],
      title: 'Ship the ring',
      state: 'IN_PROGRESS',
      allowedTransitions: [],
      openDecisions: 0,
      createdAt: 0,
      updatedAt: 0,
    },
  ];
  return rosterRows(runs, roster, workstreams);
}

function run(overrides: Partial<AgentRun> = {}): AgentRun {
  return {
    id: 'run-1' as AgentRunId,
    workstreamId: 'w1' as WorkstreamId,
    workspaceId: 'ws1' as AgentRun['workspaceId'],
    provider: 'CLAUDE_CODE',
    state: 'RUNNING',
    startedAt: 0,
    updatedAt: 0,
    cancellationRequested: false,
    correlationId: 'c1' as AgentRun['correlationId'],
    ...overrides,
  };
}

describe('the row a user clicks and the tab it opens are one identity', () => {
  it('mints keys the console can read back, for both lanes', () => {
    // The two modules spell the key in two places — `rosterRows` builds it,
    // `parseConsoleKey` takes it apart — and nothing else would notice if they
    // drifted. This is what notices.
    const built = rows([run()], [session()]);
    expect(built.map((row) => parseConsoleKey(row.key))).toEqual(
      expect.arrayContaining([
        { kind: 'run', runId: 'run-1' },
        { kind: 'session', sessionId: 'session-1' },
      ]),
    );
  });
});

describe('the tabs the strip is handed', () => {
  it('labels a console by its folder, not by its id', () => {
    const items = agentTabItems(['session:session-1'], null, rows([], [session()]));
    expect(items).toHaveLength(1);
    expect(items[0]?.label).toBe('gritty');
    expect(items[0]?.tone).toBe('working');
  });

  it('falls back to a short id when the directory was never seen', () => {
    const items = agentTabItems(
      ['session:session-1'],
      null,
      rows([], [session({ rootName: undefined })]),
    );
    expect(items[0]?.label).toBe('session');
  });

  it('gives a run Gritty owns a tab too, keyed in its own lane', () => {
    // The two lanes cannot collide: a run is deliberately absent from the
    // observed roster (T310-B §8), so its key is the only thing that can name
    // it here.
    const items = agentTabItems(['run:run-1'], null, rows([run()], [session()]));
    expect(items).toHaveLength(1);
    expect(items[0]?.id).toBe('run:run-1');
    expect(items[0]?.label).toBe('run-1');
    expect(items[0]?.tone).toBe('working');
  });

  it('marks the preview, so the strip can italicise it', () => {
    const items = agentTabItems(['session:session-1'], 'session:session-1', rows([], [session()]));
    expect(items[0]?.preview).toBe(true);
  });

  it('drops a tab whose session has left the roster', () => {
    // A tab that could only be drawn as a bare id is worse than one that is
    // gone; the reducer removes it on the same roster publish.
    expect(agentTabItems(['session:session-1'], null, [])).toEqual([]);
  });

  it('drops a run tab the same way when its workspace closed', () => {
    expect(agentTabItems(['run:run-1'], null, rows([], [session()]))).toEqual([]);
  });

  it('carries the status into the tab title, since the tab has no room for it', () => {
    const items = agentTabItems(
      ['session:session-1'],
      null,
      rows([], [session({ activity: 'BLOCKED' })]),
    );
    expect(items[0]?.hint).toContain('Pending validation');
  });

  it('says a run is blocked in the run vocabulary, not the session one', () => {
    const items = agentTabItems(['run:run-1'], null, rows([run({ state: 'BLOCKED' })], []));
    expect(items[0]?.hint).toContain('Blocked');
    expect(items[0]?.tone).toBe('blocked');
  });
});
