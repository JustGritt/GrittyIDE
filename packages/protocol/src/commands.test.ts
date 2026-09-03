import { describe, expect, it } from 'vitest';
import { COMMANDS, toRustCommandName } from './commands';

describe('command naming', () => {
  it('maps <domain>.<verb> to <domain>_<snake_verb>', () => {
    expect(toRustCommandName('operations.startDemo')).toBe('operations_start_demo');
    expect(toRustCommandName('core.health')).toBe('core_health');
  });

  it('keeps the table consistent with the naming rule', () => {
    for (const [name, rust] of Object.entries(COMMANDS)) {
      expect(rust).toBe(toRustCommandName(name));
      expect(name).toMatch(/^[a-z]+\.[a-z][A-Za-z]*$/);
    }
  });
});
