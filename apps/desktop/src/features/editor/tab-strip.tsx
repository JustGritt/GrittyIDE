import { isInstructionFile } from '@gritty/markdown';
import type { DocumentId } from '@gritty/protocol';
import { type DragEvent, type MouseEvent, useCallback, useState } from 'react';
import { Menu, type MenuAnchor, type MenuItem } from '../../components/menu';
import type { AgentTabItem } from '../agents/agent-console';
import { type DocumentsState, isPreview, isUnsaved } from '../documents/documents-reducer';
import { tabsToTheRight } from './close-plan';
import { fileName } from './file-name';

const TAB_MIME = 'application/x-gritty-tab';

/** Say how many tabs the batch would close, so it is not a leap of faith. */
function closeRightLabel(count: number): string {
  if (count === 0) return 'Close tabs to the right';
  return count === 1 ? 'Close 1 tab to the right' : `Close ${count} tabs to the right`;
}

interface OpenMenu {
  readonly documentId: DocumentId;
  readonly anchor: MenuAnchor;
}

/**
 * Open documents: activate, close (button or middle click), reorder by drag
 * and drop, and a context menu for closing more than one at a time. The dot is
 * `isUnsaved`: the core's `dirty` plus edits on their way.
 *
 * At most one tab is a *preview* — opened by a single click in the explorer,
 * to be replaced by the next one. It is set in italics, the convention every
 * editor with this behavior uses, and a double click keeps it. Dragging keeps
 * it too, through the reorder itself.
 *
 * Agent consoles are tabs here too (T233-C, ADR-026), after the documents and
 * never among them. Two things about that are deliberate:
 *
 * - **They are a second source, never a fake `DocumentTab`.** A console has no
 *   path, no buffer and no dirty state, and inventing a `Document` for one
 *   would put a non-file into the contract T030 owns. It also keeps them out of
 *   `documents.state.order`, which is what `buildSessionTabs` walks — so a
 *   console can never reach `SessionTab` and never needs the migration T231
 *   priced this feature on.
 * - **They are grouped at the end** rather than interleaved. That is the
 *   legible half of T231's objection kept: one strip, two kinds, still telling
 *   them apart at a glance. Reordering is a document affair for now.
 */
export function TabStrip({
  state,
  onActivate,
  onPromote,
  onClose,
  onCloseMany,
  onReorder,
  agents,
}: {
  state: DocumentsState;
  onActivate: (documentId: DocumentId) => void;
  /** Keep a preview tab, so the next single click no longer replaces it. */
  onPromote: (documentId: DocumentId) => void;
  onClose: (documentId: DocumentId) => void;
  /** Close a list of documents in order, asking about each unsaved one. */
  onCloseMany: (documentIds: readonly DocumentId[]) => void;
  onReorder: (documentId: DocumentId, toIndex: number) => void;
  /** The open agent consoles. Absent with no workspace, and then there are none. */
  agents?: {
    readonly items: readonly AgentTabItem[];
    /** Non-null while a console is showing, which is what deactivates the documents. */
    readonly activeId: string | null;
    onActivate: (sessionId: string) => void;
    onPromote: (sessionId: string) => void;
    onClose: (sessionId: string) => void;
  };
}) {
  const [dragging, setDragging] = useState<DocumentId | null>(null);
  const [menu, setMenu] = useState<OpenMenu | null>(null);
  const dismissMenu = useCallback(() => setMenu(null), []);
  const agentItems = agents?.items ?? [];
  // A console outlives the last file: closing every document must not take the
  // agent you are watching off the screen with them.
  if (state.order.length === 0 && agentItems.length === 0) return null;

  function onDrop(event: DragEvent<HTMLButtonElement>, index: number) {
    event.preventDefault();
    const moved = (event.dataTransfer.getData(TAB_MIME) || dragging) as DocumentId | '';
    setDragging(null);
    if (moved !== '' && state.tabs[moved]) onReorder(moved, index);
  }

  // The pointer for a right click; the tab's own corner for the keyboard menu
  // key, which reports (0, 0) and would otherwise open in the window corner.
  function openMenu(event: MouseEvent<HTMLElement>, documentId: DocumentId) {
    event.preventDefault();
    const keyboard = event.clientX === 0 && event.clientY === 0;
    const box = event.currentTarget.getBoundingClientRect();
    setMenu({
      documentId,
      anchor: keyboard ? { x: box.left, y: box.bottom } : { x: event.clientX, y: event.clientY },
    });
  }

  const menuTab = menu === null ? null : (state.tabs[menu.documentId] ?? null);
  let menuItems: readonly MenuItem[] = [];
  if (menu && menuTab) {
    const right = tabsToTheRight(state, menu.documentId);
    const id = menu.documentId;
    menuItems = [
      { label: 'Close', onSelect: () => onClose(id) },
      {
        label: closeRightLabel(right.length),
        onSelect: () => onCloseMany(right),
        disabled: right.length === 0,
      },
    ];
  }

  return (
    <div className="tabs" role="tablist" aria-label="Open documents">
      {state.order.map((id, index) => {
        const tab = state.tabs[id];
        if (!tab) return null;
        // A document tab cannot be the active one while a console is showing.
        // `documents.state.activeId` is deliberately left pointing at it — that
        // is what session restore persists — so the check is here, in the view.
        const isActive = id === state.activeId && agents?.activeId == null;
        const unsaved = isUnsaved(tab);
        const preview = isPreview(state, id);
        const name = fileName(tab.document.path);
        return (
          <div
            key={id}
            role="presentation"
            className={`tabs__item${isActive ? ' tabs__item--active' : ''}${
              dragging === id ? ' tabs__item--dragging' : ''
            }${menu?.documentId === id ? ' tabs__item--menu' : ''}`}
          >
            <button
              type="button"
              role="tab"
              aria-selected={isActive}
              aria-haspopup="menu"
              className={`tabs__tab${preview ? ' tabs__tab--preview' : ''}`}
              title={
                preview ? `${tab.document.path} — preview, double-click to keep` : tab.document.path
              }
              draggable
              onClick={() => onActivate(id)}
              onDoubleClick={() => onPromote(id)}
              onContextMenu={(event) => openMenu(event, id)}
              onAuxClick={(event) => {
                if (event.button === 1) onClose(id);
              }}
              onDragStart={(event) => {
                event.dataTransfer.setData(TAB_MIME, id);
                event.dataTransfer.effectAllowed = 'move';
                setDragging(id);
              }}
              onDragEnd={() => setDragging(null)}
              onDragOver={(event) => {
                if (dragging === null) return;
                event.preventDefault();
                event.dataTransfer.dropEffect = 'move';
              }}
              onDrop={(event) => onDrop(event, index)}
            >
              <span className="tabs__dot" aria-hidden="true">
                {unsaved ? '●' : ''}
              </span>
              {name}
              {isInstructionFile(tab.document.path) && (
                <span className="tabs__badge" title="instruction file">
                  instructions
                </span>
              )}
            </button>
            <button
              type="button"
              className="tabs__close"
              aria-label={`Close ${name}${unsaved ? ' (unsaved changes)' : ''}`}
              onClick={() => onClose(id)}
              onContextMenu={(event) => openMenu(event, id)}
            >
              ×
            </button>
          </div>
        );
      })}
      {agentItems.map((item) => {
        const isActive = item.sessionId === agents?.activeId;
        return (
          <div
            key={`agent:${item.sessionId}`}
            role="presentation"
            className={`tabs__item tabs__item--agent${isActive ? ' tabs__item--active' : ''}`}
          >
            <button
              type="button"
              role="tab"
              aria-selected={isActive}
              className={`tabs__tab${item.preview ? ' tabs__tab--preview' : ''}`}
              title={
                item.preview
                  ? `${item.label} — ${item.hint} — preview, double-click to keep`
                  : `${item.label} — ${item.hint}`
              }
              onClick={() => agents?.onActivate(item.sessionId)}
              onDoubleClick={() => agents?.onPromote(item.sessionId)}
              onAuxClick={(event) => {
                if (event.button === 1) agents?.onClose(item.sessionId);
              }}
            >
              {/* The status mark stands where a document's unsaved dot stands,
                  so the two kinds of tab read in the same places. Its shape
                  carries the status as well as its colour (T231 §10). */}
              <span
                className={`agent-status agent-status--${item.tone} tabs__agent-mark`}
                aria-hidden="true"
              >
                <span className="agent-status__mark" />
              </span>
              {item.label}
            </button>
            <button
              type="button"
              className="tabs__close"
              aria-label={`Close the ${item.label} agent console`}
              onClick={() => agents?.onClose(item.sessionId)}
            >
              ×
            </button>
          </div>
        );
      })}
      {menu && menuTab && (
        <Menu
          label={`${fileName(menuTab.document.path)} tab actions`}
          anchor={menu.anchor}
          items={menuItems}
          onDismiss={dismissMenu}
        />
      )}
    </div>
  );
}
