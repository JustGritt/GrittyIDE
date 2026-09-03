const INSTRUCTION_FILES = new Set(['readme.md', 'agents.md']);

/**
 * README.md and AGENTS.md are recognized for a badge and nothing else
 * (ADR-006: no vendor-specific semantics).
 */
export function isInstructionFile(path: string): boolean {
  const name = path.slice(path.lastIndexOf('/') + 1).toLowerCase();
  return INSTRUCTION_FILES.has(name);
}
