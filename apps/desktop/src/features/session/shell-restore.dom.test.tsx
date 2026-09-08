/** @vitest-environment jsdom */
import type {
  AgentNotificationSettings,
  AgentPermissionGrants,
  AgentRoster,
  AgentRun,
  Decision,
  Document,
  HealthReport,
  LastSession,
  ListDirectoryResponse,
  OwnershipMap,
  RecentWorkspace,
  RecoveryCandidate,
  SessionRestoreResponse,
  Terminal,
  TerminalProfile,
  Workspace,
  WorkspaceId,
  WorkspacePath,
  Workstream,
  WorkstreamAgentLink,
} from '@gritty/protocol';
import { PROTOCOL_VERSION } from '@gritty/protocol';
import { cleanup, render, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from '../../App';
import { resetSessionRestore } from './use-session';

/**
 * The launcher-versus-workspace decision, rendered (JSDOM-VITEST).
 *
 * The regression this exists for shipped in PR #33: the shell showed the
 * startup launcher *while a workspace was open*. `session.restore` reports the
 * workspace it reopened in its own response, but the shell used to wait for a
 * `WorkspaceOpened` event to say it again — and the core opens that workspace
 * while the WebView is still registering its listener, so the event is emitted
 * to nobody. `runRestore` now calls `deps.adoptWorkspace(response.workspace)`.
 *
 * That fix had no direct test because nothing here could render a component,
 * which is the ticket this file closes. The two cases below are a pair on
 * purpose: one proves the launcher goes away, the other proves it still
 * appears when it should, so the first cannot pass because the launcher never
 * rendered at all.
 *
 * Deliberately faithful to the race: `workspace.current` answers `null` (the
 * core has not reopened anything at the moment the explorer asks) and
 * `WorkspaceOpened` is never emitted. The *only* route from core to shell is
 * the restore response. Delete the `adoptWorkspace` call in `restore-flow.ts`
 * and the first test fails.
 */

// ---------------------------------------------------------------------------
// jsdom gaps that Monaco and xterm hit. All four are APIs jsdom simply does not
// implement; none of them is behaviour under test.
//
// Inside `vi.hoisted` because Monaco touches `document.queryCommandSupported`
// at *import* time, and `import { App }` above runs before this module's body
// would. Hoisting is what puts the shims in front of it.
// ---------------------------------------------------------------------------
vi.hoisted(() => {
  Object.assign(document, { queryCommandSupported: () => false });
  globalThis.ResizeObserver ??= class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
  window.matchMedia ??= ((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener() {},
    removeEventListener() {},
    addListener() {},
    removeListener() {},
    dispatchEvent: () => false,
  })) as typeof window.matchMedia;
  HTMLCanvasElement.prototype.getContext = (() => null) as HTMLCanvasElement['getContext'];
});

// ---------------------------------------------------------------------------
// The core seam. `apps/desktop/src/core/client.ts` is a module singleton built
// on the Tauri transport, so it is replaced rather than reached around; the
// shell's hooks then talk to a transport this test drives.
//
// `createFakeTransport` from `packages/protocol/src/testing` would be the
// natural thing to reuse, but the package's `exports` map only publishes `.`
// and the index does not re-export it, so it is not reachable from
// `apps/desktop` without editing `packages/protocol` (recorded as a follow-up).
// ---------------------------------------------------------------------------
const seam = vi.hoisted(() => {
  const handlers = new Map<string, (args?: Record<string, unknown>) => unknown>();
  const listeners = new Set<(payload: unknown) => void>();
  return {
    handlers,
    /**
     * Exposed so a test can assert the shell *did* subscribe. Nothing is ever
     * emitted through it: that is the whole point of the first case.
     */
    listeners,
    transport: {
      async invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
        const handler = handlers.get(command);
        // A command this test did not stub is a fact worth seeing, not a
        // silent empty answer that could make an assertion pass vacuously.
        if (!handler) {
          throw { code: 'Internal', message: `unstubbed command ${command}`, retryable: false };
        }
        return (await handler(args)) as T;
      },
      async listen(_channel: string, handler: (payload: unknown) => void) {
        listeners.add(handler);
        return () => {
          listeners.delete(handler);
        };
      },
    },
  };
});

vi.mock('../../core/client', async () => {
  const { createCoreClient } = await import('@gritty/protocol');
  return { core: createCoreClient(seam.transport) };
});

const WORKSPACE: Workspace = {
  id: 'ws-1' as WorkspaceId,
  root: '/tmp/gritty-fixture',
  name: 'gritty-fixture',
  openedAt: 1,
};

function restoreResponse(workspace: Workspace | null): SessionRestoreResponse {
  const response: SessionRestoreResponse = {
    tabs: [],
    persistence: { available: true, healthy: true },
    notices: [],
  };
  if (workspace) response.workspace = workspace;
  return response;
}

/**
 * Everything the shell asks for on startup, answered as an empty-but-healthy
 * core. Every handler declares its generated return type so a stub that drifts
 * from the wire contract is a typecheck failure rather than a render crash —
 * the first draft of this file returned `[]` for `ownership.list` and took
 * `AgentIsland` down with it.
 */
function stubCore(restored: Workspace | null): void {
  const h = seam.handlers;
  h.set(
    'core_health',
    (): HealthReport => ({
      protocolVersion: PROTOCOL_VERSION,
      coreVersion: '0.1.0',
      os: 'macos',
      arch: 'aarch64',
      debug: true,
      shell: { appVersion: '0.1.0', runtime: 'tauri', runtimeVersion: '2.11.5' },
    }),
  );
  h.set('session_restore', (): SessionRestoreResponse => restoreResponse(restored));
  // The race, stated: at the moment the explorer asks, nothing is open yet.
  // The restore is what reopens the workspace.
  h.set('workspace_current', (): Workspace | null => null);
  h.set(
    'workspace_list',
    (): ListDirectoryResponse => ({
      status: 'ready',
      page: { path: '' as WorkspacePath, entries: [], total: 0, truncated: false },
    }),
  );
  h.set('workspace_recents', (): RecentWorkspace[] => []);
  h.set('documents_list', (): Document[] => []);
  h.set('session_candidates', (): RecoveryCandidate[] => []);
  h.set('session_last_session', (): LastSession | null => null);
  h.set('session_update', (): void => undefined);
  h.set('terminal_list', (): Terminal[] => []);
  h.set('terminal_profiles', (): TerminalProfile[] => []);
  h.set('workstreams_list', (): Workstream[] => []);
  h.set('workstreams_agent_links', (): WorkstreamAgentLink[] => []);
  h.set('agents_list', (): AgentRoster => ({ sessions: [], needsYou: 0, processEvidence: true }));
  h.set(
    'agents_notifications',
    (): AgentNotificationSettings => ({
      enabled: false,
      supported: false,
      permission: 'PROMPT',
    }),
  );
  h.set('agents_permission_grants', (): AgentPermissionGrants => ({ grants: [] }));
  h.set('decisions_list', (): Decision[] => []);
  // `useRuns` is called from `App` for the sidebar's Agents section (T233-S),
  // so the shell asks for this on every startup now.
  h.set('runs_list', (): AgentRun[] => []);
  h.set('ownership_list', (): OwnershipMap => ({ leases: [], conflicts: [] }));
}

/**
 * Queries are scoped to this render's own container rather than to `screen`
 * (which is `document.body`): if one case fails mid-flight its DOM can outlive
 * the assertion, and a body-wide query would then see two shells.
 */
function renderShell() {
  const { container } = render(<App />);
  return { container, ui: within(container) };
}

beforeEach(() => {
  // The one piece of module-level state the shell keeps across mounts: the
  // single-flight startup restore. `App` is imported statically on purpose —
  // importing it inside a test would charge Monaco's several-second module
  // load to that test's timeout, which is exactly how this file first went
  // red under a parallel `vitest run`.
  resetSessionRestore();
  seam.handlers.clear();
  // `seam.listeners` is deliberately NOT cleared. It holds the registration
  // state of a client that lives as long as this module, not per-test state:
  // since EVENT-LISTEN-GAP the hub registers once, when the client is
  // constructed, instead of on the first `subscribe`. Clearing it here would
  // discard a listener that nothing is going to register again.
});

afterEach(() => {
  cleanup();
});

describe('the shell after a startup restore', () => {
  it('shows the workspace, not the launcher, when the restore reopened one', async () => {
    stubCore(WORKSPACE);
    const { container, ui } = renderShell();

    // The workspace UI is up... (T100-U: the sidebar header became a real
    // tablist once the Changes tab had somewhere to go, so the Files section
    // is a `role="tab"` rather than a heading. The assertion follows the
    // change the way T233-S's followed the rename before it.)
    expect(await ui.findByRole('tab', { name: 'Files' })).toBeTruthy();
    expect(ui.getByText(WORKSPACE.name)).toBeTruthy();
    expect(ui.getByTitle(WORKSPACE.root)).toBeTruthy();
    // ...including the Agents section above the tree (T233-S). Asserted here
    // rather than left implicit: this is the only test that mounts the real
    // `App`, so it is the only place that shows the section survives being
    // composed into the shell rather than only rendering in isolation.
    expect(container.querySelector('.agents-section')).not.toBeNull();
    // ...and the Changes tab beside Files (T100-U), for the same reason: the
    // render tests next door cannot show that the tab survives composition
    // into the shell, and a correct component with no caller is the defect
    // this repository keeps catching late.
    expect(ui.getByRole('tab', { name: 'Changes' })).toBeTruthy();
    // ...and the rail it replaced is gone (T233-C, ADR-026). Asserted rather
    // than assumed: this is the only test that mounts the real `App`, so it is
    // the only place that would notice the two surfaces coexisting again.
    expect(container.querySelector('.agent-rail')).toBeNull();
    // ...and the launcher is not, even though `WorkspaceOpened` never arrived.
    expect(container.querySelector('#launcher-title')).toBeNull();
    expect(container.querySelector('.launcher')).toBeNull();
    expect(ui.queryByText('No workspace open')).toBeNull();
    // The channel really is open — this shell is not merely deaf, the event
    // simply never carried the workspace, which is the situation the fix
    // exists for. Note this proves the *client's* one listener is live rather
    // than that any particular hook subscribed: the hub multiplexes every
    // subscription onto a single transport listener, so this set never holds
    // more than one however many hooks are mounted.
    expect(seam.listeners.size).toBeGreaterThan(0);
  });

  it('shows the launcher when the restore reopened nothing', async () => {
    stubCore(null);
    const { container, ui } = renderShell();

    // The control for the test above: same harness, no workspace in the
    // restore response, and the launcher must appear.
    await waitFor(() => {
      expect(container.querySelector('#launcher-title')).not.toBeNull();
    });
    expect(ui.getByText('No workspace open')).toBeTruthy();
    expect(ui.queryByRole('tab', { name: 'Files' })).toBeNull();
  });
});
