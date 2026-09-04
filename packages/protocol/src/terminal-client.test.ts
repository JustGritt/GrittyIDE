import { describe, expect, it, vi } from 'vitest';
import { createCoreClient } from './client';
import { createFakeTransport } from './testing/fake-transport';
import type {
  EventOf,
  HealthReport,
  Terminal,
  TerminalId,
  TerminalProfile,
  WorkspaceId,
  WorkspacePath,
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

const terminalId = 'term-1' as TerminalId;
const workspaceId = 'ws-1' as WorkspaceId;

const profile: TerminalProfile = {
  path: '/bin/zsh',
  name: 'zsh',
  isDefault: true,
  isCustom: false,
  available: true,
};

const terminal: Terminal = {
  id: terminalId,
  workspaceId,
  cwd: '' as WorkspacePath,
  shell: '$SHELL',
  pid: 4242,
  status: 'RUNNING',
  cols: 80,
  rows: 24,
  createdAt: 1,
};

function connected() {
  const transport = createFakeTransport();
  transport.handlers.set('core_health', () => health);
  return transport;
}

describe('terminal client', () => {
  it('sends every command as a single request object', async () => {
    const transport = connected();
    transport.handlers.set('terminal_spawn', () => terminal);
    transport.handlers.set('terminal_write', () => null);
    transport.handlers.set('terminal_resize', () => ({ ...terminal, cols: 100, rows: 30 }));
    transport.handlers.set('terminal_rename', () => ({ ...terminal, name: 'claude' }));
    transport.handlers.set('terminal_kill', () => terminal);
    transport.handlers.set('terminal_list', () => [terminal]);
    transport.handlers.set('terminal_profiles', () => [profile]);
    const client = createCoreClient(transport);

    await expect(client.terminal.spawn({ workspaceId })).resolves.toEqual(terminal);
    await client.terminal.write({ terminalId, data: 'ls\n' });
    await expect(
      client.terminal.resize({ terminalId, cols: 100, rows: 30 }),
    ).resolves.toMatchObject({ cols: 100, rows: 30 });
    await expect(client.terminal.rename({ terminalId, name: 'claude' })).resolves.toHaveProperty(
      'name',
      'claude',
    );
    await expect(client.terminal.kill({ terminalId })).resolves.toEqual(terminal);
    await expect(client.terminal.list()).resolves.toEqual([terminal]);
    await expect(client.terminal.profiles({ custom: ['/bin/fish'] })).resolves.toEqual([profile]);

    expect(transport.calls.filter((call) => call.command.startsWith('terminal_'))).toEqual([
      { command: 'terminal_spawn', args: { request: { workspaceId } } },
      { command: 'terminal_write', args: { request: { terminalId, data: 'ls\n' } } },
      { command: 'terminal_resize', args: { request: { terminalId, cols: 100, rows: 30 } } },
      { command: 'terminal_rename', args: { request: { terminalId, name: 'claude' } } },
      { command: 'terminal_kill', args: { request: { terminalId } } },
      // `list` takes no arguments.
      { command: 'terminal_list', args: undefined },
      { command: 'terminal_profiles', args: { request: { custom: ['/bin/fish'] } } },
    ]);
  });

  it('forwards control characters verbatim', async () => {
    const transport = connected();
    transport.handlers.set('terminal_write', () => null);
    const client = createCoreClient(transport);

    // Ctrl+C. The shell interprets it, so it must reach the core unescaped.
    await client.terminal.write({ terminalId, data: '\u0003' });
    const call = transport.calls.find((c) => c.command === 'terminal_write');
    expect(call?.args).toEqual({ request: { terminalId, data: '\u0003' } });
  });

  it('delivers output and exit events to their own subscribers', async () => {
    const transport = connected();
    const client = createCoreClient(transport);
    const onOutput = vi.fn();
    const onExit = vi.fn();
    client.subscribe('TerminalOutput', onOutput);
    client.subscribe('TerminalExited', onExit);
    await client.health();

    transport.emit({
      name: 'TerminalOutput',
      seq: 1,
      actor: { type: 'SYSTEM' },
      payload: { terminalId, data: 'hello\r\n', truncatedBytes: 0 },
    });
    transport.emit({
      name: 'TerminalExited',
      seq: 2,
      actor: { type: 'SYSTEM' },
      payload: {
        terminal: { ...terminal, status: 'EXITED', pid: undefined, exitCode: 0 },
        code: 0,
      },
    });

    expect(onOutput).toHaveBeenCalledTimes(1);
    const output = onOutput.mock.calls[0]?.[0] as EventOf<'TerminalOutput'> | undefined;
    expect(output?.payload.data).toBe('hello\r\n');
    expect(onExit).toHaveBeenCalledTimes(1);
    const exit = onExit.mock.calls[0]?.[0] as EventOf<'TerminalExited'> | undefined;
    expect(exit?.payload.terminal.status).toBe('EXITED');
    expect(exit?.payload.code).toBe(0);
  });

  it('reports truncated output so a gap is never presented as continuous', async () => {
    const transport = connected();
    const client = createCoreClient(transport);
    const onOutput = vi.fn();
    client.subscribe('TerminalOutput', onOutput);
    await client.health();

    transport.emit({
      name: 'TerminalOutput',
      seq: 1,
      actor: { type: 'SYSTEM' },
      payload: { terminalId, data: 'tail of the flood', truncatedBytes: 4096 },
    });

    const event = onOutput.mock.calls[0]?.[0] as EventOf<'TerminalOutput'> | undefined;
    expect(event?.payload.truncatedBytes).toBe(4096);
  });

  it('surfaces a core rejection as a CoreError', async () => {
    const transport = connected();
    transport.handlers.set('terminal_write', () => {
      throw {
        code: 'NotFound',
        message: 'terminal term-9 does not exist',
        retryable: false,
      };
    });
    const client = createCoreClient(transport);

    await expect(
      client.terminal.write({ terminalId: 'term-9' as TerminalId, data: 'x' }),
    ).rejects.toMatchObject({ code: 'NotFound' });
  });
});
