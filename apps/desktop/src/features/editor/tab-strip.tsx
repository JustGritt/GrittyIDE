import type { DocumentId } from '@gritty/protocol';
import { type DragEvent, useState } from 'react';
import { type DocumentsState, isUnsaved } from '../documents/documents-reducer';
import { fileName } from './file-name';

const TAB_MIME = 'application/x-gritty-tab';

/**
 * Open documents: activate, close (button or middle click), reorder by drag
 * and drop. The dot is `isUnsaved`: the core's `dirty` plus edits on their way.
 */
export function TabStrip({
  state,
  onActivate,
  onClose,
  onReorder,
}: {
  state: DocumentsState;
  onActivate: (documentId: DocumentId) => void;
  onClose: (documentId: DocumentId) => void;
  onReorder: (documentId: DocumentId, toIndex: number) => void;
}) {
  const [dragging, setDragging] = useState<DocumentId | null>(null);
  if (state.order.length === 0) return null;

  function onDrop(event: DragEvent<HTMLButtonElement>, index: number) {
    event.preventDefault();
    const moved = (event.dataTransfer.getData(TAB_MIME) || dragging) as DocumentId | '';
    setDragging(null);
    if (moved !== '' && state.tabs[moved]) onReorder(moved, index);
  }

  return (
    <div className="tabs" role="tablist" aria-label="Open documents">
      {state.order.map((id, index) => {
        const tab = state.tabs[id];
        if (!tab) return null;
        const isActive = id === state.activeId;
        const unsaved = isUnsaved(tab);
        const name = fileName(tab.document.path);
        return (
          <div
            key={id}
            role="presentation"
            className={`tabs__item${isActive ? ' tabs__item--active' : ''}${
              dragging === id ? ' tabs__item--dragging' : ''
            }`}
          >
            <button
              type="button"
              role="tab"
              aria-selected={isActive}
              className="tabs__tab"
              title={tab.document.path}
              draggable
              onClick={() => onActivate(id)}
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
            </button>
            <button
              type="button"
              className="tabs__close"
              aria-label={`Close ${name}${unsaved ? ' (unsaved changes)' : ''}`}
              onClick={() => onClose(id)}
            >
              ×
            </button>
          </div>
        );
      })}
    </div>
  );
}
