import type { ReactNode } from 'react';

/**
 * The frame every icon in the window is drawn in (SIDEBAR-ICONS, FILE-ICONS):
 * a 16px, 16-unit SVG stroked with `currentColor`, so an icon takes the colour
 * of whatever holds it and the ADR-016 tokens stay the only palette.
 *
 * Inline SVG rather than an icon package or font: the glyphs are few, they
 * need no network or font loading, and nothing here would overlap a dependency
 * the tree already has. Icons are decorative — `aria-hidden` — so whatever
 * holds one carries the accessible name.
 */
export function Glyph({
  children,
  className,
  strokeWidth = 1.25,
}: {
  children: ReactNode;
  className?: string;
  strokeWidth?: number;
}) {
  return (
    <svg
      className={className ? `glyph ${className}` : 'glyph'}
      width="16"
      height="16"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      {children}
    </svg>
  );
}

/** Points right; CSS turns it down when what it discloses is open. */
export function ChevronIcon() {
  return (
    <Glyph strokeWidth={1.5}>
      <path d="M6 3.5 10.5 8 6 12.5" />
    </Glyph>
  );
}
