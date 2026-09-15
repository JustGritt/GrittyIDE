import type { AgentRun, CoreError, Workstream, WorkstreamId } from '@gritty/protocol';
import { type KeyboardEvent, useEffect, useId, useRef, useState } from 'react';
import { ErrorEnvelope } from '../../components/error-envelope';
import {
  canLaunch,
  type LaunchRequest,
  MAX_PROMPT,
  NEW_WORKSTREAM,
  titleFromPrompt,
} from './run-launcher-model';

/**
 * What the launcher may do, narrowed from the two controllers `App` holds.
 *
 * Narrowed for the reason `ConsoleRuns` is: this form starts a run and makes a
 * workstream to hold it, and a type that let it prompt, cancel or transition
 * something could.
 */
export interface LaunchDeps {
  /** The workspace's workstreams, for the "belongs to" choice. */
  readonly workstreams: readonly Workstream[];
  /** Either controller has a command in flight. */
  readonly busy: boolean;
  createWorkstream(title: string): Promise<Workstream | undefined>;
  startRun(workstreamId: WorkstreamId, prompt: string): Promise<AgentRun | undefined>;
  /** The controllers' own last refusals; the launcher shows the one its step hit. */
  readonly workstreamError: CoreError | null;
  readonly runError: CoreError | null;
  clearWorkstreamError(): void;
  clearRunError(): void;
}

export type LaunchOutcome =
  | { readonly kind: 'started'; readonly run: AgentRun }
  | {
      readonly kind: 'refused';
      readonly step: 'workstream' | 'run';
      /** A workstream this attempt created before the run was refused. */
      readonly created: WorkstreamId | null;
    };

/**
 * Make the workstream if one was asked for, then start the run against it.
 *
 * Two commands and no transaction, so a refusal of the second leaves the first
 * done. That is reported rather than undone: the workstream is real, visible on
 * the island, and the caller points its retry at it instead of making another.
 */
export async function launchRun(
  request: LaunchRequest,
  deps: Pick<LaunchDeps, 'createWorkstream' | 'startRun'>,
): Promise<LaunchOutcome> {
  let workstreamId = request.workstreamId;
  let created: WorkstreamId | null = null;
  if (workstreamId === null) {
    const workstream = await deps.createWorkstream(titleFromPrompt(request.prompt));
    if (!workstream) return { kind: 'refused', step: 'workstream', created: null };
    workstreamId = workstream.id;
    created = workstream.id;
  }
  const run = await deps.startRun(workstreamId, request.prompt);
  if (!run) return { kind: 'refused', step: 'run', created };
  return { kind: 'started', run };
}

/** A workstream that is over is not somewhere new work should be started. */
function acceptsRuns(workstream: Workstream): boolean {
  return workstream.state !== 'DONE' && workstream.state !== 'ABANDONED';
}

/**
 * Start a Claude run from the sidebar (RUN-LAUNCHER, ADR-035 M0).
 *
 * The prompt box follows the run console's rules, so the two places a person
 * types to Claude behave alike: ⌘↵ or ⌃↵ starts, a bare ↵ is a newline, and
 * whitespace sends nothing. **Nothing optimistic**: the form closes when the
 * core has started the run, and the run's row and console come from the core's
 * answer rather than from this form.
 *
 * A refusal keeps what was typed, beside the envelope of the step that refused.
 */
export function RunLauncher({
  deps,
  onStarted,
  onClose,
}: {
  deps: LaunchDeps;
  /** The run exists; the caller opens its console and folds the form away. */
  onStarted(run: AgentRun): void;
  onClose(): void;
}) {
  const [prompt, setPrompt] = useState('');
  const [choice, setChoice] = useState<string>(NEW_WORKSTREAM);
  const [failed, setFailed] = useState<'workstream' | 'run' | null>(null);
  const [launching, setLaunching] = useState(false);
  const promptId = useId();
  const choiceId = useId();
  const box = useRef<HTMLTextAreaElement>(null);

  // Opened to type into, so the caret starts there.
  useEffect(() => {
    box.current?.focus();
  }, []);

  const open = deps.workstreams.filter(acceptsRuns);
  // A chosen workstream that has since ended or been removed falls back to a
  // new one rather than leaving the select naming something not in its list.
  const selected = open.some((workstream) => workstream.id === choice) ? choice : NEW_WORKSTREAM;
  const startable = canLaunch(prompt) && !deps.busy && !launching;
  const error =
    failed === 'workstream' ? deps.workstreamError : failed === 'run' ? deps.runError : null;

  async function start() {
    if (!startable) return;
    setLaunching(true);
    setFailed(null);
    try {
      const outcome = await launchRun(
        {
          prompt: prompt.trim(),
          workstreamId: selected === NEW_WORKSTREAM ? null : (selected as WorkstreamId),
        },
        deps,
      );
      if (outcome.kind === 'started') {
        setPrompt('');
        setChoice(NEW_WORKSTREAM);
        onStarted(outcome.run);
        return;
      }
      setFailed(outcome.step);
      if (outcome.created !== null) setChoice(outcome.created);
    } finally {
      setLaunching(false);
    }
  }

  function onKeyDown(event: KeyboardEvent<HTMLFormElement>) {
    if (event.key === 'Escape') {
      event.preventDefault();
      onClose();
      return;
    }
    if (event.key !== 'Enter' || !(event.metaKey || event.ctrlKey)) return;
    if (!(event.target instanceof HTMLTextAreaElement)) return;
    event.preventDefault();
    void start();
  }

  return (
    <form
      className="agent-prompt run-launcher"
      aria-label="Start a Claude run"
      onKeyDown={onKeyDown}
      onSubmit={(event) => {
        event.preventDefault();
        void start();
      }}
    >
      <label className="agent-prompt__label" htmlFor={promptId}>
        What should Claude do?
      </label>
      <textarea
        id={promptId}
        ref={box}
        className="agent-prompt__input"
        value={prompt}
        onChange={(event) => setPrompt(event.target.value)}
        maxLength={MAX_PROMPT}
        rows={4}
        spellCheck={true}
        placeholder="Describe the task… (⌘↵ to start)"
      />
      <label className="agent-prompt__label" htmlFor={choiceId}>
        Belongs to
      </label>
      <select
        id={choiceId}
        className="run-launcher__workstream"
        value={selected}
        onChange={(event) => setChoice(event.target.value)}
      >
        <option value={NEW_WORKSTREAM}>New workstream from this prompt</option>
        {open.map((workstream) => (
          <option key={workstream.id} value={workstream.id}>
            {workstream.title}
          </option>
        ))}
      </select>
      {/* What pressing Start does, in words, before it does it: it spends the
          user's own Claude account, and it acts in this workspace. */}
      <p className="run-launcher__note">
        Runs Claude Code in this workspace on your own Claude account. Tools your Claude settings do
        not already allow ask you here first.
      </p>
      {error && (
        <ErrorEnvelope
          error={error}
          onDismiss={() => {
            if (failed === 'workstream') deps.clearWorkstreamError();
            else deps.clearRunError();
            setFailed(null);
          }}
        />
      )}
      <div className="agent-prompt__foot">
        <span className="agent-console__spacer" />
        <button type="button" className="btn--bare" onClick={onClose}>
          Cancel
        </button>
        <button type="submit" className="btn--primary" disabled={!startable}>
          {launching ? 'Starting…' : 'Start'}
        </button>
      </div>
    </form>
  );
}
