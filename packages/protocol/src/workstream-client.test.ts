import { describe, expect, it } from 'vitest';
import { createCoreClient } from './client';
import { createFakeTransport } from './testing/fake-transport';
import type {
  Decision,
  DecisionId,
  HealthReport,
  WorkspaceId,
  Workstream,
  WorkstreamId,
} from './types';
import { PROTOCOL_VERSION } from './types';

const health: HealthReport = {
  protocolVersion: PROTOCOL_VERSION,
  coreVersion: '0.1.0',
  os: 'macos',
  arch: 'aarch64',
  debug: true,
  shell: { appVersion: '0.1.0', runtime: 'tauri', runtimeVersion: '2.11.5' },
};

const workspaceId = 'ws-1' as WorkspaceId;
const workstreamId = 'wst-1' as WorkstreamId;
const decisionId = 'dec-1' as DecisionId;

const workstream: Workstream = {
  id: workstreamId,
  workspaceId,
  title: 'Terminal profiles',
  state: 'IN_PROGRESS',
  allowedTransitions: ['BLOCKED', 'IN_REVIEW', 'READY', 'ABANDONED'],
  openDecisions: 0,
  createdAt: 1,
  updatedAt: 2,
};

const decision: Decision = {
  id: decisionId,
  workstreamId,
  question: 'Where should diagnostics events be batched?',
  options: [
    { key: 'core', label: 'Core, bounded queue per document' },
    { key: 'renderer', label: 'Renderer, throttled by frame' },
  ],
  state: 'OPEN',
  raisedBy: { type: 'USER' },
  raisedAt: 3,
};

function client() {
  const transport = createFakeTransport();
  transport.handlers.set('core_health', () => health);
  return { transport, core: createCoreClient(transport) };
}

describe('workstream client', () => {
  it('wraps every workstream command in the single request object', async () => {
    const { transport, core } = client();
    transport.handlers.set('workstreams_list', () => [workstream]);
    transport.handlers.set('workstreams_create', () => workstream);
    transport.handlers.set('workstreams_update', () => workstream);
    transport.handlers.set('workstreams_transition', () => workstream);
    transport.handlers.set('workstreams_remove', () => null);

    await expect(core.workstreams.list({ workspaceId })).resolves.toEqual([workstream]);
    await expect(
      core.workstreams.create({ workspaceId, title: 'Terminal profiles' }),
    ).resolves.toEqual(workstream);
    await expect(
      core.workstreams.update({ workspaceId, workstreamId, goal: 'Pick the shell' }),
    ).resolves.toEqual(workstream);
    await expect(
      core.workstreams.transition({ workspaceId, workstreamId, to: 'IN_REVIEW' }),
    ).resolves.toEqual(workstream);
    await expect(core.workstreams.remove({ workspaceId, workstreamId })).resolves.toBeNull();

    const args = (command: string) => transport.calls.find((c) => c.command === command)?.args;
    expect(args('workstreams_list')).toEqual({ request: { workspaceId } });
    expect(args('workstreams_transition')).toEqual({
      request: { workspaceId, workstreamId, to: 'IN_REVIEW' },
    });
  });

  it('wraps every decision command in the single request object', async () => {
    const { transport, core } = client();
    transport.handlers.set('decisions_list', () => [decision]);
    transport.handlers.set('decisions_raise', () => decision);
    transport.handlers.set('decisions_resolve', () => ({
      ...decision,
      state: 'RESOLVED',
      chosen: 'core',
      resolvedAt: 9,
    }));
    transport.handlers.set('decisions_dismiss', () => ({
      ...decision,
      state: 'DISMISSED',
      resolvedAt: 9,
    }));

    await expect(
      core.decisions.list({ workspaceId, workstreamId, openOnly: true }),
    ).resolves.toEqual([decision]);
    await expect(
      core.decisions.raise({
        workspaceId,
        workstreamId,
        question: decision.question,
        options: decision.options,
      }),
    ).resolves.toEqual(decision);
    await expect(
      core.decisions.resolve({ workspaceId, decisionId, chosen: 'core' }),
    ).resolves.toMatchObject({ state: 'RESOLVED', chosen: 'core' });
    const dismissed = await core.decisions.dismiss({ workspaceId, decisionId });
    expect(dismissed.state).toBe('DISMISSED');
    expect(dismissed.chosen).toBeUndefined();

    const args = (command: string) => transport.calls.find((c) => c.command === command)?.args;
    expect(args('decisions_list')).toEqual({
      request: { workspaceId, workstreamId, openOnly: true },
    });
    expect(args('decisions_resolve')).toEqual({
      request: { workspaceId, decisionId, chosen: 'core' },
    });
  });

  it('surfaces an illegal transition as a Conflict', async () => {
    const { transport, core } = client();
    transport.handlers.set('workstreams_transition', () => {
      throw {
        code: 'Conflict',
        message: 'a workstream cannot go from DRAFT to DONE',
        retryable: false,
      };
    });
    await expect(
      core.workstreams.transition({ workspaceId, workstreamId, to: 'DONE' }),
    ).rejects.toMatchObject({ code: 'Conflict' });
  });

  it('surfaces a missing session database as a retryable Io failure', async () => {
    const { transport, core } = client();
    transport.handlers.set('workstreams_list', () => {
      throw {
        code: 'Io',
        message: 'workstreams need the session database, which is unavailable for this run',
        retryable: true,
      };
    });
    await expect(core.workstreams.list({ workspaceId })).rejects.toMatchObject({
      code: 'Io',
      retryable: true,
    });
  });
});
