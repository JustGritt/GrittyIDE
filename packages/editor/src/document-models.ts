import type { Document, DocumentId } from '@gritty/protocol';
import { bindModel, type ModelBinding } from './binding';
import { detectEol, type Eol } from './eol';
import { PLAINTEXT_LANGUAGE, resolveLanguageId } from './languages';
import type { TextModelLike } from './text-model';

export interface ModelKey {
  readonly documentId: DocumentId;
  readonly path: string;
}

/** Creates models; `createMonacoModelFactory` (browser) and the test fake implement it. */
export interface ModelFactory<M extends TextModelLike> {
  /** A model that already exists for this key (renderer reload, StrictMode double mount). */
  find(key: ModelKey): M | null;
  create(text: string, languageId: string, key: ModelKey): M;
  registeredLanguageIds(): ReadonlySet<string>;
}

export interface DocumentModelEntry<M extends TextModelLike> {
  readonly documentId: DocumentId;
  readonly model: M;
  readonly binding: ModelBinding;
  /** Monaco language actually used (`plaintext` in large-file mode or for unknown ids). */
  readonly languageId: string;
  /** Line endings of the last text received from the core. */
  eol: Eol;
}

export interface EnsureOptions {
  readonly largeFileMode: boolean;
  readonly tabSize: number;
  readonly onTouch: () => void;
}

/**
 * One model per open document, keyed by `DocumentId`. Text enters only through
 * `ensure` (initial `documents.open` text) and `applyCoreText` (reload, conflict
 * refetch, reopen); it leaves only through `read`.
 */
export class DocumentModelRegistry<M extends TextModelLike> {
  readonly #factory: ModelFactory<M>;
  readonly #entries = new Map<DocumentId, DocumentModelEntry<M>>();

  constructor(factory: ModelFactory<M>) {
    this.#factory = factory;
  }

  get(documentId: DocumentId): DocumentModelEntry<M> | null {
    return this.#entries.get(documentId) ?? null;
  }

  get size(): number {
    return this.#entries.size;
  }

  ensure(document: Document, text: string, options: EnsureOptions): DocumentModelEntry<M> {
    const existing = this.#entries.get(document.id);
    if (existing) return existing;

    const key: ModelKey = { documentId: document.id, path: document.path };
    const languageId = options.largeFileMode
      ? PLAINTEXT_LANGUAGE
      : resolveLanguageId(document.languageId, this.#factory.registeredLanguageIds());
    const found = this.#factory.find(key);
    const model = found ?? this.#factory.create(text, languageId, key);
    model.updateOptions({ tabSize: options.tabSize });
    const binding = bindModel(model, options.onTouch);
    const entry: DocumentModelEntry<M> = {
      documentId: document.id,
      model,
      binding,
      languageId,
      eol: detectEol(text),
    };
    // A model we did not create may hold stale text; the core's text wins.
    if (found) entry.eol = binding.applyCoreText(text);
    this.#entries.set(document.id, entry);
    return entry;
  }

  /** Returns the line-ending kind of `text`, or `null` when the document has no model. */
  applyCoreText(documentId: DocumentId, text: string): Eol | null {
    const entry = this.#entries.get(documentId);
    if (!entry) return null;
    entry.eol = entry.binding.applyCoreText(text);
    return entry.eol;
  }

  read(documentId: DocumentId): string | null {
    return this.#entries.get(documentId)?.binding.read() ?? null;
  }

  setTabSize(tabSize: number): void {
    for (const entry of this.#entries.values()) entry.model.updateOptions({ tabSize });
  }

  dispose(documentId: DocumentId): void {
    const entry = this.#entries.get(documentId);
    if (!entry) return;
    this.#entries.delete(documentId);
    entry.binding.dispose();
    entry.model.dispose();
  }

  disposeAll(): void {
    for (const documentId of [...this.#entries.keys()]) this.dispose(documentId);
  }
}
