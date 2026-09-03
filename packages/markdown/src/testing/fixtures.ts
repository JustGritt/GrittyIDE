import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { createMarkdownRenderer } from '../render';
import { createSanitizer } from '../sanitize';

export const FIXTURES = resolve(import.meta.dirname, '../../../../fixtures/markdown');

export function fixture(name: string): string {
  return readFileSync(resolve(FIXTURES, name), 'utf8');
}

/** A renderer bound to jsdom's window (every test file runs under jsdom). */
export function testRenderer() {
  return createMarkdownRenderer(createSanitizer(window));
}

/** Parse sanitized HTML into a detached DOM for assertions. */
export function parse(html: string): HTMLElement {
  const root = document.createElement('div');
  root.innerHTML = html;
  return root;
}
