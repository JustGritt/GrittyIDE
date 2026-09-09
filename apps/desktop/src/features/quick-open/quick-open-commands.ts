/**
 * The commands the `>` mode offers (T130-U, §6).
 *
 * **A list, not a layer.** `quickOpenCommands` is a pure function returning a
 * typed array, and there is deliberately no way to register into it: hard rule
 * 9 names plugin systems and speculative framework layers, and an
 * anything-can-contribute registry is both. It would also displace the five
 * modules that own chords today (`open-shortcuts.ts`, `terminal-chords.ts`,
 * `agents-section-chords.ts`, `permission-chords.ts`, `explorer-keymap.ts`),
 * which is hard rule 2. A real command registry is a real ticket and a real
 * ADR; the ticket records it as a follow-up, as `T120-U:63` and `T120-M:58`
 * already did. Adding a command here means adding an entry to this array.
 *
 * Pure in the sense that matters: calling it builds descriptions, and nothing
 * happens until someone calls a `run`.
 */

export interface QuickCommand {
  /** Stable, whitespace-free, and unique in the array; the row's identity. */
  readonly id: string;
  /** What the row says. Written to describe the effect, not the mechanism. */
  readonly title: string;
  /** The heading a row sorts under. A plain string; there is no group registry. */
  readonly group: string;
  /**
   * Do the thing.
   *
   * Void-returning on purpose. A command's failure surfaces where that
   * command's own errors already go — `useDocuments` keeps its save error,
   * the launcher keeps its open error — so quick open never becomes an error
   * sink (the ticket's failure behaviour). A dep that is asynchronous is
   * wrapped by whoever supplies it; nothing here awaits, and nothing here
   * catches.
   */
  readonly run: () => void;
  /**
   * Why this command cannot run *right now* — a reason, not a boolean.
   *
   * Absent means it can run. A string means the row is inert and says this
   * sentence, so a greyed row explains itself instead of being mysteriously
   * dead. Never the empty string: an empty reason is a disabled row that
   * refuses to say why, which is the failure this field exists to prevent.
   */
  readonly disabled?: string;
}

/** The active document, as much of it as a command needs to decide. */
export interface QuickCommandDocument {
  /** The tab's label. Untrusted text: shown, never parsed. */
  readonly name: string;
  /** Whether there is anything to save. */
  readonly dirty: boolean;
}

/**
 * What the commands need from the shell.
 *
 * A plain interface of callbacks and facts, supplied by the mount (T130-M).
 * Nothing here is a service, a context or a lookup: the caller already holds
 * these functions, and passing them in is what keeps this module pure and
 * testable without a React tree.
 */
export interface QuickCommandDeps {
  /** Whether a workspace is open. */
  readonly hasWorkspace: boolean;
  /** The focused editor's document, or `null` when nothing is open. */
  readonly document: QuickCommandDocument | null;
  /** Whether the terminal dock is showing, so its command can say what it does. */
  readonly terminalVisible: boolean;
  readonly openFolder: () => void;
  readonly saveDocument: () => void;
  readonly toggleTerminal: () => void;
  readonly openSettings: () => void;
}

/**
 * The commands, in the order they are offered.
 *
 * Source order is the order rows appear: the array is authored, so it is also
 * the ranking, and nothing sorts it behind the author's back. A disabled
 * command is still listed — it is how the user finds out the command exists
 * and what it is waiting for.
 */
export function quickOpenCommands(deps: QuickCommandDeps): readonly QuickCommand[] {
  return [
    {
      id: 'workspace.openFolder',
      title: 'Open Folder…',
      group: 'File',
      run: deps.openFolder,
      // Never disabled: it is the one command that works with nothing open,
      // and it is the way out of having nothing open.
    },
    {
      id: 'file.save',
      title: 'Save',
      group: 'File',
      run: deps.saveDocument,
      ...disabledSave(deps),
    },
    {
      id: 'view.toggleTerminal',
      // The title says what the command will do rather than what it toggles,
      // so the row is readable without already knowing the current state.
      title: deps.terminalVisible ? 'Hide Terminal' : 'Show Terminal',
      group: 'View',
      run: deps.toggleTerminal,
      ...(deps.hasWorkspace
        ? {}
        : { disabled: 'A terminal needs a folder to start in. Open one first.' }),
    },
    {
      id: 'settings.open',
      title: 'Settings',
      group: 'Preferences',
      run: deps.openSettings,
    },
  ];
}

/**
 * Save's two refusals, which are different sentences because they are
 * different situations: nothing to save it *to*, versus nothing to save.
 */
function disabledSave(deps: QuickCommandDeps): { disabled?: string } {
  if (deps.document === null) return { disabled: 'No file is open, so there is nothing to save.' };
  if (!deps.document.dirty) return { disabled: 'This file has no unsaved changes.' };
  return {};
}
