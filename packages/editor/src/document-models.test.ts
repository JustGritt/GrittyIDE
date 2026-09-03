import type { Document, DocumentId, WorkspaceId } from '@gritty/protocol';
import { describe, expect, it, vi } from 'vitest';
import { DocumentModelRegistry, type ModelFactory, type ModelKey } from './document-models';
import { FakeTextModel } from './testing/fake-model';

const WS = 'ws-1' as WorkspaceId;

function doc(id: string, overrides: Partial<Document> = {}): Document {
  return {
    id: id as DocumentId,
    workspaceId: WS,
    path: `src/${id}.ts`,
    encoding: 'utf-8',
    languageId: 'typescript',
    savedRevision: 1,
    savedHash: 'sha256:a',
    currentVersion: 1,
    dirty: false,
    readOnly: false,
    binary: false,
    largeFileMode: false,
    externalModificationState: 'UNCHANGED',
    size: 5,
    ...overrides,
  };
}

interface FakeFactory extends ModelFactory<FakeTextModel> {
  created: Array<{ text: string; languageId: string; key: ModelKey }>;
  existing: Map<string, FakeTextModel>;
}

function factory(registered = new Set(['plaintext', 'typescript', 'rust'])): FakeFactory {
  const fake: FakeFactory = {
    created: [],
    existing: new Map(),
    find: (key) => fake.existing.get(key.documentId) ?? null,
    create(text, languageId, key) {
      fake.created.push({ text, languageId, key });
      return new FakeTextModel(text);
    },
    registeredLanguageIds: () => registered,
  };
  return fake;
}

const options = { largeFileMode: false, tabSize: 2, onTouch: () => {} };

describe('DocumentModelRegistry', () => {
  it('creates one model per document from the core text and reuses it', () => {
    const fake = factory();
    const registry = new DocumentModelRegistry(fake);
    const entry = registry.ensure(doc('a'), 'const a = 1;\n', options);
    expect(entry.model.value).toBe('const a = 1;\n');
    expect(entry.languageId).toBe('typescript');
    expect(entry.eol).toBe('lf');
    expect(entry.model.options).toEqual({ tabSize: 2 });
    expect(fake.created).toEqual([
      {
        text: 'const a = 1;\n',
        languageId: 'typescript',
        key: { documentId: 'a', path: 'src/a.ts' },
      },
    ]);

    expect(registry.ensure(doc('a'), 'ignored: the model already exists', options)).toBe(entry);
    expect(fake.created).toHaveLength(1);
    expect(registry.size).toBe(1);
  });

  it('uses plaintext for unknown ids and for large-file mode', () => {
    const registry = new DocumentModelRegistry(factory());
    expect(registry.ensure(doc('t', { languageId: 'toml' }), '', options).languageId).toBe(
      'plaintext',
    );
    expect(
      registry.ensure(doc('big', { largeFileMode: true }), 'x', { ...options, largeFileMode: true })
        .languageId,
    ).toBe('plaintext');
  });

  it('adopts a model that already exists for the key and gives it the core text', () => {
    const fake = factory();
    const stale = new FakeTextModel('stale text\n');
    fake.existing.set('a', stale);
    const registry = new DocumentModelRegistry(fake);
    const entry = registry.ensure(doc('a'), 'fresh text\n', options);
    expect(entry.model).toBe(stale);
    expect(stale.value).toBe('fresh text\n');
    expect(fake.created).toHaveLength(0);
  });

  it('forwards touches, reads lazily and applies core text through the binding', () => {
    const onTouch = vi.fn();
    const registry = new DocumentModelRegistry(factory());
    const entry = registry.ensure(doc('a'), 'one\n', { ...options, onTouch });

    entry.model.type('one\ntwo\n');
    expect(onTouch).toHaveBeenCalledTimes(1);
    expect(registry.read('a' as DocumentId)).toBe('one\ntwo\n');
    expect(registry.read('missing' as DocumentId)).toBeNull();

    expect(registry.applyCoreText('a' as DocumentId, 'one\r\ntwo\r\n')).toBe('crlf');
    expect(entry.eol).toBe('crlf');
    expect(entry.model.value).toBe('one\r\ntwo\r\n');
    expect(onTouch).toHaveBeenCalledTimes(1);
    expect(registry.applyCoreText('missing' as DocumentId, 'x')).toBeNull();
  });

  it('updates the tab size of every model', () => {
    const registry = new DocumentModelRegistry(factory());
    const a = registry.ensure(doc('a'), '', options);
    const b = registry.ensure(doc('b'), '', options);
    registry.setTabSize(8);
    expect(a.model.options.tabSize).toBe(8);
    expect(b.model.options.tabSize).toBe(8);
  });

  it('disposes the model and the binding', () => {
    const onTouch = vi.fn();
    const registry = new DocumentModelRegistry(factory());
    const entry = registry.ensure(doc('a'), 'x', { ...options, onTouch });
    registry.ensure(doc('b'), 'y', options);

    registry.dispose('a' as DocumentId);
    expect(entry.model.disposed).toBe(true);
    expect(registry.get('a' as DocumentId)).toBeNull();
    entry.model.type('z');
    expect(onTouch).not.toHaveBeenCalled();
    registry.dispose('a' as DocumentId);

    registry.disposeAll();
    expect(registry.size).toBe(0);
  });
});
