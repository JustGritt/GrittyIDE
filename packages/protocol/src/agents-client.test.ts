import { describe, expect, it } from 'vitest';
import { createCoreClient } from './client';
import { createFakeTransport } from './testing/fake-transport';
import type {
  AgentIntegration,
  AgentIntegrationChange,
  AgentRoster,
  AgentSession,
  HealthReport,
} from './types';
import { CORE_EVENT_CHANNEL, PROTOCOL_VERSION } from './types';

const health: HealthReport = {
  protocolVersion: PROTOCOL_VERSION,
  coreVersion: '0.1.0',
  os: 'macos',
  arch: 'aarch64',
  debug: true,
  shell: { appVersion: '0.1.0', runtime: 'tauri', runtimeVersion: '2.11.5' },
};

const session: AgentSession = {
  id: 'claude_code:s-1',
  provider: 'CLAUDE_CODE',
  activity: 'WORKING',
  activitySource: 'TRANSCRIPT',
  evidence: 'BOTH',
  title: 'Terminal profiles',
  root: '~/code/gritty',
  rootName: 'gritty',
  inWorkspace: true,
  tool: { name: 'Bash', detail: 'cargo test' },
  pid: 42,
  tty: '/dev/ttys001',
  lastActiveAt: 1_800_000_000_000,
};

const roster: AgentRoster = { sessions: [session], needsYou: 0, processEvidence: true };

function client() {
  const transport = createFakeTransport();
  transport.handlers.set('core_health', () => health);
  transport.handlers.set('agents_list', () => roster);
  transport.handlers.set('agents_refresh', () => roster);
  return { transport, core: createCoreClient(transport) };
}

describe('agents client', () => {
  it('lists the roster through the typed command name', async () => {
    const { transport, core } = client();

    await expect(core.agents.list()).resolves.toEqual(roster);

    // `agents.list` maps to `agents_list`, and the request travels as one
    // `request` object even though it is empty.
    const call = transport.calls.find((entry) => entry.command === 'agents_list');
    expect(call).toBeDefined();
    expect(call?.args).toEqual({ request: {} });
  });

  it('refreshes on demand', async () => {
    const { transport, core } = client();

    await expect(core.agents.refresh()).resolves.toEqual(roster);
    expect(transport.calls.some((entry) => entry.command === 'agents_refresh')).toBe(true);
  });

  it('delivers the roster event on the single core channel', async () => {
    const { transport, core } = client();
    const seen: AgentRoster[] = [];

    core.subscribe('AgentSessionsChanged', (event) => seen.push(event.payload));
    await Promise.resolve();

    transport.emit({
      name: 'AgentSessionsChanged',
      payload: roster,
      seq: 7,
      actor: { type: 'SYSTEM' },
    });

    expect(seen).toEqual([roster]);
    expect(CORE_EVENT_CHANNEL).toBe('gritty://core/event');
  });

  it('carries a session with no transcript without inventing fields', async () => {
    const transport = createFakeTransport();
    const bare: AgentSession = {
      id: 'claude_code:pid-42',
      provider: 'CLAUDE_CODE',
      activity: 'WAITING',
      activitySource: 'PROCESS',
      evidence: 'PROCESS_ONLY',
      inWorkspace: false,
      pid: 42,
    };
    transport.handlers.set('core_health', () => health);
    transport.handlers.set('agents_list', () => ({
      sessions: [bare],
      needsYou: 1,
      processEvidence: true,
    }));

    const core = createCoreClient(transport);
    const result = await core.agents.list();
    const [only] = result.sessions;

    expect(only).toBeDefined();
    expect(only?.title).toBeUndefined();
    expect(only?.lastActiveAt).toBeUndefined();
    expect(result.needsYou).toBe(1);
  });
});

describe('agents integration client (T221)', () => {
  const notInstalled: AgentIntegration = {
    provider: 'CLAUDE_CODE',
    status: 'NOT_INSTALLED',
    configPath: '~/.claude/settings.json',
    events: [],
    hookAvailable: true,
  };

  const plan: AgentIntegrationChange = {
    provider: 'CLAUDE_CODE',
    applied: false,
    configPath: '~/.claude/settings.json',
    backupPath: '~/.claude/settings.json.gritty-backup-1800000000000',
    createsConfig: false,
    command: "'/Applications/Gritty.app/Contents/MacOS/gritty-agent-hook' --provider claude_code",
    events: [
      { event: 'PermissionRequest', foreignEntries: 1 },
      { event: 'Stop', foreignEntries: 1 },
    ],
    status: 'INSTALLED',
  };

  function integrationClient() {
    const transport = createFakeTransport();
    transport.handlers.set('core_health', () => health);
    transport.handlers.set('agents_integration_status', () => notInstalled);
    transport.handlers.set('agents_integration_install', (args) => ({
      ...plan,
      applied: (args as { request: { apply: boolean } }).request.apply,
    }));
    transport.handlers.set('agents_integration_uninstall', () => ({
      ...plan,
      applied: true,
      status: 'NOT_INSTALLED',
    }));
    return { transport, core: createCoreClient(transport) };
  }

  it('reads the status through the typed command name', async () => {
    const { transport, core } = integrationClient();

    await expect(core.agents.integrationStatus({ provider: 'CLAUDE_CODE' })).resolves.toEqual(
      notInstalled,
    );
    const call = transport.calls.find((entry) => entry.command === 'agents_integration_status');
    expect(call?.args).toEqual({ request: { provider: 'CLAUDE_CODE' } });
  });

  it('previews the change without applying it', async () => {
    const { core } = integrationClient();

    const preview = await core.agents.integrationInstall({
      provider: 'CLAUDE_CODE',
      apply: false,
    });

    // The same shape the apply returns, so the UI shows the diff it is about
    // to make rather than a description of one (ADR-018 §3).
    expect(preview.applied).toBe(false);
    expect(preview.backupPath).toBeDefined();
    expect(preview.events.map((event) => event.event)).toEqual(['PermissionRequest', 'Stop']);
    expect(preview.events.every((event) => event.foreignEntries === 1)).toBe(true);
  });

  it('applies and reverses', async () => {
    const { core } = integrationClient();

    await expect(
      core.agents.integrationInstall({ provider: 'CLAUDE_CODE', apply: true }),
    ).resolves.toMatchObject({ applied: true, status: 'INSTALLED' });
    await expect(
      core.agents.integrationUninstall({ provider: 'CLAUDE_CODE', apply: true }),
    ).resolves.toMatchObject({ applied: true, status: 'NOT_INSTALLED' });
  });

  it('carries a pending permission prompt on the timeline', async () => {
    const transport = createFakeTransport();
    transport.handlers.set('core_health', () => health);
    transport.handlers.set('agents_timeline', () => ({
      sessionId: 'claude_code:s-1',
      entries: [],
      dropped: 0,
      restarted: false,
      transcriptEvidence: true,
      pending: {
        seq: Number.MAX_SAFE_INTEGER,
        sidechain: false,
        kind: { type: 'PERMISSION_PENDING', tool: 'Bash', detail: 'cargo test' },
      },
    }));

    const core = createCoreClient(transport);
    const timeline = await core.agents.timeline({ sessionId: 'claude_code:s-1' });

    expect(timeline.pending?.kind).toEqual({
      type: 'PERMISSION_PENDING',
      tool: 'Bash',
      detail: 'cargo test',
    });
  });
});
