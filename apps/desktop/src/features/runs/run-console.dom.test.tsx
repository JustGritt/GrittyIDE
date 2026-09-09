/** @vitest-environment jsdom */
import type { AgentRun, AgentRunId } from '@gritty/protocol';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { RunConsole } from './run-console';
import type { RunLog } from './runs-reducer';
import type { RunsController } from './use-runs';

/**
 * The prompt box, as a person actually meets it (T310-U §9).
 *
 * A DOM rather than static markup, because what is asserted here does not exist
 * in a string: *which* keystroke sends, whether the draft survives, and what a
 * disabled box does when someone types into it anyway. The shape of the markup
 * is `run-console-render.test.ts`'s.
 */

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

const emptyLog: RunLog = { events: [], dropped: 0 };

function controller(overrides: Partial<RunsController> = {}): RunsController {
  return {
    runs: [],
    loaded: true,
    error: null,
    clearError: () => {},
    busy: false,
    startFake: async () => {},
    startClaude: async () => {},
    prompt: async () => {},
    cancel: async () => {},
    openEvents: async () => true,
    logFor: () => emptyLog,
    ...overrides,
  } as RunsController;
}

function show(overrides: Partial<AgentRun> = {}, runs = controller()) {
  render(
    <RunConsole run={run(overrides)} title="Ship the ring" controller={runs} pending={false} />,
  );
  return screen.getByRole('textbox');
}

afterEach(() => {
  cleanup();
});

describe('sending a turn', () => {
  it('sends on ⌘↵ and clears the box', () => {
    const prompt = vi.fn(async () => {});
    const box = show({}, controller({ prompt }));
    fireEvent.change(box, { target: { value: '  run the tests  ' } });
    fireEvent.keyDown(box, { key: 'Enter', metaKey: true });
    // Trimmed, because trailing whitespace is not something anyone means to
    // pay for; the full text otherwise reaches the child untouched.
    expect(prompt).toHaveBeenCalledWith('run-1', 'run the tests');
    expect((box as HTMLTextAreaElement).value).toBe('');
  });

  it('sends on ⌃↵ too, for a keyboard without a ⌘', () => {
    const prompt = vi.fn(async () => {});
    const box = show({}, controller({ prompt }));
    fireEvent.change(box, { target: { value: 'again' } });
    fireEvent.keyDown(box, { key: 'Enter', ctrlKey: true });
    expect(prompt).toHaveBeenCalledOnce();
  });

  it('leaves a bare ↵ to make a newline', () => {
    // A prompt is prose and often more than one line. Sending on ↵ would make a
    // paragraph impossible to type, and cost money every time someone reached
    // for one.
    const prompt = vi.fn(async () => {});
    const box = show({}, controller({ prompt }));
    fireEvent.change(box, { target: { value: 'first line' } });
    fireEvent.keyDown(box, { key: 'Enter' });
    expect(prompt).not.toHaveBeenCalled();
    expect((box as HTMLTextAreaElement).value).toBe('first line');
  });

  it('sends nothing for whitespace, and does not clear what is there', () => {
    const prompt = vi.fn(async () => {});
    const box = show({}, controller({ prompt }));
    fireEvent.change(box, { target: { value: '   ' } });
    fireEvent.keyDown(box, { key: 'Enter', metaKey: true });
    expect(prompt).not.toHaveBeenCalled();
    expect((box as HTMLTextAreaElement).value).toBe('   ');
  });

  it('draws nothing in the feed on the keystroke', () => {
    // Nothing optimistic (§9): the user's own words appear when the run
    // confirms them with `PROMPT_STARTED`, never on the way out.
    const box = show({}, controller({ prompt: async () => {} }));
    fireEvent.change(box, { target: { value: 'run the tests' } });
    fireEvent.keyDown(box, { key: 'Enter', metaKey: true });
    expect(screen.queryByText('run the tests')).toBeNull();
    expect(screen.getByText(/Nothing reported yet/)).toBeTruthy();
  });

  it('holds the Send button until there is something to send', () => {
    const box = show();
    const send = screen.getByRole('button', { name: 'Send' });
    expect((send as HTMLButtonElement).disabled).toBe(true);
    fireEvent.change(box, { target: { value: 'go' } });
    expect((send as HTMLButtonElement).disabled).toBe(false);
  });

  it('holds it again while a command is already in flight', () => {
    render(
      <RunConsole
        run={run()}
        title="Ship the ring"
        controller={controller({ busy: true })}
        pending={false}
      />,
    );
    fireEvent.change(screen.getByRole('textbox'), { target: { value: 'go' } });
    expect((screen.getByRole('button', { name: 'Send' }) as HTMLButtonElement).disabled).toBe(true);
  });
});

describe('a run that is not accepting input', () => {
  it('disables the box and says why, without taking the draft away', () => {
    const prompt = vi.fn(async () => {});
    const runs = controller({ prompt });
    const { rerender } = render(
      <RunConsole run={run()} title="Ship the ring" controller={runs} pending={false} />,
    );
    const box = screen.getByRole('textbox') as HTMLTextAreaElement;
    fireEvent.change(box, { target: { value: 'half a thought' } });

    // The run goes BLOCKED while someone is mid-sentence. It must not also take
    // their sentence with it.
    rerender(
      <RunConsole
        run={run({ state: 'BLOCKED' })}
        title="Ship the ring"
        controller={runs}
        pending={false}
      />,
    );
    expect(box.disabled).toBe(true);
    expect(box.value).toBe('half a thought');
    expect(screen.getByText(/a prompt does not answer/)).toBeTruthy();

    // And the chord cannot get past the gate either.
    fireEvent.keyDown(box, { key: 'Enter', metaKey: true });
    expect(prompt).not.toHaveBeenCalled();
  });

  it('offers no box at all on a scripted run', () => {
    render(
      <RunConsole
        run={run({ provider: 'FAKE' })}
        title="Ship the ring"
        controller={controller()}
        pending={false}
      />,
    );
    expect(screen.queryByRole('textbox')).toBeNull();
    expect(screen.queryByRole('button', { name: 'Send' })).toBeNull();
  });
});
