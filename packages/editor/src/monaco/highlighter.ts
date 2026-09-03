import * as monaco from 'monaco-editor';

/**
 * Fenced-code highlighting with the editor's own grammars and theme
 * (`monaco.editor.colorize`): no second highlighter, identical colors to the
 * source view. Fence info strings are aliases (`js`, `sh`, `yml`); Monaco
 * needs exact language ids, so they are resolved through the registry.
 */
export interface CodeHighlighter {
  /** Monaco language id for a fence info string, or `null` when unknown. */
  resolveLanguage(alias: string): string | null;
  /** Theme-styled HTML (`<span class="mtkN">…`) for `code` in `languageId`. */
  colorize(languageId: string, code: string, tabSize?: number): Promise<string>;
}

export function createMonacoHighlighter(): CodeHighlighter {
  let aliases: Map<string, string> | null = null;
  const build = (): Map<string, string> => {
    const map = new Map<string, string>();
    for (const language of monaco.languages.getLanguages()) {
      for (const extension of language.extensions ?? []) {
        map.set(extension.replace(/^\./, '').toLowerCase(), language.id);
      }
      for (const alias of language.aliases ?? []) map.set(alias.toLowerCase(), language.id);
      map.set(language.id.toLowerCase(), language.id);
    }
    return map;
  };
  return {
    resolveLanguage(alias) {
      if (aliases === null) aliases = build();
      return aliases.get(alias.trim().toLowerCase()) ?? null;
    },
    colorize: (languageId, code, tabSize = 4) =>
      monaco.editor.colorize(code, languageId, { tabSize }),
  };
}
