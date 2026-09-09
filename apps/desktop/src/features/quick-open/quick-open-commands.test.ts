import { describe, expect, it, vi } from 'vitest';
import { type QuickCommand, type QuickCommandDeps, quickOpenCommands } from './quick-open-commands';

/**
 * The command list (T130-U §6): a typed array, and a disabled command that
 * says why.
 */

function deps(overrides: Partial<QuickCommandDeps> = {}): QuickCommandDeps {
  return {
    hasWorkspace: true,
    document: { name: 'editor-pane.tsx', dirty: true },
    terminalVisible: false,
    openFolder: vi.fn(),
    saveDocument: vi.fn(),
    toggleTerminal: vi.fn(),
    openSettings: vi.fn(),
    ...overrides,
  };
}

function byId(commands: readonly QuickCommand[], id: string): QuickCommand {
  const found = commands.find((command) => command.id === id);
  if (found === undefined) throw new Error(`no command ${id}`);
  return found;
}

describe('the shape of the array', () => {
  it('is a plain array built from the deps, with no registration step', () => {
    // The non-goal stated as a test: calling the function is the whole of it.
    // If a registry ever appears, this assertion is where it announces itself.
    const commands = quickOpenCommands(deps());
    expect(Array.isArray(commands)).toBe(true);
    expect(commands.length).toBeGreaterThan(0);
  });

  it('gives every command a unique id with no whitespace in it', () => {
    // The id reaches the DOM through the row, and HTML5 forbids whitespace in
    // an id — an `aria-activedescendant` pointing at one would resolve to
    // nothing.
    const ids = quickOpenCommands(deps()).map((command) => command.id);
    expect(new Set(ids).size).toBe(ids.length);
    for (const id of ids) expect(id).not.toMatch(/\s/);
  });

  it('gives every command a title and a group', () => {
    for (const command of quickOpenCommands(deps())) {
      expect(command.title).not.toBe('');
      expect(command.group).not.toBe('');
    }
  });

  it('does not run anything just by being built', () => {
    // The list is descriptions; nothing happens until a row is accepted.
    const supplied = deps();
    quickOpenCommands(supplied);
    expect(supplied.openFolder).not.toHaveBeenCalled();
    expect(supplied.saveDocument).not.toHaveBeenCalled();
    expect(supplied.toggleTerminal).not.toHaveBeenCalled();
    expect(supplied.openSettings).not.toHaveBeenCalled();
  });

  it('runs exactly the dep it names, and only that one', () => {
    const supplied = deps();
    const commands = quickOpenCommands(supplied);
    byId(commands, 'settings.open').run();
    expect(supplied.openSettings).toHaveBeenCalledTimes(1);
    expect(supplied.openFolder).not.toHaveBeenCalled();
  });
});

describe('a disabled command says why, in a sentence', () => {
  it('never carries an empty reason', () => {
    // An empty string is a disabled row that refuses to explain itself, which
    // is the failure the reason-not-a-boolean field exists to prevent.
    for (const state of [
      deps(),
      deps({ document: null }),
      deps({ hasWorkspace: false, document: null }),
      deps({ document: { name: 'a.ts', dirty: false } }),
    ]) {
      for (const command of quickOpenCommands(state)) {
        if (command.disabled === undefined) continue;
        expect(command.disabled.length).toBeGreaterThan(0);
        expect(command.disabled).toMatch(/\.$/);
      }
    }
  });

  it('gives save two different reasons, because they are two situations', () => {
    // "There is nothing to save it to" and "there is nothing to save" send the
    // user to different next actions; one shared "unavailable" would not.
    const noFile = byId(quickOpenCommands(deps({ document: null })), 'file.save');
    const clean = byId(
      quickOpenCommands(deps({ document: { name: 'a.ts', dirty: false } })),
      'file.save',
    );
    expect(noFile.disabled).toContain('No file is open');
    expect(clean.disabled).toContain('no unsaved changes');
    expect(noFile.disabled).not.toBe(clean.disabled);
  });

  it('leaves save enabled when there are unsaved changes', () => {
    const supplied = deps({ document: { name: 'a.ts', dirty: true } });
    const save = byId(quickOpenCommands(supplied), 'file.save');
    expect(save.disabled).toBeUndefined();
    save.run();
    expect(supplied.saveDocument).toHaveBeenCalledTimes(1);
  });

  it('keeps Open Folder enabled with nothing open at all', () => {
    // The ticket's failure behaviour: with no workspace, commands still work —
    // and this is the command that is the way out of that state, so disabling
    // it would strand the user.
    const commands = quickOpenCommands(deps({ hasWorkspace: false, document: null }));
    expect(byId(commands, 'workspace.openFolder').disabled).toBeUndefined();
  });

  it('disables the terminal with no folder open, and says what is missing', () => {
    const commands = quickOpenCommands(deps({ hasWorkspace: false, document: null }));
    expect(byId(commands, 'view.toggleTerminal').disabled).toContain('folder');
  });

  it('still lists a disabled command rather than hiding it', () => {
    // Hiding it would mean the user cannot discover the command exists, or
    // find out what it is waiting for.
    const ids = quickOpenCommands(deps({ hasWorkspace: false, document: null })).map(
      (command) => command.id,
    );
    expect(ids).toContain('file.save');
    expect(ids).toContain('view.toggleTerminal');
  });
});

describe('a title that changes with the world', () => {
  it('says what the terminal command will do, not what it toggles', () => {
    // Readable without already knowing the current state, which "Toggle
    // Terminal" is not.
    expect(
      byId(quickOpenCommands(deps({ terminalVisible: false })), 'view.toggleTerminal').title,
    ).toBe('Show Terminal');
    expect(
      byId(quickOpenCommands(deps({ terminalVisible: true })), 'view.toggleTerminal').title,
    ).toBe('Hide Terminal');
  });
});
