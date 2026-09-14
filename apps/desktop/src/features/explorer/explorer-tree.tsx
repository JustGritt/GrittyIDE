import { isInstructionFile } from '@gritty/markdown';
import type { DirEntry } from '@gritty/protocol';
import {
  Fragment,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { FileIcon, type FileIconKind, fileIconKind } from '../../components/file-icon';
import { ChevronIcon } from '../../components/icons';
import { Menu, type MenuAnchor, type MenuItem } from '../../components/menu';
import { explorerIntentFor } from './explorer-keymap';
import { checkName, checkNewPath } from './explorer-names';
import { type ExplorerNode, type ExplorerRow, isExpandable, selectRows } from './explorer-reducer';
import { focusAt, moveFocus, selectAll, selectOn, typeahead } from './explorer-selection';
import type { ExplorerController } from './use-explorer';
import {
  animateRowIn,
  EXIT_MS,
  type ExitBlock,
  MOTION_EASING,
  rowKey,
  useRowMotion,
} from './use-row-motion';

/** Our own drag payload, so an OS drag and a tab drag are not mistaken for one. */
const ENTRY_MIME = 'application/x-gritty-entry';
/** How long a typed prefix keeps accumulating before it starts over. */
const TYPEAHEAD_RESET_MS = 800;

/** An inline input: renaming an entry, or naming a new one. */
type Draft =
  | { readonly kind: 'rename'; readonly path: string; readonly value: string }
  | {
      readonly kind: 'create';
      readonly parent: string;
      readonly entry: 'FILE' | 'DIRECTORY';
      readonly value: string;
    };

/** A delete waiting for the user to agree to it. */
interface PendingDelete {
  readonly paths: readonly string[];
  readonly permanent: boolean;
}

/**
 * The workspace tree (T042).
 *
 * Split out of `explorer-panel.tsx`, which keeps the frame — the section tabs,
 * the toolbar, the notices — so that the frame and the rows can be worked on
 * without the two colliding.
 *
 * It is a real tree widget: `role="tree"` with a roving `tabIndex`, arrow
 * navigation, type-ahead, and Cmd/Shift multi-select. Before this the explorer
 * was the one interactive surface in the app with no keyboard handling at all.
 * Every decision it makes lives in a pure module beside it —
 * `explorer-selection`, `explorer-keymap`, `explorer-names` — and this file is
 * the DOM and the wiring.
 */
export function ExplorerTree({
  explorer,
  onOpenFile,
}: {
  explorer: ExplorerController;
  onOpenFile?: (path: string, preview: boolean) => void;
}) {
  const { state, setSelection } = explorer;
  const { selection } = state;
  const rows = useMemo(() => selectRows(state), [state]);
  const visible = useMemo(
    () => rows.filter((row) => row.kind === 'entry').map((row) => row.node.path),
    [rows],
  );
  const nameOf = useCallback(
    (path: string) => state.nodes[path]?.entry?.name ?? path,
    [state.nodes],
  );
  const motion = useRowMotion(rows);
  const exitsAfter = useMemo(() => {
    const byAnchor = new Map<string, ExitBlock[]>();
    for (const block of motion.exits) {
      const key = `entry:${block.parent}`;
      byAnchor.set(key, [...(byAnchor.get(key) ?? []), block]);
    }
    return byAnchor;
  }, [motion.exits]);

  const [menu, setMenu] = useState<{ anchor: MenuAnchor; path: string } | null>(null);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [pendingDelete, setPendingDelete] = useState<PendingDelete | null>(null);
  const [dropTarget, setDropTarget] = useState<string | null>(null);
  const treeRef = useRef<HTMLDivElement>(null);
  /** The drag payload, because `dataTransfer` cannot be read during dragover. */
  const dragging = useRef<readonly string[]>([]);
  const typed = useRef<{ prefix: string; at: number }>({ prefix: '', at: 0 });

  /** Keep the DOM focus on the row that owns the roving tabIndex. */
  useEffect(() => {
    if (draft !== null || selection.focused === null) return;
    const row = treeRef.current?.querySelector<HTMLElement>(
      `[data-path="${CSS.escape(selection.focused)}"]`,
    );
    if (row && treeRef.current?.contains(document.activeElement)) row.focus();
  }, [selection.focused, draft]);

  /** The directory an action lands in: a folder itself, otherwise its parent. */
  const containerOf = useCallback(
    (path: string): string => {
      const node = state.nodes[path];
      if (node && isExpandable(node.entry)) return path;
      const cut = path.lastIndexOf('/');
      return cut === -1 ? '' : path.slice(0, cut);
    },
    [state.nodes],
  );

  /** Which paths an action applies to: the selection, or the row acted on. */
  const targets = useCallback(
    (path: string): readonly string[] =>
      selection.paths.includes(path) ? selection.paths : [path],
    [selection.paths],
  );

  const beginCreate = useCallback((parent: string, entry: 'FILE' | 'DIRECTORY') => {
    setDraft({ kind: 'create', parent, entry, value: '' });
  }, []);

  const commitDraft = useCallback(async () => {
    if (draft === null) return;
    const value = draft.value.trim();
    setDraft(null);
    if (value === '') return;
    if (draft.kind === 'rename') {
      if (checkName(value) !== null) return;
      if (value === nameOf(draft.path)) return;
      await explorer.renameEntry(draft.path, value);
      return;
    }
    if (checkNewPath(value) !== null) return;
    const path = draft.parent === '' ? value : `${draft.parent}/${value}`;
    await explorer.createEntry(path, draft.entry);
  }, [draft, explorer, nameOf]);

  const confirmDelete = useCallback(async () => {
    if (pendingDelete === null) return;
    const { paths, permanent } = pendingDelete;
    setPendingDelete(null);
    await explorer.deleteEntries(paths, permanent ? 'PERMANENT' : 'TRASH');
  }, [pendingDelete, explorer]);

  /** Trash goes straight through; only a permanent delete is confirmed. */
  const requestDelete = useCallback(
    (paths: readonly string[], permanent: boolean) => {
      if (paths.length === 0) return;
      if (permanent) {
        setPendingDelete({ paths, permanent });
        return;
      }
      void explorer.deleteEntries(paths, 'TRASH');
    },
    [explorer],
  );

  const copyPath = useCallback(
    (path: string, relative: boolean) => {
      const root = state.workspace?.root ?? '';
      const text = relative ? path : `${root}/${path}`;
      void navigator.clipboard?.writeText(text);
    },
    [state.workspace],
  );

  const duplicate = useCallback(
    (path: string) => {
      void explorer.transfer([path], containerOf(path) === path ? path : containerOf(path), 'COPY');
    },
    [explorer, containerOf],
  );

  const onKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLDivElement>) => {
      if (draft !== null) return;
      const intent = explorerIntentFor(event.nativeEvent);
      if (intent === null) return;
      const focused = selection.focused;
      const node = focused === null ? undefined : state.nodes[focused];

      switch (intent.kind) {
        case 'move':
          setSelection(moveFocus(selection, visible, intent.delta, intent.extend));
          break;
        case 'edge':
          setSelection(
            focusAt(
              selection,
              visible,
              intent.edge === 'first' ? 0 : visible.length - 1,
              intent.extend,
            ),
          );
          break;
        case 'expand':
          if (node && isExpandable(node.entry) && !node.expanded) explorer.toggle(node);
          else setSelection(moveFocus(selection, visible, 1));
          break;
        case 'collapse':
          if (node && isExpandable(node.entry) && node.expanded) explorer.toggle(node);
          else setSelection(moveFocus(selection, visible, -1));
          break;
        case 'open':
          if (node && isExpandable(node.entry)) explorer.toggle(node);
          else if (focused !== null) onOpenFile?.(focused, false);
          break;
        case 'preview':
          if (focused !== null && node && !isExpandable(node.entry)) onOpenFile?.(focused, true);
          break;
        case 'rename':
          if (focused !== null) setDraft({ kind: 'rename', path: focused, value: nameOf(focused) });
          break;
        case 'delete':
          if (focused !== null) requestDelete(targets(focused), intent.permanent);
          break;
        case 'cut':
        case 'copy':
          if (focused !== null) {
            explorer.setClipboard({
              paths: targets(focused),
              mode: intent.kind === 'cut' ? 'MOVE' : 'COPY',
            });
          }
          break;
        case 'paste':
          if (focused !== null) void explorer.paste(containerOf(focused));
          break;
        case 'duplicate':
          if (focused !== null) duplicate(focused);
          break;
        case 'undo':
          void explorer.undo();
          break;
        case 'redo':
          void explorer.redo();
          break;
        case 'selectAll':
          setSelection(selectAll(selection, visible));
          break;
        case 'copyPath':
          if (focused !== null) copyPath(focused, intent.relative);
          break;
        case 'typeahead': {
          const now = Date.now();
          const prefix =
            now - typed.current.at > TYPEAHEAD_RESET_MS
              ? intent.character
              : typed.current.prefix + intent.character;
          typed.current = { prefix, at: now };
          const found = typeahead(selection, visible, prefix, nameOf);
          if (found !== null) setSelection(focusAt(selection, visible, visible.indexOf(found)));
          break;
        }
      }
      event.preventDefault();
    },
    [
      draft,
      selection,
      visible,
      state.nodes,
      explorer,
      setSelection,
      onOpenFile,
      nameOf,
      requestDelete,
      targets,
      containerOf,
      duplicate,
      copyPath,
    ],
  );

  const menuItems = useCallback(
    (path: string): MenuItem[] => {
      const container = containerOf(path);
      const many = targets(path).length;
      const suffix = many > 1 ? ` ${many} items` : '';
      return [
        { label: 'New File…', onSelect: () => beginCreate(container, 'FILE') },
        { label: 'New Folder…', onSelect: () => beginCreate(container, 'DIRECTORY') },
        {
          label: 'Rename…',
          separatorBefore: true,
          disabled: many > 1,
          onSelect: () => setDraft({ kind: 'rename', path, value: nameOf(path) }),
        },
        { label: `Duplicate${suffix}`, disabled: many > 1, onSelect: () => duplicate(path) },
        {
          label: `Cut${suffix}`,
          separatorBefore: true,
          onSelect: () => explorer.setClipboard({ paths: targets(path), mode: 'MOVE' }),
        },
        {
          label: `Copy${suffix}`,
          onSelect: () => explorer.setClipboard({ paths: targets(path), mode: 'COPY' }),
        },
        {
          label: 'Paste',
          disabled: explorer.clipboard === null,
          onSelect: () => void explorer.paste(container),
        },
        { label: 'Copy Path', separatorBefore: true, onSelect: () => copyPath(path, false) },
        { label: 'Copy Relative Path', onSelect: () => copyPath(path, true) },
        {
          label: 'Reveal in Finder',
          onSelect: () => void explorer.revealInFileManager(path),
        },
        {
          label: `Move to Trash${suffix}`,
          separatorBefore: true,
          onSelect: () => requestDelete(targets(path), false),
        },
        {
          label: `Delete Permanently${suffix}…`,
          onSelect: () => requestDelete(targets(path), true),
        },
      ];
    },
    [containerOf, targets, beginCreate, nameOf, duplicate, explorer, copyPath, requestDelete],
  );

  const openMenu = useCallback(
    (event: React.MouseEvent, path: string) => {
      event.preventDefault();
      if (!selection.paths.includes(path)) setSelection(selectOn(selection, visible, path));
      // A keyboard menu key reports (0, 0); anchor to the row instead of the
      // window corner. The same handling `tab-strip.tsx` does.
      const anchor =
        event.clientX === 0 && event.clientY === 0
          ? (() => {
              const box = (event.currentTarget as HTMLElement).getBoundingClientRect();
              return { x: box.left, y: box.bottom };
            })()
          : { x: event.clientX, y: event.clientY };
      setMenu({ anchor, path });
    },
    [selection, visible, setSelection],
  );

  const draftRow = draft?.kind === 'create' ? draft : null;

  return (
    <>
      <div
        className="tree"
        ref={treeRef}
        role="tree"
        aria-label="Workspace files"
        aria-multiselectable="true"
        tabIndex={selection.focused === null ? 0 : -1}
        onKeyDown={onKeyDown}
        onDragOver={(event) => {
          if (dragging.current.length > 0) event.preventDefault();
        }}
        onDrop={(event) => {
          // A drop on the tree's own padding lands in the root.
          if (dragging.current.length === 0) return;
          event.preventDefault();
          void explorer.transfer(dragging.current, '', event.altKey ? 'COPY' : 'MOVE');
          dragging.current = [];
          setDropTarget(null);
        }}
      >
        {rows.map((row) => {
          const key = rowKey(row);
          return (
            <Fragment key={key}>
              <TreeRow
                row={row}
                enter={motion.enter.has(key)}
                explorer={explorer}
                selected={row.kind === 'entry' && selection.paths.includes(row.node.path)}
                focused={row.kind === 'entry' && selection.focused === row.node.path}
                cut={
                  explorer.clipboard?.mode === 'MOVE' &&
                  explorer.clipboard.paths.includes(row.node.path)
                }
                dropTarget={dropTarget === row.node.path}
                draft={draft?.kind === 'rename' && draft.path === row.node.path ? draft : null}
                onDraftChange={(value) => setDraft((d) => (d === null ? null : { ...d, value }))}
                onCommitDraft={() => void commitDraft()}
                onCancelDraft={() => setDraft(null)}
                onSelect={(event) =>
                  setSelection(
                    selectOn(selection, visible, row.node.path, {
                      toggle: event.metaKey || event.ctrlKey,
                      range: event.shiftKey,
                    }),
                  )
                }
                onOpenFile={onOpenFile}
                onContextMenu={(event) => openMenu(event, row.node.path)}
                onDragStart={(event) => {
                  const paths = targets(row.node.path);
                  dragging.current = paths;
                  event.dataTransfer.setData(ENTRY_MIME, paths.join('\n'));
                  event.dataTransfer.effectAllowed = 'copyMove';
                }}
                onDragEnd={() => {
                  dragging.current = [];
                  setDropTarget(null);
                }}
                onDragOverRow={(event) => {
                  if (dragging.current.length === 0) return;
                  const container = containerOf(row.node.path);
                  // Never offer to drop something into itself.
                  if (
                    dragging.current.some((p) => container === p || container.startsWith(`${p}/`))
                  ) {
                    return;
                  }
                  event.preventDefault();
                  event.stopPropagation();
                  event.dataTransfer.dropEffect = event.altKey ? 'copy' : 'move';
                  setDropTarget(row.node.path);
                }}
                onDropRow={(event) => {
                  if (dragging.current.length === 0) return;
                  event.preventDefault();
                  event.stopPropagation();
                  void explorer.transfer(
                    dragging.current,
                    containerOf(row.node.path),
                    event.altKey ? 'COPY' : 'MOVE',
                  );
                  dragging.current = [];
                  setDropTarget(null);
                }}
              />
              {exitsAfter.get(key)?.map((block) => (
                <ExitingRows key={block.id} block={block} onDone={motion.finishExit} />
              ))}
            </Fragment>
          );
        })}
        {draftRow && (
          <div className="tree__row tree__row--draft" style={{ paddingLeft: '0.5rem' }}>
            <DraftInput
              value={draftRow.value}
              label={draftRow.entry === 'FILE' ? 'New file name' : 'New folder name'}
              problem={draftRow.value === '' ? null : checkNewPath(draftRow.value)}
              onChange={(value) => setDraft((d) => (d === null ? null : { ...d, value }))}
              onCommit={() => void commitDraft()}
              onCancel={() => setDraft(null)}
            />
          </div>
        )}
      </div>

      {menu && (
        <Menu
          label="Explorer actions"
          anchor={menu.anchor}
          items={menuItems(menu.path)}
          onDismiss={() => setMenu(null)}
        />
      )}

      {pendingDelete && (
        <DeleteDialog
          pending={pendingDelete}
          nameOf={nameOf}
          onConfirm={() => void confirmDelete()}
          onCancel={() => setPendingDelete(null)}
        />
      )}
    </>
  );
}

/**
 * The rows a collapse just removed, folding away (TREE-DISCLOSURE). A copy,
 * not the rows themselves: no roles, no `data-path`, `inert` — nothing can
 * focus, select or click a row that is already gone from the tree's state.
 */
function ExitingRows({ block, onDone }: { block: ExitBlock; onDone: (id: number) => void }) {
  const ref = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const element = ref.current;
    if (!element || typeof element.animate !== 'function') {
      onDone(block.id);
      return;
    }
    const fold = element.animate(
      [
        { height: `${element.offsetHeight}px`, opacity: 1 },
        { height: '0px', opacity: 0 },
      ],
      { duration: EXIT_MS, easing: MOTION_EASING, fill: 'forwards' },
    );
    fold.onfinish = () => onDone(block.id);
    return () => {
      fold.onfinish = null;
      fold.cancel();
    };
  }, [block, onDone]);
  return (
    <div ref={ref} className="tree__exit" aria-hidden="true" inert>
      {block.rows.map((row) => (
        <GhostRow key={rowKey(row)} row={row} />
      ))}
    </div>
  );
}

/** What `TreeRow` draws, without any of its behaviour. */
function GhostRow({ row }: { row: ExplorerRow }) {
  const indent = indentOf(row.depth);
  switch (row.kind) {
    case 'entry': {
      const { node, entry } = row;
      return (
        <div className={`tree__row tree__row--${entry.kind}`} style={indent}>
          {isExpandable(entry) ? (
            <span className={`tree__toggle${node.expanded ? ' tree__toggle--open' : ''}`}>
              <ChevronIcon />
            </span>
          ) : (
            <span className="tree__glyph">
              <FileIcon kind={iconKind(entry)} />
            </span>
          )}
          <span className={`tree__name${entry.hidden ? ' tree__name--hidden' : ''}`}>
            {entry.name}
          </span>
          {describe(entry).map((badge) => (
            <span key={badge} className="tree__badge">
              {badge}
            </span>
          ))}
          {entry.size !== undefined && <span className="tree__size">{formatSize(entry.size)}</span>}
        </div>
      );
    }
    case 'more':
      return (
        <div className="tree__row tree__row--more" style={indent}>
          <button type="button" tabIndex={-1}>
            Load {row.remaining.toLocaleString()} more…
          </button>
        </div>
      );
    case 'status':
      return (
        <div
          className={`tree__row tree__row--${row.node.error ? 'error' : 'loading'}`}
          style={indent}
        >
          <span>{row.node.error ? row.node.error.message : 'Listing…'}</span>
        </div>
      );
  }
}

function indentOf(depth: number) {
  return { paddingLeft: `${0.5 + depth * 1.1}rem` };
}

function TreeRow({
  row,
  enter,
  explorer,
  selected,
  focused,
  cut,
  dropTarget,
  draft,
  onDraftChange,
  onCommitDraft,
  onCancelDraft,
  onSelect,
  onOpenFile,
  onContextMenu,
  onDragStart,
  onDragEnd,
  onDragOverRow,
  onDropRow,
}: {
  row: ExplorerRow;
  /** Fade in on mount (TREE-DISCLOSURE); ignored once the row is mounted. */
  enter: boolean;
  explorer: ExplorerController;
  selected: boolean;
  focused: boolean;
  cut: boolean;
  dropTarget: boolean;
  draft: { readonly value: string } | null;
  onDraftChange: (value: string) => void;
  onCommitDraft: () => void;
  onCancelDraft: () => void;
  onSelect: (event: React.MouseEvent) => void;
  onOpenFile?: (path: string, preview: boolean) => void;
  onContextMenu: (event: React.MouseEvent) => void;
  onDragStart: (event: React.DragEvent) => void;
  onDragEnd: () => void;
  onDragOverRow: (event: React.DragEvent) => void;
  onDropRow: (event: React.DragEvent) => void;
}) {
  const indent = indentOf(row.depth);
  // A callback ref runs as the row attaches, before paint, so the first frame
  // is already transparent. Once the row stops entering the ref is dropped,
  // which calls it with `null` and leaves a running fade alone.
  const ref = enter ? animateRowIn : undefined;
  switch (row.kind) {
    case 'entry': {
      const { node, entry } = row;
      const expandable = isExpandable(entry);
      const classes = [
        'tree__row',
        `tree__row--${entry.kind}`,
        selected ? 'tree__row--selected' : '',
        focused ? 'tree__row--focused' : '',
        cut ? 'tree__row--cut' : '',
        dropTarget ? 'tree__row--drop-target' : '',
      ]
        .filter(Boolean)
        .join(' ');
      return (
        // biome-ignore lint/a11y/useKeyWithClickEvents: every key is handled once, on the tree, where the roving tabIndex is.
        <div
          ref={ref}
          className={classes}
          style={indent}
          role="treeitem"
          aria-level={row.depth + 1}
          aria-selected={selected}
          aria-expanded={expandable ? node.expanded : undefined}
          data-path={node.path}
          tabIndex={focused ? 0 : -1}
          draggable={draft === null}
          onMouseDown={onSelect}
          onContextMenu={onContextMenu}
          onDragStart={onDragStart}
          onDragEnd={onDragEnd}
          onDragOver={onDragOverRow}
          onDragLeave={() => undefined}
          onDrop={onDropRow}
          onClick={() => {
            if (expandable) explorer.toggle(node);
            else onOpenFile?.(node.path, true);
          }}
          onDoubleClick={() => {
            if (!expandable) onOpenFile?.(node.path, false);
          }}
        >
          {/* One 16px column holds either a folder's chevron or a file's type
              icon (FILE-ICONS), so names line up across both, the way the
              Seti-themed trees in VS Code and Cursor draw it. */}
          {expandable ? (
            <button
              type="button"
              className={`tree__toggle${node.expanded ? ' tree__toggle--open' : ''}`}
              onClick={(event) => {
                event.stopPropagation();
                explorer.toggle(node);
              }}
              aria-hidden="true"
              tabIndex={-1}
            >
              <ChevronIcon />
            </button>
          ) : (
            <span className="tree__glyph" aria-hidden="true">
              <FileIcon kind={iconKind(entry)} />
            </span>
          )}
          {draft ? (
            <DraftInput
              value={draft.value}
              label={`Rename ${entry.name}`}
              problem={draft.value === '' ? null : checkName(draft.value)}
              onChange={onDraftChange}
              onCommit={onCommitDraft}
              onCancel={onCancelDraft}
            />
          ) : (
            <span className={`tree__name${entry.hidden ? ' tree__name--hidden' : ''}`}>
              {entry.name}
            </span>
          )}
          {describe(entry).map((badge) => (
            <span key={badge} className="tree__badge">
              {badge}
            </span>
          ))}
          {entry.size !== undefined && <span className="tree__size">{formatSize(entry.size)}</span>}
        </div>
      );
    }
    case 'more':
      return (
        <div ref={ref} className="tree__row tree__row--more" style={indent} role="none">
          <button type="button" onClick={() => explorer.loadMore(row.node)}>
            Load {row.remaining.toLocaleString()} more…
          </button>
          <span className="tree__badge">
            {row.node.children?.length.toLocaleString()} of {row.node.total?.toLocaleString()}
          </span>
        </div>
      );
    case 'status': {
      const { node } = row;
      if (node.error) {
        return (
          <div ref={ref} className="tree__row tree__row--error" style={indent} role="none">
            <span>
              {node.error.code}: {node.error.message}
            </span>
            <button type="button" onClick={() => explorer.reload(node)}>
              Retry
            </button>
          </div>
        );
      }
      return (
        <div ref={ref} className="tree__row tree__row--loading" style={indent} role="none">
          <span>{node.pending ? 'Listing (operation)…' : 'Listing…'}</span>
          {node.pending && (
            <button type="button" onClick={() => void explorer.cancelListing(node)}>
              Cancel
            </button>
          )}
        </div>
      );
    }
  }
}

/**
 * The inline input for a rename or a new name. The pattern `terminal-pill.tsx`
 * established: `select()` in an effect rather than `autoFocus`, blur commits,
 * Enter commits, Escape cancels.
 */
function DraftInput({
  value,
  label,
  problem,
  onChange,
  onCommit,
  onCancel,
}: {
  value: string;
  label: string;
  problem: string | null;
  onChange: (value: string) => void;
  onCommit: () => void;
  onCancel: () => void;
}) {
  const field = useRef<HTMLInputElement>(null);
  useEffect(() => {
    field.current?.select();
  }, []);
  return (
    <span className="tree__input">
      <input
        ref={field}
        value={value}
        aria-label={label}
        aria-invalid={problem !== null}
        aria-describedby={problem ? 'tree-draft-problem' : undefined}
        spellCheck={false}
        onChange={(event) => onChange(event.target.value)}
        onClick={(event) => event.stopPropagation()}
        onMouseDown={(event) => event.stopPropagation()}
        onBlur={onCommit}
        onKeyDown={(event) => {
          event.stopPropagation();
          if (event.key === 'Enter') {
            event.preventDefault();
            if (problem === null) onCommit();
          } else if (event.key === 'Escape') {
            event.preventDefault();
            onCancel();
          }
        }}
      />
      {problem && (
        <span id="tree-draft-problem" className="tree__problem" role="alert">
          {problem}
        </span>
      )}
    </span>
  );
}

/**
 * Only a *permanent* delete asks. Moving to the Trash is recoverable outside
 * Gritty, and a prompt in front of every delete trains people to dismiss it.
 */
function DeleteDialog({
  pending,
  nameOf,
  onConfirm,
  onCancel,
}: {
  pending: PendingDelete;
  nameOf: (path: string) => string;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const cancelButton = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    cancelButton.current?.focus();
  }, []);
  const what =
    pending.paths.length === 1 ? nameOf(pending.paths[0] ?? '') : `${pending.paths.length} items`;
  return (
    <div className="dialog__backdrop">
      <div
        className="dialog"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="delete-dialog-title"
        aria-describedby="delete-dialog-text"
        onKeyDown={(event) => {
          if (event.key === 'Escape') onCancel();
        }}
      >
        <h3 id="delete-dialog-title">Delete {what} permanently?</h3>
        <p id="delete-dialog-text">This does not use the Trash and cannot be undone from Gritty.</p>
        <div className="dialog__actions">
          <button type="button" ref={cancelButton} onClick={onCancel}>
            Cancel
          </button>
          <button type="button" className="dialog__danger" onClick={onConfirm}>
            Delete permanently
          </button>
        </div>
      </div>
    </div>
  );
}

/** Only for rows without a chevron: an expandable symlink draws as a folder. */
function iconKind(entry: DirEntry): FileIconKind {
  switch (entry.kind) {
    case 'symlink':
      return 'symlink';
    case 'other':
      return 'special';
    default:
      return fileIconKind(entry.name);
  }
}

function describe(entry: DirEntry): string[] {
  const badges: string[] = [];
  if (entry.ignoredHint) badges.push('ignored');
  if (entry.kind === 'symlink' && entry.symlink) {
    if (entry.symlink.broken) badges.push('broken link');
    else if (!entry.symlink.resolvesInsideRoot) badges.push('link outside root');
    else badges.push(`link → ${entry.symlink.targetKind ?? 'unknown'}`);
  }
  if (entry.kind === 'other') badges.push('special');
  if (entry.kind === 'file' && isInstructionFile(entry.name)) badges.push('instructions');
  return badges;
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(value < 10 ? 1 : 0)} ${units[unit]}`;
}

/** Unused today; kept so the row keeps its identity when a node is re-listed. */
export type { ExplorerNode };
