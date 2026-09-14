/**
 * The settings tab's open / preview / active state (T120-U, ADR-026).
 *
 * ADR-026 made the agent console an editor tab and recorded its own "Revisit
 * when: a second kind of non-document tab is wanted". This is that second
 * kind, and it is answered by **extending the pattern rather than adding a
 * third tab mechanism**: a literal key, a state that is not `DocumentsState`,
 * and a surface that renders as a sibling inside `.editor__surfaces`.
 *
 * It is a singleton, so this is `agent-tabs-reducer.ts` with the container
 * taken out — one boolean where that one keeps a list, and the same three
 * rules about what a preview is, what a deliberate open does and what closing
 * the tab you are looking at hands back.
 *
 * **Not persisted**, per ADR-026: nothing here may reach `documents.state`,
 * which is what lets a non-document tab exist without a `SessionTab` wire
 * change and a migration. A settings tab restored into a launch the user did
 * not ask for it in is also simply wrong — you open settings to do something,
 * and it is done by the next launch.
 *
 * Pure, so opening, previewing, switching away and closing are tested without
 * React or a DOM.
 */

/**
 * The tab's identity, in `ConsoleKey`'s namespace but not of it.
 *
 * A literal rather than a minted `session:`/`run:` key, because settings is not
 * an agent lane and never will be: `parseConsoleKey` returns `null` for it,
 * which is the correct answer — there is no id inside it to recover.
 */
export const SETTINGS_TAB_KEY = 'settings';

/** What the tab strip writes on it. */
export const SETTINGS_TAB_LABEL = 'Settings';

export interface SettingsTabState {
  /** There is a tab in the strip. */
  readonly open: boolean;
  /**
   * Opened for a look rather than to keep, so the next preview replaces it —
   * `DocumentsState.previewId`'s rule (T041), for its reason.
   */
  readonly preview: boolean;
  /**
   * Showing now.
   *
   * Applied on the click, like the console's `activeId` and unlike the rail's
   * `selectedId`: a tab is a control the user operates and it has to answer
   * immediately. Nothing about the *contents* is faked in the meantime — the
   * page says what it is waiting for.
   */
  readonly active: boolean;
  /**
   * The section a "take me to X" asked the page to open on — the editor bar's
   * button asks for `editor`, the terminal dock's for `terminal` (T120-A) — or
   * `null` when nothing did, which is the page's own first section.
   *
   * Cleared whenever the page leaves the pane, so a later ⌘, is not steered
   * by an old click.
   */
  readonly section: string | null;
  /**
   * Counts those asks, so a repeat is honoured: the terminal dock stays on
   * screen beside the page, so its button can be pressed while the page is
   * already showing another section. The page is keyed on this — it reads its
   * section once, on mount — and an open that names no section leaves it alone,
   * so ⌘, does not throw away a search in progress.
   */
  readonly sectionRequest: number;
}

export const initialSettingsTabState: SettingsTabState = {
  open: false,
  preview: false,
  active: false,
  section: null,
  sectionRequest: 0,
};

export type SettingsTabAction =
  /**
   * Open the tab, or bring it forward if it is already open — on `section`
   * when one is named.
   */
  | { readonly type: 'open'; readonly preview: boolean; readonly section?: string }
  /** Keep a preview tab, so the next single click no longer replaces it. */
  | { readonly type: 'keep' }
  | { readonly type: 'close' }
  /**
   * Another surface took the pane, so settings is no longer what is showing.
   *
   * Named for the case that motivates it — a file was clicked, hand the pane
   * back — but it covers a console tab being activated too. From here the two
   * are the same event: something else is on screen, and the tab stays open
   * and inactive rather than closing, exactly as a document tab does.
   */
  | { readonly type: 'documents' };

export function settingsTabReducer(
  state: SettingsTabState,
  action: SettingsTabAction,
): SettingsTabState {
  switch (action.type) {
    case 'open': {
      // Only a *new* tab can become a preview, and a deliberate open keeps the
      // tab. Both halves matter for the double click, which fires its single
      // click first: either order of arrival lands on "kept".
      const preview = state.open ? state.preview && action.preview : action.preview;
      if (action.section === undefined) return { ...state, open: true, preview, active: true };
      return {
        open: true,
        preview,
        active: true,
        section: action.section,
        sectionRequest: state.sectionRequest + 1,
      };
    }

    case 'keep':
      return state.preview ? { ...state, preview: false } : state;

    case 'close':
      // Closing hands the area back to the documents rather than guessing at
      // another surface; there is nothing unsaved to protect, because every
      // control here has already committed or already been refused.
      return state.open ? initialSettingsTabState : state;

    case 'documents':
      return state.active ? { ...state, active: false, section: null } : state;

    default:
      return state;
  }
}
