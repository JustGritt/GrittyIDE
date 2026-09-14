/**
 * The one place that knows what theme is in force (T120-T, ADR-016 amendment).
 *
 * ADR-016 costed this precisely and left it undone: *"an in-app theme toggle is
 * not free: everything currently keys off `prefers-color-scheme`. Adding one
 * means a root `data-theme` attribute, a persisted field, and a signal the two
 * `matchMedia` listeners can also watch."* This is that signal. The persisted
 * field landed in `settings.json` rather than in `persisted-layout.ts`, because
 * theme is user intent and not window state (ADR-031 §7).
 *
 * Three surfaces have to agree: the shell's CSS, Monaco and a live xterm. CSS
 * resolves the theme by itself from `data-theme` plus the media query — see
 * `styles/tokens.css` — and the other two cannot, because they paint their own
 * surfaces and read the computed tokens back out. Before this, both watched
 * `matchMedia` directly, which is exactly the thing an explicit choice has to
 * override. So they watch this instead, and there is one answer to "what is the
 * theme" rather than three.
 *
 * **It never fails closed to an unreadable surface.** No settings, an absent
 * value, a value from a newer build: all of them are `system`, which is today's
 * behaviour. No `matchMedia`: dark, and the picker still works.
 */

/** What the user picked. Mirrors `workbench.theme`'s enum, exactly. */
export type ThemeChoice = 'system' | 'light' | 'dark';

/** What that resolves to once the operating system has had its say. */
export type ResolvedTheme = 'light' | 'dark';

const CHOICES: readonly ThemeChoice[] = ['system', 'light', 'dark'];

/**
 * The choice a settings value names, or `system` for anything else.
 *
 * Total by construction: the value arrives from a JSON file the user can edit
 * and from a core that may be newer than this build, and a theme that throws
 * would leave the window unpainted.
 */
export function themeChoiceOf(value: unknown): ThemeChoice {
  return typeof value === 'string' && (CHOICES as readonly string[]).includes(value)
    ? (value as ThemeChoice)
    : 'system';
}

/** The theme actually in force. `system` is the only one the OS gets a vote in. */
export function resolveTheme(choice: ThemeChoice, systemPrefersDark: boolean): ResolvedTheme {
  if (choice === 'light') return 'light';
  if (choice === 'dark') return 'dark';
  return systemPrefersDark ? 'dark' : 'light';
}

/**
 * The bit of `MediaQueryList` this needs.
 *
 * Narrowed to an interface so a test can drive an OS appearance change, which
 * jsdom's `matchMedia` cannot be made to do.
 */
export interface ColorSchemeQuery {
  readonly matches: boolean;
  addEventListener(type: 'change', listener: () => void): void;
  removeEventListener(type: 'change', listener: () => void): void;
}

export interface ThemeSignal {
  /** What the user picked. */
  readonly choice: ThemeChoice;
  /** What that resolves to right now. */
  readonly resolved: ResolvedTheme;
  /** `resolved === 'dark'`. What Monaco and xterm actually branch on. */
  readonly dark: boolean;
  /** Apply a new choice, from `settings.get` or a `SettingsChanged`. */
  setChoice(choice: ThemeChoice): void;
  /**
   * Be told when the *resolved* theme changes, and only then.
   *
   * Not on every input: `system → dark` while the OS is already dark moves no
   * token, and re-reading the palette for it would be work with nothing behind
   * it. The two copies of the light ramp are identical (guarded by
   * `tokens-duplication.test.ts`), which is what makes "resolved is the same"
   * a safe proxy for "the tokens are the same".
   */
  subscribe(listener: (resolved: ResolvedTheme) => void): () => void;
  /** Stop listening to the operating system. */
  dispose(): void;
}

/**
 * A signal over one document root and one media query.
 *
 * Both are injected so the whole thing is testable; the app's instance is
 * `themeSignal` below.
 */
export function createThemeSignal(
  root: HTMLElement,
  query: ColorSchemeQuery | null,
  initial: ThemeChoice = 'system',
): ThemeSignal {
  let choice = initial;
  // No media query at all is dark: it is the designed theme, and it is what the
  // bare `:root` block paints, so this agrees with the CSS rather than fighting
  // it.
  let resolved = resolveTheme(choice, query?.matches ?? true);
  const listeners = new Set<(resolved: ResolvedTheme) => void>();

  const stamp = () => {
    // `system` leaves the attribute *off*, which is what lets the media query
    // in `tokens.css` keep working — and with it, correct first paint. An
    // explicit choice stamps, and `:root:not([data-theme="dark"])` is what
    // makes Dark beat an OS that asked for light.
    if (choice === 'system') {
      delete root.dataset.theme;
      // Back to the stylesheet's `dark light`: for a system user the platform
      // is what decides how a scrollbar or a native control is painted, and
      // pinning the value JavaScript computed a moment ago would take that away.
      root.style.removeProperty('color-scheme');
    } else {
      root.dataset.theme = choice;
      // So native scrollbars, form controls and the window's own backdrop
      // follow the choice rather than the OS.
      root.style.setProperty('color-scheme', choice);
    }
  };

  const recompute = () => {
    const next = resolveTheme(choice, query?.matches ?? true);
    // Stamped before anyone is told: a subscriber re-reads the computed tokens,
    // so the attribute has to be on the root by the time it looks.
    stamp();
    if (next === resolved) return;
    resolved = next;
    for (const listener of [...listeners]) listener(resolved);
  };

  const onSystemChange = () => recompute();
  query?.addEventListener('change', onSystemChange);
  stamp();

  return {
    get choice() {
      return choice;
    },
    get resolved() {
      return resolved;
    },
    get dark() {
      return resolved === 'dark';
    },
    setChoice(next: ThemeChoice) {
      if (next === choice) return;
      choice = next;
      recompute();
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    dispose() {
      query?.removeEventListener('change', onSystemChange);
      listeners.clear();
    },
  };
}

/** A signal for a build with no DOM at all, so importing this module is safe. */
function inertSignal(): ThemeSignal {
  return {
    choice: 'system',
    resolved: 'dark',
    dark: true,
    setChoice: () => {},
    subscribe: () => () => {},
    dispose: () => {},
  };
}

/**
 * The application's signal.
 *
 * A module singleton rather than a React context, because its two consumers are
 * not React state: `watchColorScheme` runs inside the Monaco engine's lifetime
 * and the terminal's effect applies a palette to an xterm instance in place.
 * Neither re-renders, and neither should.
 */
export const themeSignal: ThemeSignal =
  typeof document === 'undefined'
    ? inertSignal()
    : createThemeSignal(
        document.documentElement,
        typeof window !== 'undefined' && typeof window.matchMedia === 'function'
          ? window.matchMedia('(prefers-color-scheme: dark)')
          : null,
      );
