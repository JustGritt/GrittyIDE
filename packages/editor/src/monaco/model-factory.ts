import * as monaco from 'monaco-editor';
import type { ModelFactory, ModelKey } from '../document-models';

/** `gritty://<documentId>/<path>`: unique per document, keeps the extension for the language workers. */
export function modelUri(key: ModelKey): monaco.Uri {
  return monaco.Uri.from({ scheme: 'gritty', authority: key.documentId, path: `/${key.path}` });
}

export function createMonacoModelFactory(): ModelFactory<monaco.editor.ITextModel> {
  let registered: ReadonlySet<string> | null = null;
  return {
    find: (key) => monaco.editor.getModel(modelUri(key)),
    create: (text, languageId, key) => monaco.editor.createModel(text, languageId, modelUri(key)),
    registeredLanguageIds() {
      if (registered === null) {
        registered = new Set(monaco.languages.getLanguages().map((language) => language.id));
      }
      return registered;
    },
  };
}
