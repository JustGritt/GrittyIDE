import type { AgentIntegration, AgentIntegrationChange, CoreError } from '@gritty/protocol';

/**
 * The hook integration as the settings popover holds it (T221 §25).
 *
 * Pure, so the one screen in Gritty that can write into a file the user owns is
 * tested without React and without a filesystem. There are exactly two things
 * on screen at a time — what the configuration says now, and the change that is
 * about to be made to it — and this is the state machine that decides which.
 *
 * Nothing is applied optimistically, which is T210's rule and matters more
 * here than anywhere else: a row that claimed to be installed before the core
 * had written anything would be a claim about somebody else's file.
 */
export interface AgentIntegrationState {
  /** What the core last said the configuration holds. */
  readonly status: AgentIntegration | null;
  /**
   * The change the user is being shown before it is applied, and which
   * direction it goes in. `null` while nothing is proposed.
   */
  readonly proposed: AgentIntegrationProposal | null;
  /** A command is in flight, so nothing may be started twice. */
  readonly busy: boolean;
  /** A refusal, rendered as the envelope rather than swallowed. */
  readonly error: CoreError | null;
}

export interface AgentIntegrationProposal {
  readonly direction: 'install' | 'uninstall';
  readonly change: AgentIntegrationChange;
}

export const initialAgentIntegrationState: AgentIntegrationState = {
  status: null,
  proposed: null,
  busy: false,
  error: null,
};

export type AgentIntegrationAction =
  | { readonly type: 'busy' }
  | { readonly type: 'status'; readonly status: AgentIntegration }
  | {
      readonly type: 'proposed';
      readonly direction: 'install' | 'uninstall';
      readonly change: AgentIntegrationChange;
    }
  /** The change was applied; the fresh status arrives with it. */
  | { readonly type: 'applied'; readonly status: AgentIntegration }
  | { readonly type: 'cancelled' }
  | { readonly type: 'failed'; readonly error: CoreError }
  | { readonly type: 'clearError' };

export function agentIntegrationReducer(
  state: AgentIntegrationState,
  action: AgentIntegrationAction,
): AgentIntegrationState {
  switch (action.type) {
    case 'busy':
      return { ...state, busy: true, error: null };
    case 'status':
      // A status read never disturbs a proposal the user is still looking at.
      return { ...state, status: action.status, busy: false };
    case 'proposed':
      return {
        ...state,
        proposed: { direction: action.direction, change: action.change },
        busy: false,
        error: null,
      };
    case 'applied':
      return { status: action.status, proposed: null, busy: false, error: null };
    case 'cancelled':
      return { ...state, proposed: null, busy: false, error: null };
    case 'failed':
      // The proposal goes with the refusal: what the user was shown described a
      // file the core has now told us it will not touch, so continuing to offer
      // "Apply" would offer something that cannot happen.
      return { ...state, proposed: null, busy: false, error: action.error };
    case 'clearError':
      return { ...state, error: null };
    default:
      return state;
  }
}

/** Whether the integration can be installed at all, and why not. */
export function installBlockedReason(status: AgentIntegration | null): string | null {
  if (!status) return null;
  if (!status.hookAvailable) {
    return 'This build cannot find its own hook binary, so it will not write one into your configuration.';
  }
  if (status.status === 'UNSUPPORTED_CONFIG') {
    return 'Gritty will not change a configuration file it cannot read. Fix the file and try again.';
  }
  return null;
}

/**
 * What the status line says, in the user's words.
 *
 * `BROKEN` is the one that has to explain itself: nothing is failing that the
 * user can see — the agent tolerates a hook that fails — so the only symptom is
 * that "Pending validation" quietly stopped appearing.
 */
export function integrationSummary(status: AgentIntegration): string {
  switch (status.status) {
    case 'INSTALLED':
      return `Installed, on ${status.events.length} event${status.events.length === 1 ? '' : 's'}.`;
    case 'NOT_INSTALLED':
      return 'Not installed. Agent sessions are discovered without it; a permission prompt is not.';
    case 'BROKEN':
      return 'Installed, but pointing at a binary that is not there. Repair it by installing again.';
    case 'UNSUPPORTED_CONFIG':
      return 'This configuration file cannot be read, so Gritty will not change it.';
    default:
      return status.status;
  }
}
