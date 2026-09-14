import type { TerminalId } from '@gritty/protocol';
import { useEffect, useId, useRef, useState } from 'react';
import { ErrorEnvelope } from '../../components/error-envelope';
import { Menu, type MenuAnchor, type MenuItem } from '../../components/menu';
import type { TerminalLayout } from '../session/persisted-layout';
import { shouldStartFirstTerminal } from './auto-start';
import { TERMINAL_CHORD_HINT, terminalChordIndex } from './terminal-chords';
import { normalizeTerminalName, type PillWorkspace } from './terminal-naming';
import { TerminalPill } from './terminal-pill';
import { selectedProfile, spawnShell } from './terminal-profiles';
import { TerminalView } from './terminal-view';
import type { TerminalsController } from './use-terminals';

/**
 * The terminal dock: one 32px row and, under it, the active session's xterm.
 *
 * The row is design artboard 8a. It is the whole of the dock's chrome — the
 * label is the disclosure, a session is a pill rather than a tab, terminating
 * is the ✕ on the pill you are on, and the shell picker folds into the "New
 * terminal" split button. It lives outside the collapsible body on purpose: a
 * collapsed dock still shows which shells are running, which is what the count
 * badge used to approximate.
 *
 * Every view stays mounted so switching sessions keeps the scrollback; only the
 * active one is displayed.
 */
export function TerminalPanel({
  terminals,
  open,
  onOpenChange,
  workspace,
  cwd,
  chords,
  settings,
  onShellChange,
  onOpenSettings,
}: {
  terminals: TerminalsController;
  /**
   * Whether the dock is expanded. The panel stays mounted while collapsed so
   * xterm keeps its scrollback, so this is the only way it can tell that the
   * user actually asked to see a terminal.
   */
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** The open workspace, for the directory a pill shows. */
  workspace: PillWorkspace | null;
  /**
   * Where a new terminal should start, relative to the workspace root: the
   * folder of the document being edited (T098). `undefined` roots it at the
   * root, which is what happens with nothing open.
   */
  cwd: string | undefined;
  /**
   * Whether ⌥1…⌥9 select a session. Off while the island is open, which binds
   * the same chord to a decision's options — two live handlers would switch a
   * terminal and answer a question on one keystroke.
   */
  chords: boolean;
  /**
   * The `terminal.*` settings in force (T120-A): shell, custom shells, font.
   * In the shape T095 persisted them in, which is why the shell is still
   * `profilePath`.
   */
  settings: TerminalLayout;
  /** The picker chose a shell for new terminals: `terminal.shell`. */
  onShellChange: (path: string) => void;
  /** Open the settings page on its Terminal section, where the rest are edited. */
  onOpenSettings: () => void;
}) {
  const { workspaceId, active, error, profiles, ready } = terminals;
  // A remembered shell that no longer exists falls back to the default rather
  // than leaving the picker showing something that cannot start.
  const profile = selectedProfile(profiles, settings.profilePath);
  // A broken custom entry belongs in the settings, where it can be explained
  // or removed — not in a menu of things to start.
  const startable = profiles.filter((candidate) => candidate.available);
  const bodyId = useId();

  // Which pill is being renamed, and what has been typed so far. One at a
  // time: two open editors would be two answers to "what is this called".
  const [renaming, setRenaming] = useState<TerminalId | null>(null);
  const [draft, setDraft] = useState('');
  const [shellMenu, setShellMenu] = useState<MenuAnchor | null>(null);

  // Opening the dock onto nothing starts a shell (T046). The latch is per
  // opening, so closing the last pill leaves the dock empty rather than
  // immediately growing another one, and a shell that fails to start is not
  // retried in a loop.
  const sessions = terminals.terminals.length;
  const started = useRef(false);
  // Held in a ref so the effect depends only on the conditions, not on a
  // controller whose identity changes every render.
  const startFirst = useRef<() => void>(() => {});
  startFirst.current = () => void terminals.spawn(workspaceId, spawnShell(profile), cwd);
  useEffect(() => {
    if (!open) {
      started.current = false;
      return;
    }
    if (
      !shouldStartFirstTerminal({ open, ready, sessions, startedForThisOpening: started.current })
    )
      return;
    started.current = true;
    startFirst.current();
  }, [open, ready, sessions]);

  // ⌥1…⌥9 select a session from anywhere, including from inside a terminal —
  // `terminal-view.tsx` refuses the same chord so xterm neither draws the
  // character Option composed nor sends it to the shell. A chord that lands on
  // a collapsed dock opens it: switching to something you cannot see is not
  // what was asked for.
  const chordTargets = useRef<{ ids: readonly TerminalId[]; open: boolean }>({
    ids: [],
    open,
  });
  chordTargets.current = { ids: terminals.terminals.map((session) => session.id), open };
  const select = useRef<(id: TerminalId) => void>(() => {});
  select.current = (id) => {
    terminals.activate(id);
    if (!chordTargets.current.open) onOpenChange(true);
  };
  useEffect(() => {
    if (!chords) return;
    function onKeyDown(event: KeyboardEvent) {
      const index = terminalChordIndex(event);
      if (index === null) return;
      // A rename field is a field: ⌥1 halfway through typing a name must not
      // switch sessions instead.
      if (event.target instanceof HTMLElement && event.target.tagName === 'INPUT') return;
      // Claimed whether or not there is a ninth session: ⌥7 with three shells
      // open doing nothing is more predictable than it typing the character
      // Option composed into whichever one has focus.
      event.preventDefault();
      const id = chordTargets.current.ids[index];
      if (id) select.current(id);
    }
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [chords]);

  const beginRename = (id: TerminalId, current: string) => {
    setRenaming(id);
    setDraft(current);
  };

  const commitRename = (id: TerminalId) => {
    setRenaming(null);
    const result = normalizeTerminalName(draft);
    // A name the core would refuse is dropped rather than argued about: the
    // bar is not the place for a validation message, and the label the session
    // already has is still on screen.
    if ('name' in result) void terminals.rename(id, result.name);
  };

  const shellItems: readonly MenuItem[] = startable.map((candidate) => ({
    label: `${candidate.name}${candidate.isDefault ? ' (default)' : ''}`,
    onSelect: () => onShellChange(candidate.path),
  }));

  return (
    <section className="terminal-panel" aria-label="Terminals">
      <header className="terminal-panel__bar">
        <button
          type="button"
          className="terminal-panel__label"
          aria-expanded={open}
          aria-controls={bodyId}
          onClick={() => onOpenChange(!open)}
        >
          Terminal
        </button>

        <div className="terminal-panel__pills" role="tablist" aria-label="Terminal sessions">
          {terminals.terminals.map((session) => (
            <TerminalPill
              key={session.id}
              session={session}
              active={session.id === active?.id}
              workspace={workspace}
              renaming={renaming === session.id}
              draft={draft}
              onDraftChange={setDraft}
              onBeginRename={() => beginRename(session.id, session.name ?? '')}
              onCommitRename={() => commitRename(session.id)}
              onCancelRename={() => setRenaming(null)}
              onActivate={() => select.current(session.id)}
              // Terminating keeps the session listed so its exit status can be
              // read; closing forgets it. Both are idempotent in the core.
              onTerminate={() => void terminals.kill(session.id)}
              onClose={() => void terminals.close(session.id)}
              onRecolor={(color) => void terminals.recolor(session.id, color)}
            />
          ))}
        </div>

        <span className="terminal-panel__spacer" />

        {sessions > 1 && <kbd className="terminal-panel__chord">{TERMINAL_CHORD_HINT}</kbd>}

        <div className="terminal-panel__new">
          <button
            type="button"
            className="terminal-panel__new-primary"
            title={`Start ${profile ? profile.path : 'your login shell'} in ${
              workspaceId ? (cwd ? cwd : 'the workspace root') : 'your home directory'
            }`}
            onClick={() => void terminals.spawn(workspaceId, spawnShell(profile), cwd)}
          >
            New terminal
          </button>
          {startable.length > 1 && (
            <>
              <span className="terminal-panel__new-divider" aria-hidden />
              <button
                type="button"
                className="terminal-panel__new-menu"
                aria-haspopup="menu"
                aria-expanded={shellMenu !== null}
                aria-label="Shell for new terminals"
                title={profile ? profile.path : 'The default login shell'}
                onClick={(event) => {
                  const box = event.currentTarget.getBoundingClientRect();
                  setShellMenu({ x: box.left, y: box.bottom + 4 });
                }}
              >
                {profile?.name ?? 'default'} ⌄
              </button>
            </>
          )}
          {shellMenu && (
            <Menu
              label="Shell for new terminals"
              anchor={shellMenu}
              items={shellItems}
              onDismiss={() => setShellMenu(null)}
            />
          )}
        </div>

        {/* The bar's ⋯ (artboard 8a) is a way into the settings page, not a
            second place to edit them: the popover that hung here was a copy of
            settings the page owns (T120-A). A broken custom shell is explained
            there too, which is why the picker above leaves it out. */}
        <button
          type="button"
          className="terminal-panel__settings"
          aria-label="Terminal settings"
          title="Terminal settings"
          onClick={onOpenSettings}
        >
          ⋯
        </button>
      </header>

      <div className="terminal-panel__body" id={bodyId} hidden={!open}>
        {error && (
          <div className="terminal-panel__error">
            <ErrorEnvelope error={error} />
            <button type="button" onClick={terminals.clearError}>
              Dismiss
            </button>
          </div>
        )}

        <div className="terminal-panel__surface">
          {sessions === 0 && (
            <p className="terminal-panel__empty">
              {workspaceId
                ? 'No terminal yet. “New terminal” starts your login shell at the workspace root.'
                : 'No terminal yet. “New terminal” starts your login shell in your home directory.'}
            </p>
          )}
          {terminals.terminals.map((session) => (
            <TerminalView
              key={session.id}
              session={session}
              active={session.id === active?.id}
              bus={terminals.bus}
              fontFamily={settings.fontFamily}
              fontSize={settings.fontSize}
              onData={(data) => terminals.write(session.id, data)}
              onResize={(cols, rows) => terminals.resize(session.id, cols, rows)}
            />
          ))}
        </div>
      </div>
    </section>
  );
}
