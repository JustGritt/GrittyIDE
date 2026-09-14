import { describe, expect, it } from 'vitest';
import {
  initialSettingsTabState,
  SETTINGS_TAB_KEY,
  type SettingsTabState,
  settingsTabReducer,
} from './settings-tab';

/**
 * The settings tab's open / preview / active state (T120-U §3, ADR-026).
 *
 * `agent-tabs-reducer.test.ts`'s cases, minus the container: a singleton has no
 * "which of them" question, but every rule about what a preview is, what a
 * deliberate open does to one, and what closing the tab you are looking at
 * hands back is the same and is asserted here.
 */

function open(state: SettingsTabState, preview: boolean, section?: string): SettingsTabState {
  return settingsTabReducer(
    state,
    section === undefined ? { type: 'open', preview } : { type: 'open', preview, section },
  );
}

/** No section asked for, none ever asked. */
const UNSTEERED = { section: null, sectionRequest: 0 } as const;

describe('settingsTabReducer', () => {
  it('starts with no tab at all', () => {
    expect(initialSettingsTabState).toEqual({
      open: false,
      preview: false,
      active: false,
      ...UNSTEERED,
    });
  });

  it('opens as a preview on a single click', () => {
    expect(open(initialSettingsTabState, true)).toEqual({
      open: true,
      preview: true,
      active: true,
      ...UNSTEERED,
    });
  });

  it('opens kept on a deliberate open', () => {
    expect(open(initialSettingsTabState, false)).toEqual({
      open: true,
      preview: false,
      active: true,
      ...UNSTEERED,
    });
  });

  it('keeps a preview when a deliberate open follows it', () => {
    // The double click fires its single click first, so this order is the
    // ordinary one rather than an edge case.
    const previewed = open(initialSettingsTabState, true);
    expect(open(previewed, false).preview).toBe(false);
  });

  it('does not demote a kept tab back to a preview', () => {
    const kept = open(initialSettingsTabState, false);
    expect(open(kept, true).preview).toBe(false);
  });

  it('keeps a preview explicitly', () => {
    const previewed = open(initialSettingsTabState, true);
    expect(settingsTabReducer(previewed, { type: 'keep' }).preview).toBe(false);
  });

  it('leaves a kept tab alone on keep', () => {
    const kept = open(initialSettingsTabState, false);
    expect(settingsTabReducer(kept, { type: 'keep' })).toBe(kept);
  });

  it('brings an open-but-hidden tab forward rather than reopening it', () => {
    const hidden = settingsTabReducer(open(initialSettingsTabState, false), { type: 'documents' });
    expect(hidden).toEqual({ open: true, preview: false, active: false, ...UNSTEERED });
    expect(open(hidden, false)).toEqual({ open: true, preview: false, active: true, ...UNSTEERED });
  });

  it('hands the pane back when another surface takes it, without closing', () => {
    // A document or a console: from here they are the same event. The tab stays
    // in the strip, exactly as a document tab does when you click another one.
    const shown = open(initialSettingsTabState, false);
    expect(settingsTabReducer(shown, { type: 'documents' })).toEqual({
      open: true,
      preview: false,
      active: false,
      ...UNSTEERED,
    });
  });

  it('ignores a hand-back when it is already not showing', () => {
    expect(settingsTabReducer(initialSettingsTabState, { type: 'documents' })).toBe(
      initialSettingsTabState,
    );
  });

  it('closing returns to nothing at all', () => {
    const previewed = open(initialSettingsTabState, true);
    expect(settingsTabReducer(previewed, { type: 'close' })).toEqual(initialSettingsTabState);
  });

  it('ignores a close when there is no tab', () => {
    expect(settingsTabReducer(initialSettingsTabState, { type: 'close' })).toBe(
      initialSettingsTabState,
    );
  });

  it('opens on the section a button asked for', () => {
    const steered = open(initialSettingsTabState, false, 'terminal');
    expect(steered).toMatchObject({ open: true, active: true, section: 'terminal' });
    expect(steered.sectionRequest).toBe(1);
  });

  it('honours a repeat ask while the page is already showing', () => {
    // The dock stays on screen beside the page, so its button can be pressed
    // again after the user has moved to another section: the count is what
    // makes that a new ask rather than the same state.
    const first = open(initialSettingsTabState, false, 'terminal');
    const again = open(first, false, 'terminal');
    expect(again.section).toBe('terminal');
    expect(again.sectionRequest).toBeGreaterThan(first.sectionRequest);
    expect(open(again, false, 'editor')).toMatchObject({ section: 'editor' });
  });

  it('leaves the page where it is on an open that names no section', () => {
    // ⌘, onto a page already showing: remounting it would throw away a search.
    const steered = open(initialSettingsTabState, false, 'terminal');
    expect(open(steered, false)).toEqual(steered);
  });

  it('forgets the ask once the page leaves the pane', () => {
    const steered = open(initialSettingsTabState, false, 'terminal');
    const handedBack = settingsTabReducer(steered, { type: 'documents' });
    expect(handedBack.section).toBeNull();
    expect(open(handedBack, false).section).toBeNull();
    expect(settingsTabReducer(steered, { type: 'close' }).section).toBeNull();
  });

  it('is keyed on a literal, not on an agent lane', () => {
    // Deliberately not a `run:`/`session:` key: there is no id inside it to
    // recover, and `parseConsoleKey` returning null for it is the right answer.
    expect(SETTINGS_TAB_KEY).toBe('settings');
  });
});
