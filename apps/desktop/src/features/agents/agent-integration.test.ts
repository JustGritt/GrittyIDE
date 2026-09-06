import { type AgentIntegration, type AgentIntegrationChange, CoreError } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import {
  type AgentIntegrationState,
  agentIntegrationReducer,
  initialAgentIntegrationState,
  installBlockedReason,
  integrationSummary,
} from './agent-integration';

const notInstalled: AgentIntegration = {
  provider: 'CLAUDE_CODE',
  status: 'NOT_INSTALLED',
  configPath: '~/.claude/settings.json',
  events: [],
  hookAvailable: true,
};

const installed: AgentIntegration = {
  ...notInstalled,
  status: 'INSTALLED',
  events: ['PermissionRequest', 'Stop'],
};

const change: AgentIntegrationChange = {
  provider: 'CLAUDE_CODE',
  applied: false,
  configPath: '~/.claude/settings.json',
  backupPath: '~/.claude/settings.json.gritty-backup-1800000000000',
  createsConfig: false,
  command: "'/Applications/Gritty.app/Contents/MacOS/gritty-agent-hook' --provider claude_code",
  events: [{ event: 'PermissionRequest', foreignEntries: 1 }],
  status: 'INSTALLED',
};

const refused = new CoreError({
  code: 'Conflict',
  message: '~/.claude/settings.json could not be understood: expected value at line 1 column 3',
  retryable: false,
});

function reduce(
  state: AgentIntegrationState,
  ...actions: Parameters<typeof agentIntegrationReducer>[1][]
): AgentIntegrationState {
  return actions.reduce(agentIntegrationReducer, state);
}

describe('agentIntegrationReducer', () => {
  it('shows the change before it is applied, and applies nothing on its own', () => {
    const proposed = reduce(
      initialAgentIntegrationState,
      { type: 'busy' },
      { type: 'status', status: notInstalled },
      { type: 'busy' },
      { type: 'proposed', direction: 'install', change },
    );

    expect(proposed.busy).toBe(false);
    expect(proposed.proposed?.direction).toBe('install');
    expect(proposed.proposed?.change.applied).toBe(false);
    expect(proposed.status?.status).toBe('NOT_INSTALLED');
  });

  it('takes the fresh status when the change lands, not the one it asked for', () => {
    const applied = reduce(
      initialAgentIntegrationState,
      { type: 'status', status: notInstalled },
      { type: 'proposed', direction: 'install', change },
      { type: 'applied', status: installed },
    );

    expect(applied.status).toEqual(installed);
    expect(applied.proposed).toBeNull();
    expect(applied.busy).toBe(false);
  });

  it('drops the proposal when the core refuses, because it can no longer happen', () => {
    const failed = reduce(
      initialAgentIntegrationState,
      { type: 'status', status: notInstalled },
      { type: 'proposed', direction: 'install', change },
      { type: 'failed', error: refused },
    );

    expect(failed.proposed).toBeNull();
    expect(failed.error).toEqual(refused);
    expect(failed.status).toEqual(notInstalled);

    expect(agentIntegrationReducer(failed, { type: 'clearError' }).error).toBeNull();
  });

  it('cancelling leaves the configuration exactly where it was', () => {
    const cancelled = reduce(
      initialAgentIntegrationState,
      { type: 'status', status: installed },
      { type: 'proposed', direction: 'uninstall', change },
      { type: 'cancelled' },
    );

    expect(cancelled.proposed).toBeNull();
    expect(cancelled.status).toEqual(installed);
  });
});

describe('installBlockedReason', () => {
  it('refuses to write an entry that would be broken the moment it landed', () => {
    expect(installBlockedReason({ ...notInstalled, hookAvailable: false })).toContain(
      'hook binary',
    );
  });

  it('refuses a configuration it cannot read', () => {
    expect(installBlockedReason({ ...notInstalled, status: 'UNSUPPORTED_CONFIG' })).toContain(
      'cannot read',
    );
  });

  it('allows the ordinary case, and says nothing before the status is known', () => {
    expect(installBlockedReason(notInstalled)).toBeNull();
    expect(installBlockedReason(installed)).toBeNull();
    expect(installBlockedReason(null)).toBeNull();
  });
});

describe('integrationSummary', () => {
  it('explains every status, including the one with no visible symptom', () => {
    expect(integrationSummary(installed)).toContain('2 events');
    expect(integrationSummary(notInstalled)).toContain('Not installed');
    // BROKEN is invisible from the outside — the agent tolerates a failing
    // hook — so this line is the only thing that says why BLOCKED stopped.
    expect(integrationSummary({ ...installed, status: 'BROKEN' })).toContain('not there');
    expect(integrationSummary({ ...notInstalled, status: 'UNSUPPORTED_CONFIG' })).toContain(
      'will not change it',
    );
  });
});
