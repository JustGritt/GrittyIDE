import type { Terminal as TerminalSession } from '@gritty/protocol';
import { FitAddon } from '@xterm/addon-fit';
import { type ITheme, Terminal } from '@xterm/xterm';
import { useEffect, useRef } from 'react';
import '@xterm/xterm/css/xterm.css';
import type { OutputBus } from './output-bus';
import { terminalChordIndex } from './terminal-chords';
import { describeTerminal } from './terminal-reducer';

/**
 * One xterm instance bound to one core session.
 *
 * The component owns the terminal imperatively and re-renders for nothing that
 * happens inside it: output arrives through the bus and goes straight to
 * `term.write`, keystrokes go straight to `terminal.write`. React only decides
 * that this view exists and whether it is the visible one.
 *
 * Inactive views stay mounted, hidden with CSS, so switching tabs keeps the
 * scrollback. A hidden element has no size, so fitting is deferred until it is
 * shown again.
 */
export function TerminalView({
  session,
  active,
  bus,
  fontFamily,
  fontSize,
  onData,
  onResize,
}: {
  session: TerminalSession;
  active: boolean;
  bus: OutputBus;
  /** From the terminal settings; `null` keeps the default. */
  fontFamily: string | null;
  fontSize: number | null;
  onData: (data: string) => void;
  onResize: (cols: number, rows: number) => void;
}) {
  const host = useRef<HTMLDivElement | null>(null);
  const term = useRef<Terminal | null>(null);
  const fit = useRef<FitAddon | null>(null);
  // Read by the imperative handlers, which are attached once.
  const handlers = useRef({ onData, onResize });
  handlers.current = { onData, onResize };

  const exited = session.status === 'EXITED';
  const exitedRef = useRef(exited);
  exitedRef.current = exited;

  // Read once at construction; the effect below applies later changes without
  // tearing the terminal down and losing the scrollback.
  const font = useRef({ fontFamily, fontSize });
  font.current = { fontFamily, fontSize };

  useEffect(() => {
    const container = host.current;
    if (!container) return;

    const terminal = new Terminal({
      fontFamily: font.current.fontFamily ?? TERMINAL_FONT_STACK,
      fontSize: font.current.fontSize ?? DEFAULT_TERMINAL_FONT_SIZE,
      cursorBlink: true,
      // Gritty's own scrollback; the shell's is unaffected.
      scrollback: 5000,
      theme: readTheme(),
      // A session that has ended must not look like it accepts input.
      disableStdin: exitedRef.current,
    });
    const fitAddon = new FitAddon();
    terminal.loadAddon(fitAddon);
    terminal.open(container);
    // ⌥1…⌥9 belong to the dock, not to the shell. Refusing the chord here stops
    // xterm sending it to the PTY; the panel's own handler calls
    // `preventDefault`, which stops the browser inserting the character Option
    // composed (⌥1 is `¡`) into xterm's helper textarea. Both are needed —
    // neither alone silences it.
    terminal.attachCustomKeyEventHandler((event) => terminalChordIndex(event) === null);
    term.current = terminal;
    fit.current = fitAddon;

    const data = terminal.onData((chunk) => {
      if (!exitedRef.current) handlers.current.onData(chunk);
    });
    // xterm reports the grid it settled on; the PTY is told about that, not
    // about pixels, so the child's `stty size` matches what is drawn.
    const resized = terminal.onResize(({ cols, rows }) => {
      if (!exitedRef.current) handlers.current.onResize(cols, rows);
    });

    const unsubscribe = bus.subscribe(session.id, (chunk) => {
      if (chunk.truncatedBytes > 0) {
        // Say it plainly: a gap the user cannot see is worse than an ugly line.
        terminal.write(
          `\r\n\x1b[33m[gritty] ${chunk.truncatedBytes} bytes of output dropped\x1b[0m\r\n`,
        );
      }
      terminal.write(chunk.data);
    });

    const observer = new ResizeObserver(() => fitIfVisible(container, fitAddon));
    observer.observe(container);
    fitIfVisible(container, fitAddon);

    return () => {
      unsubscribe();
      observer.disconnect();
      data.dispose();
      resized.dispose();
      terminal.dispose();
      term.current = null;
      fit.current = null;
    };
    // The session id identifies the PTY; the rest of the snapshot changing
    // (status, size) must not tear the terminal down.
  }, [session.id, bus]);

  // A font change resizes every cell, so the grid has to be recomputed and
  // the PTY told about it — otherwise the shell keeps wrapping at the old
  // width. Applied in place: recreating the terminal would lose the scrollback.
  useEffect(() => {
    const terminal = term.current;
    const container = host.current;
    if (!terminal) return;
    terminal.options.fontFamily = fontFamily ?? TERMINAL_FONT_STACK;
    terminal.options.fontSize = fontSize ?? DEFAULT_TERMINAL_FONT_SIZE;
    if (container && fit.current) fitIfVisible(container, fit.current);
  }, [fontFamily, fontSize]);

  // The palette changes when the OS does. Applied in place, like the font:
  // recreating the terminal would lose the scrollback of a running shell.
  // Without this a live terminal kept the theme it was born with.
  useEffect(() => {
    const query = window.matchMedia('(prefers-color-scheme: dark)');
    const update = () => {
      const terminal = term.current;
      if (terminal) terminal.options.theme = readTheme();
    };
    query.addEventListener('change', update);
    return () => query.removeEventListener('change', update);
  }, []);

  // A hidden view cannot be measured, so it is fitted when it becomes visible.
  useEffect(() => {
    if (!active) return;
    const container = host.current;
    if (container && fit.current) fitIfVisible(container, fit.current);
    term.current?.focus();
  }, [active]);

  useEffect(() => {
    const terminal = term.current;
    if (!terminal || !exited) return;
    terminal.options.disableStdin = true;
    terminal.write(`\r\n\x1b[90m[gritty] ${describeTerminal(session)}\x1b[0m\r\n`);
  }, [exited, session]);

  return (
    <div
      className="terminal-view"
      // `hidden` would remove it from layout entirely; the inline style keeps
      // the element measurable-on-demand and the scrollback intact.
      style={{ display: active ? 'block' : 'none' }}
      ref={host}
      data-terminal-id={session.id}
    />
  );
}

/** Matches the editor's default; the settings override it. */
export const DEFAULT_TERMINAL_FONT_SIZE = 13;

/**
 * Nerd Font families first, then the ordinary monospace stack.
 *
 * A configured prompt (powerlevel10k, starship, oh-my-posh) draws its
 * separators and icons from the Private Use Area, and a font without those
 * code points renders every one of them as an empty box — which reads as "the
 * terminal is broken" rather than "a font is missing". The browser skips a
 * family that is not installed, so naming the widely-installed patched fonts
 * costs nothing on a machine that has none and fixes the prompt on a machine
 * that has one.
 *
 * These are defaults, not a setting: the terminal has no preferences surface
 * yet. When it gets one (alongside the editor's), this becomes its initial
 * value.
 */
const TERMINAL_FONT_STACK = [
  '"MesloLGS NF"',
  '"Hack Nerd Font Mono"',
  '"JetBrainsMono Nerd Font Mono"',
  '"FiraCode Nerd Font Mono"',
  '"CaskaydiaCove Nerd Font Mono"',
  '"SauceCodePro Nerd Font Mono"',
  '"Symbols Nerd Font Mono"',
  'ui-monospace',
  'SFMono-Regular',
  '"SF Mono"',
  'Menlo',
  'Consolas',
  'monospace',
].join(', ');

/** Fitting a zero-sized element makes xterm compute a nonsense grid. */
function fitIfVisible(container: HTMLElement, addon: FitAddon) {
  if (container.clientWidth > 0 && container.clientHeight > 0) addon.fit();
}

/**
 * xterm paints its own surface, so it cannot inherit the app's CSS variables;
 * they are read once and handed over as a theme.
 */
/**
 * The whole palette, read from the app's design tokens (ADR-016) so the
 * terminal cannot drift from the window around it — including the sixteen ANSI
 * colors, which used to fall through to xterm's own defaults.
 */
function readTheme(): ITheme {
  const styles = getComputedStyle(document.documentElement);
  const read = (name: string, fallback: string) => styles.getPropertyValue(name).trim() || fallback;
  return {
    background: read('--surface-base', '#0f1216'),
    foreground: read('--fg-dim', '#c3cad4'),
    cursor: read('--accent', '#4c8dff'),
    cursorAccent: read('--term-cursor-accent', '#0f1216'),
    selectionBackground: read('--term-selection', 'rgba(76, 141, 255, 0.28)'),
    black: read('--ansi-black', '#0f1216'),
    red: read('--ansi-red', '#ff7b72'),
    green: read('--ansi-green', '#7bd88f'),
    yellow: read('--ansi-yellow', '#e8b54a'),
    blue: read('--ansi-blue', '#4c8dff'),
    magenta: read('--ansi-magenta', '#d2a8ff'),
    cyan: read('--ansi-cyan', '#76e3ea'),
    white: read('--ansi-white', '#c3cad4'),
    brightBlack: read('--ansi-bright-black', '#68727f'),
    brightRed: read('--ansi-bright-red', '#ffa198'),
    brightGreen: read('--ansi-bright-green', '#9ce6ae'),
    brightYellow: read('--ansi-bright-yellow', '#f0c674'),
    brightBlue: read('--ansi-bright-blue', '#7ea9f5'),
    brightMagenta: read('--ansi-bright-magenta', '#e2c5ff'),
    brightCyan: read('--ansi-bright-cyan', '#a5f3f7'),
    brightWhite: read('--ansi-bright-white', '#e6e9ef'),
  };
}
