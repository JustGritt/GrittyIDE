import { Glyph } from '../../components/icons';

/**
 * The sidebar header's glyphs (SIDEBAR-ICONS): the section tabs and the open
 * action drawn as 16px icons instead of words, so the 32px bar keeps its
 * horizontal room for the badge and the actions beside them. The frame they
 * are drawn in, and why it is inline SVG, is `components/icons.tsx`.
 */

/** Two pages, one behind the other. */
export function FilesIcon() {
  return (
    <Glyph>
      <path d="M2.5 5h5L10 7.5v7H2.5z" />
      <path d="M7.5 5v2.5H10" />
      <path d="M5.5 5V1.5H11L13.5 4v8H10" />
    </Glyph>
  );
}

/** A branch leaving a line of commits — the source-control glyph. */
export function ChangesIcon() {
  return (
    <Glyph>
      <circle cx="4.5" cy="3.5" r="1.5" />
      <circle cx="4.5" cy="12.5" r="1.5" />
      <circle cx="11.5" cy="5" r="1.5" />
      <path d="M4.5 5v6" />
      <path d="M11.5 6.5c0 3-7 2.5-7 4.5" />
    </Glyph>
  );
}

/** A folder, for opening one. */
export function FolderIcon() {
  return (
    <Glyph>
      <path d="M1.5 4.5v8a1 1 0 0 0 1 1h11a1 1 0 0 0 1-1v-6a1 1 0 0 0-1-1H8L6.5 3.5h-4a1 1 0 0 0-1 1z" />
    </Glyph>
  );
}
