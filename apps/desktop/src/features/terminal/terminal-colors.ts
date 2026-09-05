import type { TerminalColor } from '@gritty/protocol';

/**
 * The colours a session can be given (T098), and what each one is drawn with.
 *
 * A closed set, in the core's own order. The values are the ANSI tokens the
 * terminal itself paints with (ADR-016), so a pill and the output under it
 * agree, and so the light ramp follows without a second palette.
 */
export const TERMINAL_COLORS: readonly TerminalColor[] = [
  'RED',
  'GREEN',
  'YELLOW',
  'BLUE',
  'MAGENTA',
  'CYAN',
];

/** The CSS custom property a colour is drawn with. */
export function colorVariable(color: TerminalColor): string {
  return `var(--ansi-${color.toLowerCase()})`;
}

/** What the menu calls a colour. */
export function colorLabel(color: TerminalColor): string {
  return color.charAt(0) + color.slice(1).toLowerCase();
}

/** The modifier a coloured pill carries, so the CSS has one class per hue. */
export function colorModifier(color: TerminalColor): string {
  return color.toLowerCase();
}
