import { type EditorTheme, setEditorTheme } from './editor-host';

/** Follow the OS light/dark preference (the app CSS already uses `color-scheme: light dark`). */
export function watchColorScheme(apply: (theme: EditorTheme) => void = setEditorTheme): () => void {
  const query = window.matchMedia('(prefers-color-scheme: dark)');
  const update = () => apply(query.matches ? 'vs-dark' : 'vs');
  update();
  query.addEventListener('change', update);
  return () => query.removeEventListener('change', update);
}
