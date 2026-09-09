import type { CoreError, HealthReport, Workspace } from '@gritty/protocol';
import { type ReactNode, useCallback, useEffect, useRef, useState } from 'react';
import { ErrorEnvelope } from '../../components/error-envelope';
import { describeHealth } from '../../core/health';
import { QuickOpenBar } from '../quick-open/quick-open-bar';
import { type QuickOpenDeps, useQuickOpen } from '../quick-open/use-quick-open';
import type { Accelerator } from './open-shortcuts';
import { initialQueryFor, type QuickOpenShortcut } from './quick-open-chord';
import { useQuickOpenChord } from './use-quick-open-chord';

/** What the shell knows about the core while it is starting, and after. */
export type HealthState =
  | { status: 'loading' }
  | { status: 'ready'; health: HealthReport }
  | { status: 'error'; error: CoreError };

/**
 * What quick open needs from the shell (T130-M §1).
 *
 * The bar's *state* — query, results, focus — is the controller's, and whether
 * it is open is this component's, the way the health popover's already is. That
 * split is what keeps `App.tsx`'s diff to a prop.
 */
export interface QuickOpenMount {
  /**
   * Everything the controller needs except `onDismiss`, which is this
   * component's — closing the bar is exactly the state it owns.
   *
   * The ticket's §1 says `App.tsx` passes "the controller"; it passes the
   * *deps* instead, because a controller built upstairs could not close a bar
   * whose open state lives here without an imperative handle back up. The
   * point of §1 — that `App.tsx` gains a prop and nothing structural — holds
   * either way.
   */
  readonly deps: Omit<QuickOpenDeps, 'onDismiss'>;
  /** `null` until health lands; the chord is inert rather than guessing. */
  readonly accelerator: Accelerator | null;
  /**
   * Monaco's own ⌘P, which the window listener cannot see because the editor
   * puts a `<textarea>` under the cursor and the listener yields to those so a
   * live shell keeps `Ctrl+P`.
   */
  readonly subscribeEditorChord?: (listener: (commands: boolean) => void) => () => void;
}

/**
 * The window's one top bar (T048): identity on the left, the open workspace
 * beside it, and the core's health as a dot on the right. The health facts are
 * a popover anchored to the bar rather than a panel that pushes the layout
 * down, so opening them never moves the editor.
 *
 * Quick open (T130-M) is the third thing that hangs off it, and for the third
 * time for the same reason: an anchored overlay costs no layout. The bar owns
 * whether it is showing; the controller owns what is in it.
 *
 * The island (T210) sits here too, for the same reason: the bar is where the
 * window's identity already is, so putting it there costs no layout (artboard
 * 6a, placement A). The bar owns the slot, not the island's state.
 */
export function AppBar({
  health,
  workspace,
  island,
  quickOpen,
}: {
  health: HealthState;
  workspace: Workspace | null;
  /** The island, when a workspace is open; nothing otherwise. */
  island?: ReactNode;
  /** Quick open (T130-M). Absent before the shell has a controller to give. */
  quickOpen?: QuickOpenMount;
}) {
  // The bar owns whether the overlay is showing, as it already owns the health
  // popover's — which is what lets `App.tsx` gain a prop and nothing else.
  const [open, setOpen] = useState(false);
  const [focusSignal, setFocusSignal] = useState(0);
  const dismiss = useCallback(() => setOpen(false), []);
  // Hooks are unconditional, so the controller exists whether or not quick open
  // was given; with no deps it has no workspace and asks the core nothing.
  const controller = useQuickOpen({ ...(quickOpen?.deps ?? INERT_QUICK_OPEN), onDismiss: dismiss });
  const reset = controller.reset;

  const openWith = useCallback(
    (shortcut: QuickOpenShortcut) => {
      // Opening it twice focuses and selects rather than toggling it shut
      // (§5), the same rule T120-M states for ⌘,. ⌘⇧P on an already-open bar
      // still switches it into command mode, because the chord named a mode
      // and doing nothing would be the surprising answer.
      if (open) {
        setFocusSignal((signal) => signal + 1);
        if (shortcut === 'commands') reset?.(initialQueryFor(shortcut));
        return;
      }
      reset?.(initialQueryFor(shortcut));
      setOpen(true);
    },
    [open, reset],
  );

  useQuickOpenChord(quickOpen?.accelerator ?? null, openWith);

  const subscribeEditorChord = quickOpen?.subscribeEditorChord;
  useEffect(() => {
    if (subscribeEditorChord === undefined) return;
    return subscribeEditorChord((commands) => openWith(commands ? 'commands' : 'files'));
  }, [subscribeEditorChord, openWith]);

  // The overlay dismisses rather than holding stale rows when the workspace
  // closes under it (§ failure behaviour).
  const workspaceId = workspace?.id ?? null;
  const previousWorkspace = useRef(workspaceId);
  useEffect(() => {
    if (previousWorkspace.current !== workspaceId) {
      previousWorkspace.current = workspaceId;
      setOpen(false);
    }
  }, [workspaceId]);

  return (
    <header className="appbar">
      <div className="appbar__identity">
        <h1 className="appbar__wordmark">Gritty</h1>
        {workspace ? (
          <span className="appbar__workspace">{workspace.name}</span>
        ) : (
          // Says the state rather than leaving a gap where a name goes (T049).
          <span className="appbar__workspace appbar__workspace--empty">No workspace open</span>
        )}
      </div>
      {workspace && (
        <code className="appbar__path" title={workspace.root}>
          {workspace.root}
        </code>
      )}
      <span className="appbar__spacer" />
      {quickOpen && (
        <div className="quick-open-slot">
          {open && (
            <QuickOpenBar
              raw={controller.raw}
              list={controller.list}
              activeId={controller.activeId}
              onRawChange={controller.setRaw}
              onKey={controller.handleKey}
              onFocusRow={controller.focusRow}
              onAccept={controller.accept}
              onDismiss={dismiss}
              focusSignal={focusSignal}
            />
          )}
        </div>
      )}
      {island}
      <details className="appbar__health">
        <summary aria-live="polite">
          <span className={`appbar__dot appbar__dot--${dotState(health)}`} />
          {summaryLabel(health)}
        </summary>
        {health.status === 'ready' && <HealthFacts health={health.health} />}
        {health.status === 'error' && (
          <div className="appbar__facts">
            <p className="appbar__error">Core unreachable or incompatible.</p>
            <ErrorEnvelope error={health.error} />
          </div>
        )}
      </details>
    </header>
  );
}

/** No workspace, no document, no callbacks that do anything. */
const INERT_QUICK_OPEN: Omit<QuickOpenDeps, 'onDismiss'> = {
  workspaceId: null,
  hasWorkspace: false,
  document: null,
  terminalVisible: false,
  openFolder: () => {},
  saveDocument: () => {},
  toggleTerminal: () => {},
  openSettings: () => {},
  openFile: () => {},
  revealLine: () => {},
};

function dotState(health: HealthState): string {
  if (health.status === 'ready') return 'ok';
  return health.status === 'error' ? 'error' : 'loading';
}

function summaryLabel(health: HealthState): string {
  if (health.status === 'loading') return 'Contacting core…';
  return health.status === 'ready' ? describeHealth(health.health) : 'Core unreachable';
}

function HealthFacts({ health }: { health: HealthReport }) {
  return (
    <dl className="appbar__facts">
      <dt>App</dt>
      <dd>{health.shell.appVersion}</dd>
      <dt>Core</dt>
      <dd>{health.coreVersion}</dd>
      <dt>Protocol</dt>
      <dd>{health.protocolVersion}</dd>
      <dt>Runtime</dt>
      <dd>
        {health.shell.runtime} {health.shell.runtimeVersion}
      </dd>
      <dt>WebView</dt>
      <dd>{health.shell.webviewVersion ?? 'unknown'}</dd>
      <dt>Platform</dt>
      <dd>
        {health.os}/{health.arch}
      </dd>
      <dt>Build</dt>
      <dd>{health.debug ? 'debug' : 'release'}</dd>
    </dl>
  );
}
