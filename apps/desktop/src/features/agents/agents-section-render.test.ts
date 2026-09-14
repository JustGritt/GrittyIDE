import type { AgentRoster, AgentRun, AgentRunId, AgentSession, Workstream } from '@gritty/protocol';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { SidebarAgents } from './agents-section';
import { rosterRows } from './roster-model';

/**
 * What a section row actually puts on the screen (T233-S).
 *
 * A view-model test cannot catch a correct model with no caller, which is the
 * defect the review found three times over in T210, T221 and T232. `rosterRows`
 * is unit-tested next door; this renders the component that is supposed to be
 * calling it, and asserts on the markup that comes out.
 *
 * Static markup rather than a DOM, matching `agent-timeline-render.test.ts`:
 * `apps/desktop`'s vitest project is a node one, and a string is a stronger
 * assertion than a query against a tree anyway.
 */

function tagsOf(markup: string): readonly string[] {
  return markup.match(/<[^>]*>/g) ?? [];
}

function run(overrides: Partial<AgentRun> = {}): AgentRun {
  return {
    id: 'run-1234567890' as AgentRunId,
    workstreamId: 'wst-1' as AgentRun['workstreamId'],
    workspaceId: 'w-1' as AgentRun['workspaceId'],
    provider: 'FAKE',
    state: 'RUNNING',
    startedAt: 1_800_000_000_000,
    updatedAt: 1_800_000_000_000,
    cancellationRequested: false,
    correlationId: 'corr-1' as AgentRun['correlationId'],
    ...overrides,
  };
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
    title: 'refactor the ring',
    ...overrides,
  };
}

function workstream(overrides: Partial<Workstream> = {}): Workstream {
  return {
    id: 'wst-1' as Workstream['id'],
    workspaceId: 'w-1' as Workstream['workspaceId'],
    title: 'terminal pty reflow',
    state: 'IN_PROGRESS',
    allowedTransitions: [],
    openDecisions: 0,
    createdAt: 1_800_000_000_000,
    updatedAt: 1_800_000_000_000,
    ...overrides,
  };
}

/**
 * Every callback throws. Rendering must be free of side effects, and a stub
 * that merely records would let a stray call pass unnoticed.
 */
const refuse = (what: string) => () => {
  throw new Error(`nothing should ${what} while rendering`);
};

function render(
  runs: readonly AgentRun[],
  sessions: readonly AgentSession[],
  options: {
    open?: boolean;
    needsYou?: number;
    loaded?: boolean;
    selectedKey?: string | null;
    processEvidence?: boolean;
    showElsewhere?: boolean;
  } = {},
): string {
  const roster: AgentRoster = {
    sessions: [...sessions],
    needsYou: options.needsYou ?? 0,
    processEvidence: options.processEvidence ?? true,
  };
  return renderToStaticMarkup(
    createElement(SidebarAgents, {
      rows: rosterRows(runs, roster, [workstream()]),
      needsYou: roster.needsYou,
      loaded: options.loaded ?? true,
      processEvidence: roster.processEvidence,
      error: null,
      onDismissError: refuse('dismiss an error'),
      open: options.open ?? true,
      onOpenChange: refuse('fold'),
      selectedKey: options.selectedKey ?? null,
      onSelect: refuse('be selected'),
      onKeep: refuse('be kept'),
      onCancel: refuse('be cancelled'),
      onRescan: refuse('rescan'),
      busy: false,
      // The chord listener is an effect, which `renderToStaticMarkup` never
      // runs; off anyway, so the test cannot depend on that staying true.
      chords: false,
    }),
  );
}

describe('the section draws both kinds of agent', () => {
  it('shows a run Gritty owns and a session it only watches, together', () => {
    const markup = render([run()], [session()]);
    // The run, by its short handle and its workstream's title.
    expect(markup).toContain('run-123');
    expect(markup).toContain('terminal pty reflow');
    // The session, by its folder and its own task.
    expect(markup).toContain('gritty');
    expect(markup).toContain('refactor the ring');
  });

  it('marks which rows Gritty owns, because that is what gates the controls', () => {
    const markup = render([run()], [session()]);
    expect(markup).toContain('agents-section__row--owned');
    // One owned row, one not.
    expect(markup.match(/agents-section__row--owned/g)).toHaveLength(1);
  });

  it('offers Cancel on an owned run and on nothing else', () => {
    // Gritty cannot stop a session it did not start. A row that offered to
    // would be lying about the boundary the row model exists to keep.
    expect(render([run()], [])).toContain('agents-section__cancel');
    expect(render([], [session()])).not.toContain('agents-section__cancel');
  });

  it('makes every row a button, because both lanes now open a console', () => {
    // T310-U: a session has a timeline to open and a run has its event ring, so
    // there is no longer a row that looks like a list item and does nothing.
    expect(render([], [session()])).toContain('<button type="button" class="agents-section__pick"');
    expect(render([run()], [])).toContain('<button type="button" class="agents-section__pick"');
  });

  it('says the status in words, not only in a colour', () => {
    // Colour is never the only channel (T231 §10): the tone class carries the
    // token and the marker shape, and the label carries the word.
    const markup = render([run({ state: 'BLOCKED' })], [session({ activity: 'WAITING' })]);
    expect(markup).toContain('Blocked');
    expect(markup).toContain('Done');
    expect(markup).toContain('agent-status--blocked');
    expect(markup).toContain('agent-status--done');
  });

  it('draws a session only known to be alive as Alive, not Done', () => {
    // AGENT-TURN-END: no transcript and no hook, so nothing saw its turn end.
    const markup = render([], [session({ activity: 'WAITING', activitySource: 'PROCESS' })]);
    expect(markup).toContain('Alive');
    expect(markup).toContain('agent-status--alive');
    expect(markup).not.toContain('Done');
  });

  it('draws a failed run in its own tone rather than the grey of an ended one', () => {
    expect(render([run({ state: 'FAILED' })], [])).toContain('agent-status--failed');
  });

  it('badges an ask on either source with the same word', () => {
    expect(render([run({ state: 'NEEDS_DECISION' })], [])).toContain('asks');
  });
});

describe('the header', () => {
  it("reports the core's own count, not one it derived", () => {
    // Two rows, but the core says four need you: the section must publish the
    // core's number, or it and the island can disagree.
    const markup = render([run()], [session()], { needsYou: 4 });
    expect(markup).toContain('4');
    expect(markup).toContain('waiting');
  });

  it('says why every row would read as Ended when the process list is unreadable', () => {
    // The sentence that explains the roster; it lived only in the rail.
    const markup = render([], [session()], { processEvidence: false });
    expect(markup).toContain('Cannot read the process list');
  });

  it('says nothing about waiting when nothing is', () => {
    expect(render([run()], [session()], { needsYou: 0 })).not.toContain('waiting');
  });

  it('distinguishes "none" from "not looked yet"', () => {
    expect(render([], [], { loaded: false })).toContain('Looking for agents…');
    expect(render([], [], { loaded: true })).toContain('No agents here');
  });
});

describe('folded, the section still reports', () => {
  const markup = render([run({ state: 'BLOCKED' })], [session()], { open: false, needsYou: 1 });

  it('keeps a mark per agent and the waiting count', () => {
    // Artboard 11b's whole point: folding the section may never hide an
    // approval.
    expect(markup).toContain('agents-section__dot');
    expect(markup).toContain('agents-section__needs');
    expect(markup).toContain('waiting');
  });

  it('keeps the tone of each mark, so the cluster is readable', () => {
    expect(markup).toContain('agent-status--blocked');
  });

  it('drops the list itself', () => {
    expect(markup).not.toContain('agents-section__list');
  });
});

describe('agent text is text', () => {
  // Every string on a row was written by an agent or read out of a transcript,
  // and T232 §7 is the rule it inherits: never `innerHTML`, never Markdown,
  // never a resolved link or path.
  const nasty = '<script>alert(1)</script><img src=x onerror=alert(2)>';
  const markup = render(
    [run({ activity: { kind: 'EDITING', path: { display: nasty }, startedAt: 1 } })],
    [session({ title: nasty, rootName: nasty })],
  );

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
    // Hiding it would be worse: it is what the agent actually did.
    expect(markup).toContain('&lt;script&gt;');
  });
});

describe('the quiet rows are hidden, not lost', () => {
  it('keeps an ended session out of the list and behind a reveal', () => {
    // Ten dead rows above the file tree is the clutter this answers, and
    // counting them is what made the section say 12 where the top bar said 5.
    const markup = render([], [session({ activity: 'ENDED' })]);
    expect(markup).not.toContain('agents-section__list');
    expect(markup).toContain('Show 1 ended');
  });

  it('keeps a session from another workspace reachable rather than dropping it', () => {
    // The rail was machine-wide and was the only list that showed these. The
    // sidebar is scoped to this workspace, so they go behind the reveal — not
    // away, which is what filtering them out would have meant.
    const markup = render([], [session({ id: 'away', inWorkspace: false })]);
    expect(markup).toContain('Show 1 elsewhere');
  });

  it('counts both kinds separately, because they are different facts', () => {
    const markup = render(
      [],
      [session({ id: 'dead', activity: 'ENDED' }), session({ id: 'away', inWorkspace: false })],
    );
    expect(markup).toContain('Show 1 ended and 1 elsewhere');
  });

  it('never hides a row that needs you, wherever it is running', () => {
    const markup = render([], [session({ id: 'away', inWorkspace: false, activity: 'BLOCKED' })]);
    expect(markup).toContain('agents-section__list');
    expect(markup).not.toContain('Show 1 elsewhere');
  });

  it('says where a status came from, on the row that carries it', () => {
    // T221 §24. The rail's row was the only place this appeared, and the rail
    // is gone; here it rides the tooltip and the console states it in words.
    const markup = render([], [session({ activitySource: 'HOOK' })]);
    expect(markup).toContain('reported by the agent itself');
  });
});
