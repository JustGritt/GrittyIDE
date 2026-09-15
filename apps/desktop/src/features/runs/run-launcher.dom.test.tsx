/** @vitest-environment jsdom */
import {
  type AgentRun,
  type AgentRunId,
  CoreError,
  type Workstream,
  type WorkstreamId,
} from '@gritty/protocol';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { type LaunchDeps, launchRun, RunLauncher } from './run-launcher';

/**
 * The launcher as a person meets it (RUN-LAUNCHER): which keys start, what a
 * refusal leaves behind, and that a retry never makes a second workstream.
 */

function workstream(overrides: Partial<Workstream> = {}): Workstream {
  return {
    id: 'wst-1' as WorkstreamId,
    workspaceId: 'w-1' as Workstream['workspaceId'],
    title: 'existing work',
    state: 'IN_PROGRESS',
    allowedTransitions: [],
    openDecisions: 0,
    createdAt: 0,
    updatedAt: 0,
    ...overrides,
  };
}

function run(overrides: Partial<AgentRun> = {}): AgentRun {
  return {
    id: 'run-1' as AgentRunId,
    workstreamId: 'wst-new' as WorkstreamId,
    workspaceId: 'w-1' as AgentRun['workspaceId'],
    provider: 'CLAUDE_CODE',
    state: 'STARTING',
    startedAt: 0,
    updatedAt: 0,
    cancellationRequested: false,
    correlationId: 'c1' as AgentRun['correlationId'],
    ...overrides,
  };
}

const refusal = new CoreError({
  code: 'NotFound',
  message: 'claude is not on PATH',
  retryable: false,
});

function deps(overrides: Partial<LaunchDeps> = {}): LaunchDeps {
  return {
    workstreams: [],
    busy: false,
    createWorkstream: vi.fn(async (title: string) =>
      workstream({ id: 'wst-new' as WorkstreamId, title }),
    ),
    startRun: vi.fn(async (workstreamId: WorkstreamId) => run({ workstreamId })),
    workstreamError: null,
    runError: null,
    clearWorkstreamError: vi.fn(),
    clearRunError: vi.fn(),
    ...overrides,
  };
}

function mount(given: LaunchDeps, handlers: { onStarted?: () => void; onClose?: () => void } = {}) {
  const onStarted = handlers.onStarted ?? vi.fn();
  const onClose = handlers.onClose ?? vi.fn();
  const view = render(<RunLauncher deps={given} onStarted={onStarted} onClose={onClose} />);
  const box = screen.getByRole('textbox') as HTMLTextAreaElement;
  const choice = screen.getByRole('combobox') as HTMLSelectElement;
  return { view, box, choice, onStarted, onClose };
}

async function startWith(box: HTMLTextAreaElement, text: string) {
  fireEvent.change(box, { target: { value: text } });
  await act(async () => {
    fireEvent.keyDown(box, { key: 'Enter', metaKey: true });
  });
}

afterEach(() => {
  cleanup();
});

describe('launchRun', () => {
  it('creates a workstream titled from the prompt, then starts against it', async () => {
    const given = deps();
    const outcome = await launchRun(
      { prompt: 'Fix the watcher\nand more', workstreamId: null },
      given,
    );
    expect(given.createWorkstream).toHaveBeenCalledWith('Fix the watcher');
    expect(given.startRun).toHaveBeenCalledWith('wst-new', 'Fix the watcher\nand more');
    expect(outcome.kind).toBe('started');
  });

  it('starts against a chosen workstream without creating one', async () => {
    const given = deps();
    await launchRun({ prompt: 'go', workstreamId: 'wst-1' as WorkstreamId }, given);
    expect(given.createWorkstream).not.toHaveBeenCalled();
    expect(given.startRun).toHaveBeenCalledWith('wst-1', 'go');
  });

  it('starts nothing when the workstream is refused', async () => {
    const given = deps({ createWorkstream: vi.fn(async () => undefined) });
    const outcome = await launchRun({ prompt: 'go', workstreamId: null }, given);
    expect(given.startRun).not.toHaveBeenCalled();
    expect(outcome).toEqual({ kind: 'refused', step: 'workstream', created: null });
  });

  it('reports the workstream it made when the run is refused', async () => {
    const given = deps({ startRun: vi.fn(async () => undefined) });
    const outcome = await launchRun({ prompt: 'go', workstreamId: null }, given);
    expect(outcome).toEqual({ kind: 'refused', step: 'run', created: 'wst-new' });
  });
});

describe('the form', () => {
  it('defaults to a new workstream and lists only work that is not over', () => {
    const { choice } = mount(
      deps({
        workstreams: [
          workstream(),
          workstream({ id: 'wst-done' as WorkstreamId, title: 'shipped', state: 'DONE' }),
          workstream({ id: 'wst-gone' as WorkstreamId, title: 'dropped', state: 'ABANDONED' }),
        ],
      }),
    );
    expect(choice.value).toBe('');
    const labels = [...choice.options].map((option) => option.textContent);
    expect(labels).toEqual(['New workstream from this prompt', 'existing work']);
  });

  it('holds Start until there is something to start, and while a command is in flight', () => {
    const { box } = mount(deps());
    const start = screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement;
    expect(start.disabled).toBe(true);
    fireEvent.change(box, { target: { value: 'go' } });
    expect(start.disabled).toBe(false);
    cleanup();
    const busy = mount(deps({ busy: true }));
    fireEvent.change(busy.box, { target: { value: 'go' } });
    expect((screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement).disabled).toBe(
      true,
    );
  });

  it('says what starting spends and where it acts', () => {
    mount(deps());
    expect(screen.getByText(/on your own Claude account/)).toBeTruthy();
    expect(screen.getByText(/ask you here first/)).toBeTruthy();
  });

  it('starts on ⌘↵ with the trimmed prompt, and hands the run up', async () => {
    const given = deps();
    const { box, onStarted } = mount(given);
    await startWith(box, '  Fix the watcher  ');
    expect(given.startRun).toHaveBeenCalledWith('wst-new', 'Fix the watcher');
    expect(onStarted).toHaveBeenCalledWith(expect.objectContaining({ id: 'run-1' }));
    expect(box.value).toBe('');
  });

  it('starts on ⌃↵ too, and leaves a bare ↵ to make a newline', async () => {
    const given = deps();
    const { box } = mount(given);
    fireEvent.change(box, { target: { value: 'first line' } });
    fireEvent.keyDown(box, { key: 'Enter' });
    expect(given.createWorkstream).not.toHaveBeenCalled();
    await act(async () => {
      fireEvent.keyDown(box, { key: 'Enter', ctrlKey: true });
    });
    expect(given.startRun).toHaveBeenCalledOnce();
  });

  it('closes on Escape without starting anything', () => {
    const given = deps();
    const { box, onClose } = mount(given);
    fireEvent.change(box, { target: { value: 'go' } });
    fireEvent.keyDown(box, { key: 'Escape' });
    expect(onClose).toHaveBeenCalledOnce();
    expect(given.createWorkstream).not.toHaveBeenCalled();
  });

  it('keeps the prompt and shows the refusal when the workstream is refused', async () => {
    const given = deps({
      createWorkstream: vi.fn(async () => undefined),
      workstreamError: refusal,
    });
    const { box, onStarted } = mount(given);
    await startWith(box, 'go');
    expect(onStarted).not.toHaveBeenCalled();
    expect(box.value).toBe('go');
    expect(screen.getByRole('alert').textContent).toContain('claude is not on PATH');
  });

  it('points a retry at the workstream it already made when the run is refused', async () => {
    const startRun = vi.fn(async () => undefined as AgentRun | undefined);
    const created = workstream({ id: 'wst-new' as WorkstreamId, title: 'go' });
    const given = deps({ startRun, runError: refusal });
    const { view, box, choice, onStarted } = mount(given);
    await startWith(box, 'go');
    expect(given.createWorkstream).toHaveBeenCalledOnce();
    expect(screen.getByRole('alert').textContent).toContain('claude is not on PATH');
    expect(box.value).toBe('go');

    // The created workstream reaches the list the way the controller delivers
    // it, and the choice has already moved onto it.
    const retry = deps({ ...given, workstreams: [created], startRun, runError: null });
    startRun.mockImplementation(async () => run());
    view.rerender(<RunLauncher deps={retry} onStarted={onStarted} onClose={vi.fn()} />);
    expect(choice.value).toBe('wst-new');

    await act(async () => {
      fireEvent.keyDown(box, { key: 'Enter', metaKey: true });
    });
    expect(given.createWorkstream).toHaveBeenCalledOnce();
    expect(startRun).toHaveBeenLastCalledWith('wst-new', 'go');
    expect(onStarted).toHaveBeenCalledOnce();
  });
});
