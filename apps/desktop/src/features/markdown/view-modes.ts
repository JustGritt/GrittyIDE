import type { DocumentId } from '@gritty/protocol';

export type ViewMode = 'source' | 'preview' | 'split';
export type ViewModes = Readonly<Record<string, ViewMode>>;

export const VIEW_MODES: readonly ViewMode[] = ['source', 'split', 'preview'];

/** Markdown opens side by side; everything else has no preview. */
export function defaultViewMode(languageId: string): ViewMode {
  return languageId === 'markdown' ? 'split' : 'source';
}

export function viewModeFor(
  modes: ViewModes,
  documentId: DocumentId,
  languageId: string,
): ViewMode {
  return modes[documentId] ?? defaultViewMode(languageId);
}

export type ViewModesAction =
  | { type: 'set'; documentId: DocumentId; mode: ViewMode }
  | { type: 'prune'; open: readonly DocumentId[] };

/** Per-tab view mode, in memory (T060 persists it). Pure, serializable. */
export function viewModesReducer(state: ViewModes, action: ViewModesAction): ViewModes {
  switch (action.type) {
    case 'set':
      return state[action.documentId] === action.mode
        ? state
        : { ...state, [action.documentId]: action.mode };
    case 'prune': {
      const open = new Set<string>(action.open);
      const kept = Object.entries(state).filter(([id]) => open.has(id));
      return kept.length === Object.keys(state).length ? state : Object.fromEntries(kept);
    }
  }
}
