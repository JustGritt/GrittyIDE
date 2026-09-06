import type { AgentRun, AgentRunId, Workspace, Workstream } from '@gritty/protocol';
import { type FormEvent, useState } from 'react';
import { ErrorEnvelope } from '../../components/error-envelope';
import { describeEvent, describeRun, isTerminalRun, type RunLog } from './runs-reducer';
import { type RunsController, useRuns } from './use-runs';

/**
 * Development-only control surface for runs Gritty owns (T300-CMD, §17):
 * start a scripted run against a workstream, watch it through its states,
 * cancel it, and read the events it reported.
 *
 * Deliberately the throwaway version, and it is worth saying why rather than
 * leaving the next reader to wonder. A run is attached to a workstream, so the
 * surface it eventually deserves is a second line on that workstream's island
 * row — `RUNNING · Editing src/auth/session.ts` — feeding the same "needs you"
 * count the island already keeps, not a fourth roster beside the island, the
 * agent rail and the attention queue. That is a designed change and it has no
 * ticket yet; building it here would be taking scope rather than recording it
 * (rule 14), so this panel exists to make the manual test possible and for
 * nothing else. It sits beside `OperationsPanel` for the same reason that one
 * does: a debug affordance earns a collapsed row, not a panel.
 *
 * Every string it draws is text the core already bounded and collapsed. None
 * is rendered as markup, none is resolved to a path, and none is run.
 */
export function RunsPanel({
  workspace,
  workstreams,
}: {
  workspace: Workspace | null;
  workstreams: readonly Workstream[];
}) {
  const runs = useRuns(workspace);
  return (
    <details className="panel">
      <summary>Agent runs (development)</summary>
      <div className="panel__body">
        <StartFakeForm workstreams={workstreams} runs={runs} />
        {runs.error && <ErrorEnvelope error={runs.error} onDismiss={runs.clearError} />}
        {!runs.loaded ? (
          <p className="panel__empty">Loading runs…</p>
        ) : runs.runs.length === 0 ? (
          <p className="panel__empty">No runs yet.</p>
        ) : (
          <ul className="ops">
            {runs.runs.map((run) => (
              <RunRow key={run.id} run={run} controller={runs} />
            ))}
          </ul>
        )}
      </div>
    </details>
  );
}

function StartFakeForm({
  workstreams,
  runs,
}: {
  workstreams: readonly Workstream[];
  runs: RunsController;
}) {
  const [workstreamId, setWorkstreamId] = useState('');
  const [script, setScript] = useState('');
  const target = workstreamId || workstreams[0]?.id;

  function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!target) return;
    // Nothing optimistic: the row appears when the core says the run exists.
    void runs.startFake(target as Workstream['id'], script.trim() || undefined);
  }

  if (workstreams.length === 0) {
    return <p className="panel__empty">A run needs a workstream. Create one first.</p>;
  }

  return (
    <form className="panel__form" onSubmit={onSubmit}>
      <label>
        workstream
        <select value={target ?? ''} onChange={(e) => setWorkstreamId(e.target.value)}>
          {workstreams.map((workstream) => (
            <option key={workstream.id} value={workstream.id}>
              {workstream.title}
            </option>
          ))}
        </select>
      </label>
      <label>
        script
        <input
          value={script}
          onChange={(e) => setScript(e.target.value)}
          placeholder="default"
          spellCheck={false}
        />
      </label>
      <button type="submit" disabled={runs.busy || !target}>
        Start fake run
      </button>
    </form>
  );
}

function RunRow({ run, controller }: { run: AgentRun; controller: RunsController }) {
  const terminal = isTerminalRun(run.state);
  const log = controller.logFor(run.id);
  return (
    <li className={`ops__row ops__row--${run.state.toLowerCase()}`}>
      <div className="ops__head">
        <span className="ops__kind">{run.provider}</span>
        <span className="ops__status">
          {describeRun(run)}
          {run.cancellationRequested && !terminal ? ' (cancelling)' : ''}
        </span>
        <code className="ops__id">{run.id.slice(0, 8)}</code>
        {!terminal && (
          <button
            type="button"
            onClick={() => void controller.cancel(run.id)}
            disabled={run.cancellationRequested || controller.busy}
          >
            Cancel
          </button>
        )}
      </div>
      {run.error && (
        <p className="ops__error">
          {run.error.code}: {run.error.message}
        </p>
      )}
      <RunEvents runId={run.id} log={log} onOpen={controller.openEvents} />
      <p className="ops__meta">
        started {formatTime(run.startedAt)}
        {run.finishedAt !== undefined ? `, finished ${formatTime(run.finishedAt)}` : ''}
        {' · workstream '}
        {run.workstreamId.slice(0, 8)}
        {' · correlation '}
        {run.correlationId.slice(0, 8)}
      </p>
    </li>
  );
}

function RunEvents({
  runId,
  log,
  onOpen,
}: {
  runId: AgentRunId;
  log: RunLog;
  onOpen: (runId: AgentRunId) => Promise<void>;
}) {
  const [opened, setOpened] = useState(false);
  return (
    <details
      className="runs__events"
      onToggle={(event) => {
        // Fetched once, on first open: after that the appended events keep it
        // current, and asking again would only re-send what is already here.
        if (!event.currentTarget.open || opened) return;
        setOpened(true);
        void onOpen(runId);
      }}
    >
      <summary>
        events ({log.events.length}
        {log.dropped > 0 ? `, ${log.dropped} dropped` : ''})
      </summary>
      {log.dropped > 0 && (
        // The core reported a gap; it is drawn, never repaired. Splicing the
        // events either side together would invent a continuity the core
        // deliberately refused to claim.
        <p className="runs__gap">{log.dropped} earlier events dropped from this run's ring.</p>
      )}
      {log.events.length === 0 ? (
        <p className="panel__empty">Nothing reported yet.</p>
      ) : (
        <ol className="runs__event-list">
          {log.events.map((record) => (
            <li key={record.seq} className="runs__event">
              <span className="runs__event-at">{formatTime(record.at)}</span>{' '}
              {describeEvent(record.event)}
            </li>
          ))}
        </ol>
      )}
    </details>
  );
}

function formatTime(millis: number): string {
  return new Date(millis).toLocaleTimeString();
}
