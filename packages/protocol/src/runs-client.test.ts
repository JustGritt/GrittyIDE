import { describe, expect, it } from 'vitest';
import { createCoreClient } from './client';
import { COMMANDS } from './commands';
import { createFakeTransport } from './testing/fake-transport';
import type { AgentRun, AgentRunEventLog, CoreErrorShape, HealthReport } from './types';
import { PROTOCOL_VERSION } from './types';

// T300-CMD, §16: the four `runs.*` commands. `runs-events.test.ts` covers the
// two event families T300-C shipped; this covers the command surface that
// reaches them — the names on the wire, the request each carries, and the
// failures the panel has to be able to draw.

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
  state: 'STARTING',
  startedAt: 1_800_000_000_000,
  updatedAt: 1_800_000_000_000,
  cancellationRequested: false,
  correlationId: 'corr-1' as AgentRun['correlationId'],
};

const log: AgentRunEventLog = {
  runId: run.id,
  events: [{ seq: 1, runId: run.id, at: 1_800_000_000_001, event: { type: 'RUN_STARTED' } }],
  dropped: 0,
};

function client() {
  const transport = createFakeTransport();
  transport.handlers.set('core_health', () => health);
  transport.handlers.set('runs_start_fake', () => run);
  transport.handlers.set('runs_cancel', () => ({
    ...run,
    state: 'CANCELLED',
    cancellationRequested: true,
    finishedAt: 1_800_000_000_500,
  }));
  transport.handlers.set('runs_list', () => [run]);
  transport.handlers.set('runs_events', () => log);
  return { transport, core: createCoreClient(transport) };
}

/** The one call that is not `core_health`. */
function commandCall(transport: ReturnType<typeof createFakeTransport>) {
  return transport.calls.find((call) => call.command !== 'core_health');
}

describe('runs client', () => {
  it('names every runs command in the table, under the naming rule', () => {
    expect(COMMANDS['runs.startFake']).toBe('runs_start_fake');
    expect(COMMANDS['runs.cancel']).toBe('runs_cancel');
    expect(COMMANDS['runs.list']).toBe('runs_list');
    expect(COMMANDS['runs.events']).toBe('runs_events');
  });

  it('starts a fake run, passing the workspace, the workstream and the script', async () => {
    const { transport, core } = client();
    const started = await core.runs.startFake({
      workspaceId: run.workspaceId,
      workstreamId: run.workstreamId,
      script: 'permission-then-decision',
    });
    expect(started.provider).toBe('FAKE');
    // STARTING is distinct from RUNNING on purpose: a terminal-backed adapter
    // waits out shell readiness here, and the state exists now so T310 does
    // not invent it later.
    expect(started.state).toBe('STARTING');
    expect(commandCall(transport)).toEqual({
      command: 'runs_start_fake',
      args: {
        request: {
          workspaceId: 'w-1',
          workstreamId: 'wst-1',
          script: 'permission-then-decision',
        },
      },
    });
  });

  it('omits the script rather than sending null when the default is wanted', async () => {
    const { transport, core } = client();
    await core.runs.startFake({ workspaceId: run.workspaceId, workstreamId: run.workstreamId });
    const request = commandCall(transport)?.args?.request as Record<string, unknown>;
    expect('script' in request).toBe(false);
  });

  it('cancels by run id and returns the snapshot the core answered with', async () => {
    const { transport, core } = client();
    const cancelled = await core.runs.cancel({ runId: run.id });
    expect(cancelled.state).toBe('CANCELLED');
    expect(cancelled.cancellationRequested).toBe(true);
    expect(commandCall(transport)).toEqual({
      command: 'runs_cancel',
      args: { request: { runId: 'run-1' } },
    });
  });

  it('lists a workspace’s runs', async () => {
    const { transport, core } = client();
    const runs = await core.runs.list({ workspaceId: run.workspaceId });
    expect(runs.map((entry) => entry.id)).toEqual(['run-1']);
    expect(commandCall(transport)).toEqual({
      command: 'runs_list',
      args: { request: { workspaceId: 'w-1' } },
    });
  });

  it('rehydrates one run’s event log, carrying the dropped count', async () => {
    const { transport, core } = client();
    const answered = await core.runs.events({ runId: run.id });
    expect(answered.events.map((entry) => entry.event.type)).toEqual(['RUN_STARTED']);
    expect(answered.dropped).toBe(0);
    expect(commandCall(transport)).toEqual({
      command: 'runs_events',
      args: { request: { runId: 'run-1' } },
    });
  });

  it('surfaces a refusal as the error envelope, not as a thrown string', async () => {
    const { transport, core } = client();
    // `MAX_CONCURRENT_RUNS` already reached: the core refuses and names the
    // count, and the panel draws that rather than a silent no-op.
    transport.handlers.set('runs_start_fake', () => {
      throw {
        code: 'Conflict',
        message: '4 runs are already active in this workspace',
        retryable: false,
      } satisfies CoreErrorShape;
    });
    await expect(
      core.runs.startFake({ workspaceId: run.workspaceId, workstreamId: run.workstreamId }),
    ).rejects.toMatchObject({ code: 'Conflict', retryable: false });
  });

  it('surfaces an unknown run as NotFound', async () => {
    const { transport, core } = client();
    transport.handlers.set('runs_cancel', () => {
      throw {
        code: 'NotFound',
        message: 'no run run-9',
        retryable: false,
      } satisfies CoreErrorShape;
    });
    await expect(core.runs.cancel({ runId: 'run-9' as AgentRun['id'] })).rejects.toMatchObject({
      code: 'NotFound',
    });
  });

  it('checks protocol compatibility before any runs command reaches the core', async () => {
    const transport = createFakeTransport();
    transport.handlers.set('core_health', () => ({ ...health, protocolVersion: '3.0.0' }));
    transport.handlers.set('runs_list', () => [run]);
    const core = createCoreClient(transport);
    await expect(core.runs.list({ workspaceId: run.workspaceId })).rejects.toMatchObject({
      code: 'ProtocolMismatch',
    });
    expect(transport.calls.some((call) => call.command === 'runs_list')).toBe(false);
  });
});
