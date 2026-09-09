import type { AgentRun, AgentRunEvent, AgentRunId, AgentRunState } from '@gritty/protocol';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { RunConsole } from './run-console';
import type { RunLog } from './runs-reducer';
import type { RunsController } from './use-runs';

/**
 * What an owned run's console actually puts on the screen (T310-U).
 *
 * Static markup rather than a DOM, matching `agent-console-render.test.ts`:
 * `apps/desktop`'s vitest project is a node one, and a string is a stronger
 * assertion than a query against a tree. The half that genuinely needs a
 * browser — ⌘↵, and what a submit does — is `run-console.dom.test.tsx`.
 */

/**
 * The tags, with their attribute *values* blanked.
 *
 * The same scan `agent-console-render.test.ts` runs, for the same reason:
 * escaped agent text inside an attribute is the *safe* outcome, and scanning
 * raw tags would flag it as a false positive. The assertions are about which
 * attributes exist, not what is in them.
 */
function tagsOf(markup: string): readonly string[] {
  return (markup.match(/<[^>]*>/g) ?? []).map((tag) => tag.replace(/"[^"]*"/g, '""'));
}

const refuse = (what: string) => () => {
  throw new Error(`nothing should ${what} while rendering`);
};

function run(overrides: Partial<AgentRun> = {}): AgentRun {
  return {
    id: 'run-1' as AgentRunId,
    workstreamId: 'w1' as AgentRun['workstreamId'],
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

function controllerFor(log: RunLog): RunsController {
  return {
    runs: [],
    loaded: true,
    error: null,
    clearError: () => {},
    busy: false,
    startFake: refuse('start a fake run'),
    startClaude: refuse('start a run'),
    prompt: refuse('send a prompt'),
    cancel: refuse('cancel'),
    openEvents: refuse('fetch events'),
    logFor: () => log,
  } as unknown as RunsController;
}

function events(...list: readonly AgentRunEvent[]): RunLog {
  return {
    events: list.map((event, index) => ({
      seq: index + 1,
      runId: 'run-1' as AgentRunId,
      at: 0,
      event,
    })),
    dropped: 0,
  };
}

function render(
  overrides: Partial<AgentRun> = {},
  log: RunLog = { events: [], dropped: 0 },
  { title = 'Ship the ring', pending = false }: { title?: string | null; pending?: boolean } = {},
): string {
  return renderToStaticMarkup(
    createElement(RunConsole, {
      run: run(overrides),
      title,
      controller: controllerFor(log),
      pending,
    }),
  );
}

describe('the prompt box (T310-U §9)', () => {
  it('is a textarea below the feed, on a run that is accepting input', () => {
    const markup = render(
      { state: 'RUNNING' },
      events({ type: 'PROMPT_FINISHED', text: 'they pass' }),
    );
    expect(markup).toContain('<textarea');
    // Below the feed (§9), not above it: a box that pushed the newest thing the
    // agent said off the screen would be the wrong way round.
    expect(markup.indexOf('agent-feed')).toBeLessThan(markup.indexOf('agent-prompt'));
  });

  it('is disabled with the reason beside it, never disabled in silence', () => {
    // A greyed-out box that says nothing is indistinguishable from a broken one.
    const markup = render({ state: 'BLOCKED' });
    expect(markup).toContain('disabled=""');
    expect(markup).toContain('a prompt does not answer');
  });

  it('names the state in the way for each one that refuses', () => {
    expect(render({ state: 'STARTING' })).toContain('still starting');
    expect(render({ state: 'REVIEW' })).toContain('your turn');
    expect(render({ state: 'FINISHED' })).toContain('has ended');
    expect(render({ state: 'RUNNING', cancellationRequested: true })).toContain('is stopping');
  });

  it('is absent entirely on a scripted run, not drawn disabled', () => {
    // There is no channel and no waiting will open one, so a control that could
    // be enabled must not be implied (§9's own rule for the other lane).
    const markup = render({ provider: 'FAKE' });
    expect(markup).not.toContain('<textarea');
    expect(markup).not.toContain('agent-prompt');
  });

  it('draws nothing the user typed: the feed is the run confirming it', () => {
    // Nothing optimistic. What was typed reaches the feed when `PROMPT_STARTED`
    // arrives, which is what `--replay-user-messages` is passed for — so an
    // empty ring shows an empty feed even with the box on screen.
    const markup = render({ state: 'RUNNING' });
    expect(markup).toContain('Nothing reported yet');
    expect(markup).toContain('<textarea');
  });
});

describe('the feed comes from the ring (T310-U §6)', () => {
  it('says "opening" while the events are on their way, and "nothing yet" after', () => {
    // Three states that are not the same thing: not asked yet, asked and
    // waiting, and asked and empty.
    expect(render({}, { events: [], dropped: 0 }, { pending: true })).toContain('Opening…');
    expect(render({}, { events: [], dropped: 0 })).toContain('Nothing reported yet');
  });

  it('draws the conversation the run reported', () => {
    const markup = render(
      {},
      events(
        { type: 'PROMPT_STARTED', text: 'run the tests' },
        { type: 'PROMPT_FINISHED', text: 'they pass' },
      ),
    );
    expect(markup).toContain('run the tests');
    expect(markup).toContain('they pass');
  });

  it('draws a gap the core reported rather than splicing around it', () => {
    const markup = render({}, { events: [], dropped: 3 });
    expect(markup).toContain('3 earlier events dropped');
  });

  it('shows how the run itself broke, which is not a feed row', () => {
    // `RUN_FAILED` may have fallen out of the ring; the snapshot's error is the
    // durable answer to "how did it end".
    const markup = render({
      state: 'FAILED',
      error: { code: 'Internal', message: 'the pipe closed', retryable: false },
    });
    expect(markup).toContain('the pipe closed');
  });
});

describe('the header says which run, and how it is doing', () => {
  it('draws the status in words as well as in a tone', () => {
    const markup = render({ state: 'BLOCKED' });
    expect(markup).toContain('Blocked');
    expect(markup).toContain('agent-status--blocked');
  });

  it('attributes the state to Gritty rather than to a source of evidence', () => {
    // The session console says which of three disagreeing sources produced its
    // verdict (T221 §24). A run's state is a transition the core accepted, so
    // there is no source to attribute — and saying so is not the same as
    // leaving the line blank.
    expect(render()).toContain('Gritty started this run');
  });

  it('titles the console with the workstream, because that is the work', () => {
    expect(render({}, { events: [], dropped: 0 }, { title: 'Ship the ring' })).toContain(
      'Ship the ring',
    );
    expect(render({}, { events: [], dropped: 0 }, { title: null })).toContain(
      'No workstream title',
    );
  });

  it('draws a state from a newer core as it arrived instead of a blank row', () => {
    expect(render({ state: 'ASCENDED' as AgentRunState })).toContain('ASCENDED');
  });
});

describe('everything the agent said is text and nothing else (hard rule 13)', () => {
  const markup = render(
    {},
    events(
      { type: 'PROMPT_FINISHED', text: '<script>alert(1)</script>' },
      { type: 'COMMAND_STARTED', command: '<img src=x onerror=alert(1)>' },
      { type: 'FILE_WRITTEN', path: { display: '<a href="javascript:alert(1)">x</a>' } },
    ),
  );

  it('puts nothing an agent wrote into a tag', () => {
    for (const tag of tagsOf(markup)) {
      expect(tag).not.toMatch(/<script/i);
      expect(tag).not.toMatch(/\shref/i);
      expect(tag).not.toMatch(/\ssrc=/i);
      expect(tag).not.toMatch(/\son[a-z]+=/i);
    }
  });

  it('still shows what the agent wrote, escaped', () => {
    expect(markup).toContain('&lt;script&gt;');
  });
});
