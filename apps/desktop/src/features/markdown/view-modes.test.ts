import type { DocumentId } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import { defaultViewMode, viewModeFor, viewModesReducer } from './view-modes';

const A = 'a' as DocumentId;
const B = 'b' as DocumentId;

describe('view modes', () => {
  it('defaults markdown to split and everything else to source', () => {
    expect(defaultViewMode('markdown')).toBe('split');
    expect(defaultViewMode('typescript')).toBe('source');
    expect(viewModeFor({}, A, 'markdown')).toBe('split');
    expect(viewModeFor({ a: 'preview' }, A, 'markdown')).toBe('preview');
  });

  it('remembers a mode per tab and drops closed tabs', () => {
    let state = viewModesReducer({}, { type: 'set', documentId: A, mode: 'preview' });
    state = viewModesReducer(state, { type: 'set', documentId: B, mode: 'source' });
    expect(state).toEqual({ a: 'preview', b: 'source' });
    expect(viewModesReducer(state, { type: 'set', documentId: A, mode: 'preview' })).toBe(state);
    expect(viewModesReducer(state, { type: 'prune', open: [A, B] })).toBe(state);
    expect(viewModesReducer(state, { type: 'prune', open: [B] })).toEqual({ b: 'source' });
  });
});
