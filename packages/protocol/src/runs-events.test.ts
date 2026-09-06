import { describe, expect, it } from 'vitest';
import { createCoreClient } from './client';
import { createFakeTransport } from './testing/fake-transport';
import type {
  AgentRun,
  AgentRunChangedPayload,
  AgentRunEventAppendedPayload,
  AgentRunEventRecord,
  EventOf,
  HealthReport,
} from './types';
import { PROTOCOL_VERSION } from './types';

// T300-C ships the contract, not the client: there are no `runs.*` commands to
// exercise yet (they are T300-CMD's, §16). What can be pinned down now — and
// what the three tickets behind this gate build against — is the wire itself:
// the two event families, the shape of what they carry, and the fact that a
// consumer can tell one run's events from another's.

const health: HealthReport = {
  protocolVersion: PROTOCOL_VERSION,
  coreVersion: '0.1.0',
  os: 'macos',
  arch: 'aarch64',
  debug: true,
  shell: { appVersion: '0.1.0', runtime: 'tauri', runtimeVersion: '2.11.5' },
};

const run: AgentRun = {
  id: 'run-1' as AgentRun['id'],
  workstreamId: 'wst-1' as AgentRun['workstreamId'],
  workspaceId: 'w-1' as AgentRun['workspaceId'],
  provider: 'FAKE',
  state: 'RUNNING',
  activity: {
    kind: 'EDITING',
    // Inside the workspace, so `path` is present and the renderer may open it.
    path: { path: 'src/auth/session.ts', display: 'src/auth/session.ts' },
    startedAt: 1_800_000_000_100,
  },
  startedAt: 1_800_000_000_000,
  updatedAt: 1_800_000_000_100,
  cancellationRequested: false,
  correlationId: 'corr-1' as AgentRun['correlationId'],
};

function record(seq: number, runId: AgentRun['id']): AgentRunEventRecord {
  return {
    seq,
    runId,
    at: 1_800_000_000_000 + seq,
    event: { type: 'FILE_WRITTEN', path: { display: '~/elsewhere/notes.md' } },
  };
}

function envelope(seq: number, name: string, payload: unknown) {
  return { name, seq, workspaceId: 'w-1', actor: { type: 'USER' }, payload };
}

function client() {
  const transport = createFakeTransport();
  transport.handlers.set('core_health', () => health);
  return { transport, core: createCoreClient(transport) };
}

describe('agent run events', () => {
  it('delivers a whole run on AgentRunChanged and a delta on AgentRunEventAppended', async () => {
    const { transport, core } = client();
    const snapshots: AgentRunChangedPayload[] = [];
    const deltas: AgentRunEventAppendedPayload[] = [];
    core.subscribe('AgentRunChanged', (event: EventOf<'AgentRunChanged'>) => {
      snapshots.push(event.payload);
    });
    core.subscribe('AgentRunEventAppended', (event: EventOf<'AgentRunEventAppended'>) => {
      deltas.push(event.payload);
    });

    transport.emit(envelope(1, 'AgentRunChanged', { run }));
    transport.emit(
      envelope(2, 'AgentRunEventAppended', {
        runId: run.id,
        events: [record(1, run.id), record(2, run.id)],
        dropped: 0,
      }),
    );

    // The snapshot is the whole run — state and activity together, which is how
    // a UI writes "RUNNING / Editing src/auth/session.ts" without a second call.
    expect(snapshots).toHaveLength(1);
    expect(snapshots[0]?.run.state).toBe('RUNNING');
    expect(snapshots[0]?.run.activity?.kind).toBe('EDITING');

    // The delta is a batch, never one event per call (rule 8).
    expect(deltas).toHaveLength(1);
    expect(deltas[0]?.events.map((entry) => entry.seq)).toEqual([1, 2]);
    expect(deltas[0]?.dropped).toBe(0);
  });

  it('separates one run’s events from another’s, and reports what the ring dropped', async () => {
    const { transport, core } = client();
    const byRun = new Map<string, AgentRunEventRecord[]>();
    let dropped = 0;
    core.subscribe('AgentRunEventAppended', (event) => {
      const existing = byRun.get(event.payload.runId) ?? [];
      byRun.set(event.payload.runId, [...existing, ...event.payload.events]);
      dropped = event.payload.dropped;
    });

    const other = 'run-2' as AgentRun['id'];
    transport.emit(
      envelope(1, 'AgentRunEventAppended', {
        runId: run.id,
        events: [record(1, run.id)],
        dropped: 0,
      }),
    );
    transport.emit(
      envelope(2, 'AgentRunEventAppended', {
        runId: other,
        events: [record(1, other)],
        // A gap the UI is told about rather than left to splice over.
        dropped: 4,
      }),
    );

    expect([...byRun.keys()]).toEqual(['run-1', 'run-2']);
    expect(byRun.get('run-1')).toHaveLength(1);
    expect(dropped).toBe(4);
  });

  it('narrows a run event by its tag without a cast', async () => {
    const { transport, core } = client();
    const written: string[] = [];
    core.subscribe('AgentRunEventAppended', (event) => {
      for (const entry of event.payload.events) {
        // The tagged union narrows: `path` exists on FILE_WRITTEN and on
        // nothing that does not carry one.
        if (entry.event.type === 'FILE_WRITTEN') written.push(entry.event.path.display);
        if (entry.event.type === 'COMMAND_FINISHED') written.push(String(entry.event.exitCode));
      }
    });

    transport.emit(
      envelope(1, 'AgentRunEventAppended', {
        runId: run.id,
        events: [
          record(1, run.id),
          {
            seq: 2,
            runId: run.id,
            at: 1_800_000_000_002,
            event: { type: 'COMMAND_FINISHED', command: 'cargo test', ok: false, exitCode: 101 },
          },
        ],
        dropped: 0,
      }),
    );

    // A path outside the workspace arrives with `display` and no `path`: it is
    // shown and never opened (ADR-010).
    expect(written).toEqual(['~/elsewhere/notes.md', '101']);
  });
});
