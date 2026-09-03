/**
 * The core names languages with the VS Code vocabulary
 * (`crates/workspace/src/detect.rs`); Monaco registers a subset. Anything
 * Monaco does not know renders as plain text rather than failing to open.
 */
export const PLAINTEXT_LANGUAGE = 'plaintext';

export function resolveLanguageId(languageId: string, registered: ReadonlySet<string>): string {
  return registered.has(languageId) ? languageId : PLAINTEXT_LANGUAGE;
}
