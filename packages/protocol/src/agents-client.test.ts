import { describe, expect, it } from 'vitest';
import { createCoreClient } from './client';
import { createFakeTransport } from './testing/fake-transport';
import type { AgentRoster, AgentSession, HealthReport } from './types';
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
