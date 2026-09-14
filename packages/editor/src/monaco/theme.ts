import { defineEditorTheme, type EditorTheme, setEditorTheme } from './editor-host';

/**
 * Read a design token off the document root. The app's CSS owns the palette
 * (ADR-016); Monaco reads it back rather than keeping a second copy, so the
 * editor surface cannot drift from the chrome around it.
 */
function token(name: string, fallback: string): string {
  if (typeof window === 'undefined') return fallback;
  const value = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return value === '' ? fallback : value;
}

/**
 * Monaco's color keys accept `#rgb` forms only, so a token that resolves to
 * something else (a `color-mix`, an `oklch`, an `rgba`) has to be dropped
 * rather than passed through — Monaco throws on a color it cannot parse, and a
 * missing key just falls back to the base theme.
 */
function hexOnly(colors: Record<string, string>): Record<string, string> {
  return Object.fromEntries(
    Object.entries(colors).filter(([, value]) => /^#(?:[0-9a-f]{3,4}|[0-9a-f]{6,8})$/i.test(value)),
  );
}

/** The chrome colors Monaco paints, mapped from the app's tokens. */
function colorsFor(dark: boolean): Record<string, string> {
  const surface = token('--surface-editor', dark ? '#15181c' : '#fffffe');
  return hexOnly({
    'editor.background': surface,
    'editor.foreground': token('--fg-dim', dark ? '#c3cad4' : '#3d444d'),
    'editorGutter.background': surface,
    'editorLineNumber.foreground': token('--fg-gutter', dark ? '#4d5763' : '#8c959f'),
    'editorLineNumber.activeForeground': token('--fg-muted', dark ? '#9aa4b2' : '#59636e'),
    'editorCursor.foreground': token('--accent', dark ? '#4c8dff' : '#0969da'),
    'editorWidget.background': token('--surface-raised', dark ? '#161b21' : '#f0f2f5'),
    'editorWidget.border': token('--border', dark ? '#1d232a' : '#d0d7de'),
    'editorSuggestWidget.background': token('--surface-raised', dark ? '#161b21' : '#f0f2f5'),
    'editorHoverWidget.background': token('--surface-raised', dark ? '#161b21' : '#f0f2f5'),
    'editorStickyScroll.background': token('--surface-editor', dark ? '#15181c' : '#fffffe'),
    'minimap.background': surface,
    'scrollbarSlider.background': token('--border', dark ? '#1d232a' : '#d0d7de'),
    focusBorder: token('--accent', dark ? '#4c8dff' : '#0969da'),
  });
}

/**
 * Where "is it dark right now" comes from.
 *
 * Narrow on purpose: `apps/desktop` hands in its theme signal
 * (`features/theme/theme-signal.ts`), which resolves the user's
 * `workbench.theme` choice against the OS, and this package must not know that
 * settings exist. The default below keeps `packages/editor` usable on its own.
 */
export interface ColorSchemeSource {
  readonly dark: boolean;
  subscribe(listener: () => void): () => void;
}

/**
 * The OS preference, and nothing else.
 *
 * The fallback for a host that has no theme signal. It is deliberately *not*
 * what the desktop app uses any more: an explicit Light or Dark choice is
 * exactly the thing this cannot see, so a window themed from here would ignore
 * the picker (T120-T).
 */
export function mediaColorScheme(): ColorSchemeSource {
  const query = window.matchMedia('(prefers-color-scheme: dark)');
  return {
    get dark() {
      return query.matches;
    },
    subscribe(listener) {
      query.addEventListener('change', listener);
      return () => query.removeEventListener('change', listener);
    },
  };
}

/**
 * Follow the theme, re-reading the palette on every change and re-theming the
 * editor in place — live editors included.
 *
 * The tokens are read fresh each time rather than once, because the CSS custom
 * properties have new values by the time this runs: the source stamps the root
 * before it notifies, and `colorsFor` reads what the stamp produced.
 */
export function watchColorScheme(
  apply: (theme: EditorTheme) => void = setEditorTheme,
  source: ColorSchemeSource = mediaColorScheme(),
): () => void {
  const update = () => {
    const dark = source.dark;
    // Defined on every change, not once: the tokens have new values by then.
    defineEditorTheme(
      dark ? 'gritty-dark' : 'gritty-light',
      dark ? 'vs-dark' : 'vs',
      colorsFor(dark),
    );
    apply(dark ? 'gritty-dark' : 'gritty-light');
  };
  update();
  return source.subscribe(update);
}
