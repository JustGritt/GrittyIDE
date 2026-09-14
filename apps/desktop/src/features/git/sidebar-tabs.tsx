import { useRef } from 'react';
import { ChangesIcon, FilesIcon } from './sidebar-icons';

/**
 * The sidebar's section tabs (T100-U), which artboard 9a drew and T233-S could
 * not build: it renamed the header to **Files** and left it an `<h2>`, writing
 * down exactly why — 9a's second tab, **Changes**, had nothing behind it,
 * because no git command reached the renderer yet. A lone tab that cannot be
 * switched away from is an affordance that does not work.
 *
 * `git.status` and `git.diff` arrived on the wire with `PROTOCOL_VERSION`
 * 2.17.0, so the second tab now has somewhere to go and the heading becomes a
 * real tablist: `role="tablist"`, two `role="tab"` buttons, a roving tabindex,
 * and Arrow/Home/End moving between them.
 *
 * It lives beside the Changes tab rather than in a shared `components/` module
 * because there are exactly two tabs and one caller. A third section is what
 * would justify hoisting it, and moving it then is a rename.
 *
 * Since SIDEBAR-ICONS each tab draws an icon rather than its label, to give the
 * header back its width. The label is still the tab's accessible name and its
 * tooltip, so a screen reader and a hover both say what the glyph means.
 */
export type SidebarTab = 'files' | 'changes';

export const SIDEBAR_TABS: readonly {
  readonly id: SidebarTab;
  readonly label: string;
  readonly Icon: () => React.JSX.Element;
}[] = [
  { id: 'files', label: 'Files', Icon: FilesIcon },
  { id: 'changes', label: 'Changes', Icon: ChangesIcon },
];

export function sidebarTabId(tab: SidebarTab): string {
  return `sidebar-tab-${tab}`;
}

export function sidebarPanelId(tab: SidebarTab): string {
  return `sidebar-panel-${tab}`;
}

export function SidebarTabs({
  tab,
  onTabChange,
}: {
  tab: SidebarTab;
  onTabChange: (tab: SidebarTab) => void;
}) {
  const buttons = useRef(new Map<SidebarTab, HTMLButtonElement | null>());

  /**
   * Automatic activation (the APG's default for tabs whose panels are cheap to
   * show): an arrow key both selects and focuses, so a keyboard user never has
   * to press a second key to see what they moved to. Focus is moved explicitly
   * because a roving tabindex leaves the old tab with `tabindex="-1"` and the
   * browser would otherwise keep focus on it.
   */
  function onKeyDown(event: React.KeyboardEvent<HTMLDivElement>) {
    const here = SIDEBAR_TABS.findIndex((candidate) => candidate.id === tab);
    const last = SIDEBAR_TABS.length - 1;
    let next: number;
    switch (event.key) {
      case 'ArrowRight':
      case 'ArrowDown':
        next = here === last ? 0 : here + 1;
        break;
      case 'ArrowLeft':
      case 'ArrowUp':
        next = here === 0 ? last : here - 1;
        break;
      case 'Home':
        next = 0;
        break;
      case 'End':
        next = last;
        break;
      default:
        return;
    }
    const target = SIDEBAR_TABS[next];
    if (target === undefined || target.id === tab) return;
    event.preventDefault();
    onTabChange(target.id);
    buttons.current.get(target.id)?.focus();
  }

  return (
    <div
      className="explorer__tablist"
      role="tablist"
      aria-label="Sidebar sections"
      onKeyDown={onKeyDown}
    >
      {SIDEBAR_TABS.map((candidate) => {
        const selected = candidate.id === tab;
        return (
          <button
            key={candidate.id}
            ref={(element) => {
              buttons.current.set(candidate.id, element);
            }}
            type="button"
            role="tab"
            id={sidebarTabId(candidate.id)}
            className={`explorer__tab${selected ? ' explorer__tab--selected' : ''}`}
            aria-selected={selected}
            // Only the shown panel exists in the document, so only the selected
            // tab points at one. `aria-controls` naming an absent id would be a
            // broken reference rather than a helpful one.
            aria-controls={selected ? sidebarPanelId(candidate.id) : undefined}
            tabIndex={selected ? 0 : -1}
            aria-label={candidate.label}
            title={candidate.label}
            onClick={() => onTabChange(candidate.id)}
          >
            <candidate.Icon />
          </button>
        );
      })}
    </div>
  );
}
