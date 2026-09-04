import type { Terminal as TerminalSession } from '@gritty/protocol';
import { FitAddon } from '@xterm/addon-fit';
import { Terminal } from '@xterm/xterm';
import { useEffect, useRef } from 'react';
import '@xterm/xterm/css/xterm.css';
import type { OutputBus } from './output-bus';
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
  onData,
  onResize,
}: {
  session: TerminalSession;
  active: boolean;
  bus: OutputBus;
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

  useEffect(() => {
    const container = host.current;
    if (!container) return;

    const terminal = new Terminal({
      fontFamily: 'ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace',
      fontSize: 13,
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

/** Fitting a zero-sized element makes xterm compute a nonsense grid. */
function fitIfVisible(container: HTMLElement, addon: FitAddon) {
  if (container.clientWidth > 0 && container.clientHeight > 0) addon.fit();
}

/**
 * xterm paints its own surface, so it cannot inherit the app's CSS variables;
 * they are read once and handed over as a theme.
 */
function readTheme() {
  const styles = getComputedStyle(document.documentElement);
  const read = (name: string, fallback: string) => styles.getPropertyValue(name).trim() || fallback;
  return {
    background: read('--bg', '#111418'),
    foreground: read('--fg', '#e6e9ef'),
    cursor: read('--fg', '#e6e9ef'),
  };
}
