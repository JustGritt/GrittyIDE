import type { Terminal, TerminalColor } from '@gritty/protocol';
import { type MouseEvent, useEffect, useRef, useState } from 'react';
import { Menu, type MenuAnchor, type MenuItem } from '../../components/menu';
import { colorLabel, colorModifier, colorVariable, TERMINAL_COLORS } from './terminal-colors';
import {
  describeCwd,
  MAX_TERMINAL_NAME_CHARS,
  type PillWorkspace,
  pillLocation,
  pillStatus,
  pillTone,
  terminalLabel,
} from './terminal-naming';
import { describeTerminal } from './terminal-reducer';

/**
 * One session on the bar (design artboard 8a): a dot, the shell or the name the
 * user gave it, where it is, and how it ended.
 *
 * The ✕ is what "Terminate" used to be. On a running shell it terminates and
 * the pill stays, so the exit status can still be read — which is the whole
 * reason an ended session remains listed; on an ended one it removes the pill.
 * Removing a running session in one step is on the context menu, where a
 * destructive shortcut belongs.
 *
 * A user colour (T098) is a bar down the left edge, not the dot: the dot says
 * whether the shell is alive, and a group of shells you coloured yellow must
 * not stop telling you that one of them died.
 */
export function TerminalPill({
  session,
  active,
  workspace,
  renaming,
  draft,
  onDraftChange,
  onBeginRename,
  onCommitRename,
  onCancelRename,
  onActivate,
  onTerminate,
  onClose,
  onRecolor,
}: {
  session: Terminal;
  active: boolean;
  /** The open workspace, the only one whose name can be shown. */
  workspace: PillWorkspace | null;
  renaming: boolean;
  draft: string;
  onDraftChange: (draft: string) => void;
  onBeginRename: () => void;
  onCommitRename: () => void;
  onCancelRename: () => void;
  onActivate: () => void;
  onTerminate: () => void;
  onClose: () => void;
  /** `null` clears the colour, putting the pill back to carrying none. */
  onRecolor: (color: TerminalColor | null) => void;
}) {
  const [menu, setMenu] = useState<MenuAnchor | null>(null);
  const field = useRef<HTMLInputElement | null>(null);
  // Focused here rather than with `autoFocus`, which a linter cannot tell apart
  // from stealing focus on load. `select` so the old name can be typed straight
  // over, which is what renaming usually means.
  useEffect(() => {
    if (renaming) field.current?.select();
  }, [renaming]);

  const running = session.status === 'RUNNING';
  const name = terminalLabel(session);
  const where = pillLocation(session, workspace);
  const status = pillStatus(session);
  const items: readonly MenuItem[] = [
    { label: 'Rename…', onSelect: onBeginRename },
    { label: 'Terminate', onSelect: onTerminate, disabled: !running },
    { label: 'Close', onSelect: onClose },
    {
      label: 'No colour',
      onSelect: () => onRecolor(null),
      checked: session.color === undefined,
      separatorBefore: true,
    },
    ...TERMINAL_COLORS.map((color) => ({
      label: colorLabel(color),
      onSelect: () => onRecolor(color),
      checked: session.color === color,
      swatch: colorVariable(color),
    })),
  ];

  function openMenu(event: MouseEvent) {
    event.preventDefault();
    setMenu({ x: event.clientX, y: event.clientY });
  }

  return (
    <div
      role="presentation"
      className={`terminal-pill${active ? ' terminal-pill--active' : ''}${
        menu ? ' terminal-pill--menu' : ''
      }${session.color ? ' terminal-pill--coloured' : ''}`}
    >
      {session.color && (
        <span
          className={`terminal-pill__edge terminal-pill__edge--${colorModifier(session.color)}`}
          aria-hidden="true"
        />
      )}
      <span
        className={`terminal-pill__dot terminal-pill__dot--${pillTone(session)}`}
        aria-hidden="true"
      />
      {renaming ? (
        <input
          className="terminal-pill__rename"
          ref={field}
          value={draft}
          spellCheck={false}
          maxLength={MAX_TERMINAL_NAME_CHARS}
          aria-label={`Rename terminal ${name}`}
          onChange={(event) => onDraftChange(event.target.value)}
          // Blur commits, so clicking away keeps what was typed rather than
          // silently discarding it.
          onBlur={onCommitRename}
          onKeyDown={(event) => {
            if (event.key === 'Enter') {
              event.preventDefault();
              onCommitRename();
            } else if (event.key === 'Escape') {
              event.preventDefault();
              onCancelRename();
            }
          }}
        />
      ) : (
        <button
          type="button"
          role="tab"
          aria-selected={active}
          aria-haspopup="menu"
          className="terminal-pill__label"
          onClick={onActivate}
          onContextMenu={openMenu}
          // Double-click is the discoverable way in; F2 is the one that works
          // without a pointer.
          onDoubleClick={onBeginRename}
          onKeyDown={(event) => {
            if (event.key === 'F2') {
              event.preventDefault();
              onBeginRename();
            }
          }}
          title={`${session.shell} in ${describeCwd(session)} — ${describeTerminal(session)}${
            session.pid ? ` — pid ${session.pid}` : ''
          }${session.color ? ` — ${colorLabel(session.color).toLowerCase()}` : ''} — F2 to rename, right-click for more`}
        >
          <span className="terminal-pill__name">{name}</span>
          {where !== null && <span className="terminal-pill__where">{where}</span>}
          {status !== null && (
            <span
              className={`terminal-pill__status${
                pillTone(session) === 'failed' ? ' terminal-pill__status--failed' : ''
              }`}
            >
              {status}
            </span>
          )}
        </button>
      )}
      <button
        type="button"
        className="terminal-pill__close"
        // Terminating keeps the pill so the exit status can be read; the same
        // ✕ on an ended session is what finally removes it. The core's kill is
        // idempotent, so neither is ever an error.
        onClick={running ? onTerminate : onClose}
        onContextMenu={openMenu}
        aria-label={running ? `Terminate terminal ${name}` : `Close terminal ${name}`}
        title={
          running
            ? 'Terminate — the session stays listed so you can read how it ended'
            : 'Close this session'
        }
      >
        ✕
      </button>
      {menu && (
        <Menu
          label={`Terminal ${name}`}
          anchor={menu}
          items={items}
          onDismiss={() => setMenu(null)}
        />
      )}
    </div>
  );
}
