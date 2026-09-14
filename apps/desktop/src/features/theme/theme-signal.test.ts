import { describe, expect, it, vi } from 'vitest';
import {
  type ColorSchemeQuery,
  createThemeSignal,
  type ResolvedTheme,
  resolveTheme,
  themeChoiceOf,
} from './theme-signal';

/**
 * The theme signal (T120-T §3, §5, ADR-016 amendment).
 *
 * Node rather than jsdom: the signal needs a root element with `dataset` and
 * `style`, and a media query it can be told to change — jsdom gives the first
 * and cannot give the second. Both are injected, so a plain stub is both
 * sufficient and more honest than a DOM that cannot express the case under
 * test (the OS appearance changing).
 */

/** Enough of an element for `dataset` and `style.setProperty`. */
function fakeRoot() {
  const properties = new Map<string, string>();
  return {
    dataset: {} as { theme?: string },
    style: {
      setProperty: (name: string, value: string) => properties.set(name, value),
      removeProperty: (name: string) => properties.delete(name),
    },
    properties,
  };
}

function fakeQuery(matches: boolean) {
  const listeners = new Set<() => void>();
  return {
    matches,
    addEventListener: (_type: 'change', listener: () => void) => {
      listeners.add(listener);
    },
    removeEventListener: (_type: 'change', listener: () => void) => {
      listeners.delete(listener);
    },
    /** The operating system's appearance changed. */
    set(next: boolean) {
      this.matches = next;
      for (const listener of [...listeners]) listener();
    },
    get watchers() {
      return listeners.size;
    },
  };
}

function make(choice: Parameters<typeof createThemeSignal>[2], systemDark: boolean) {
  const root = fakeRoot();
  const query = fakeQuery(systemDark);
  const signal = createThemeSignal(
    root as unknown as HTMLElement,
    query as unknown as ColorSchemeQuery,
    choice,
  );
  return { root, query, signal };
}

describe('themeChoiceOf', () => {
  it('reads the three the registry declares', () => {
    expect(themeChoiceOf('system')).toBe('system');
    expect(themeChoiceOf('light')).toBe('light');
    expect(themeChoiceOf('dark')).toBe('dark');
  });

  it('falls back to system for anything else', () => {
    // The value comes from a JSON file a user can edit and from a core that may
    // be newer than this build. A theme that threw would leave the window
    // unpainted, which is the one outcome worse than the wrong colours.
    for (const value of [undefined, null, '', 'solarized', 7, {}, ['dark']]) {
      expect(themeChoiceOf(value)).toBe('system');
    }
  });
});

describe('resolveTheme', () => {
  it('lets the OS decide only for system', () => {
    expect(resolveTheme('system', true)).toBe('dark');
    expect(resolveTheme('system', false)).toBe('light');
  });

  it('ignores the OS for an explicit choice, in both directions', () => {
    expect(resolveTheme('dark', false)).toBe('dark');
    expect(resolveTheme('light', true)).toBe('light');
  });
});

describe('the signal', () => {
  it('leaves the attribute off for system, so the media query still decides', () => {
    // This is what keeps first paint correct for the default majority: the
    // stylesheet resolves the theme on its own before any JavaScript runs.
    const { root, signal } = make('system', true);
    expect(root.dataset.theme).toBeUndefined();
    expect(root.properties.has('color-scheme')).toBe(false);
    expect(signal.resolved).toBe('dark');
  });

  it('stamps the root and pins color-scheme for an explicit choice', () => {
    const { root, signal } = make('light', true);
    expect(root.dataset.theme).toBe('light');
    // So native scrollbars and form controls follow the picker, not the OS.
    expect(root.properties.get('color-scheme')).toBe('light');
    expect(signal.resolved).toBe('light');
  });

  it('takes the attribute back off when the user returns to system', () => {
    const { root, signal } = make('dark', false);
    expect(root.dataset.theme).toBe('dark');
    signal.setChoice('system');
    expect(root.dataset.theme).toBeUndefined();
    expect(root.properties.has('color-scheme')).toBe(false);
    expect(signal.resolved).toBe('light');
  });

  it('follows the OS live while it is on system', () => {
    const { query, signal } = make('system', false);
    expect(signal.resolved).toBe('light');
    query.set(true);
    expect(signal.resolved).toBe('dark');
    expect(signal.dark).toBe(true);
  });

  it('does not follow the OS once a choice is made, in either direction', () => {
    const explicitDark = make('dark', true);
    explicitDark.query.set(false);
    expect(explicitDark.signal.resolved).toBe('dark');

    const explicitLight = make('light', false);
    explicitLight.query.set(true);
    expect(explicitLight.signal.resolved).toBe('light');
  });

  it('is dark when there is no media query at all', () => {
    // The failure policy: dark is the designed theme and the bare `:root`, so
    // this agrees with what the stylesheet paints rather than fighting it.
    const root = fakeRoot();
    const signal = createThemeSignal(root as unknown as HTMLElement, null);
    expect(signal.resolved).toBe('dark');
    signal.setChoice('light');
    expect(signal.resolved).toBe('light');
  });

  it('notifies subscribers when the choice moves the theme', () => {
    const { signal } = make('system', true);
    const seen: ResolvedTheme[] = [];
    signal.subscribe((theme) => seen.push(theme));
    signal.setChoice('light');
    expect(seen).toEqual(['light']);
  });

  it('notifies subscribers when the OS moves the theme', () => {
    const { query, signal } = make('system', true);
    const seen: ResolvedTheme[] = [];
    signal.subscribe((theme) => seen.push(theme));
    query.set(false);
    expect(seen).toEqual(['light']);
  });

  it('says nothing when an input changes but the theme does not', () => {
    // `system → dark` with the OS already dark moves no token, and the two
    // copies of the light ramp are identical, so "resolved is the same" is a
    // safe proxy for "the palette is the same". Re-reading it would be work
    // with nothing behind it, on a path that re-themes a live xterm.
    const { signal } = make('system', true);
    const listener = vi.fn();
    signal.subscribe(listener);
    signal.setChoice('dark');
    expect(signal.choice).toBe('dark');
    expect(listener).not.toHaveBeenCalled();
  });

  it('stamps the root before it notifies', () => {
    // Load-bearing ordering: a subscriber re-reads the computed tokens, so the
    // attribute has to be on the root by the time it looks. Both readers would
    // otherwise apply the palette they were already showing.
    const { root, signal } = make('system', true);
    let stampedWhenTold: string | undefined = 'not called';
    signal.subscribe(() => {
      stampedWhenTold = root.dataset.theme;
    });
    signal.setChoice('light');
    expect(stampedWhenTold).toBe('light');
  });

  it('drops a subscriber that unsubscribes', () => {
    const { query, signal } = make('system', true);
    const listener = vi.fn();
    signal.subscribe(listener)();
    query.set(false);
    expect(listener).not.toHaveBeenCalled();
  });

  it('lets go of the operating system on dispose', () => {
    const { query, signal } = make('system', true);
    expect(query.watchers).toBe(1);
    signal.dispose();
    expect(query.watchers).toBe(0);
  });

  it('ignores a choice that is already in force', () => {
    const { signal } = make('light', true);
    const listener = vi.fn();
    signal.subscribe(listener);
    signal.setChoice('light');
    expect(listener).not.toHaveBeenCalled();
  });
});
