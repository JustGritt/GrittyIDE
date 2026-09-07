/** @vitest-environment jsdom */
// Evidence for the line-ending policy (ADR-012): the real Monaco text model,
// not a fake. The core keeps line endings verbatim; this pins what a model
// does with them so an upgrade that changes it fails here first.
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { beforeAll, describe, expect, it, vi } from 'vitest';
import { bindModel } from '../binding';
import { dominantEol, normalizeEol } from '../eol';

function fixture(name: string): string {
  // `import.meta.url` is a Vite `/@fs/` URL under jsdom; `dirname` is the real path.
  return readFileSync(
    resolve(import.meta.dirname, '../../../../fixtures/weird-files/encoding', name),
    'utf8',
  );
}

type MonacoApi = typeof import('monaco-editor/editor/editor.api');
let monaco: MonacoApi;

beforeAll(async () => {
  if (typeof window.matchMedia !== 'function') {
    window.matchMedia = (query: string) =>
      ({
        matches: false,
        media: query,
        onchange: null,
        addEventListener() {},
        removeEventListener() {},
        addListener() {},
        removeListener() {},
        dispatchEvent: () => false,
      }) as MediaQueryList;
  }
  if (typeof document.queryCommandSupported !== 'function') {
    document.queryCommandSupported = () => false;
  }
  monaco = await import('monaco-editor/editor/editor.api');
  // 60 s, not Vitest's default 10 s, and the reason is MONACO-JSDOM-FLAKE.
  //
  // `monaco-editor` is inlined (see this package's vitest.config.ts: it ships
  // ESM without `"type": "module"` and imports .css, so Node cannot load it
  // externalized and Vite must transform it). That transform is ~9 s of real
  // work on an idle machine and it happens on every run — measured at 8.8-9.0 s
  // with `Duration ... (transform 8.80s)` while this file was failing.
  //
  // So the default left roughly one second of headroom, and any load at all
  // spent it: at load average 80 this hook timed out three runs out of three,
  // deterministically. Nothing was wrong with the import — the budget was
  // simply smaller than the work.
  //
  // This is a timeout on the slowest *setup* in the repository, not a licence
  // for slow tests: the assertions below still run under the default
  // `testTimeout`, and they are microseconds each once the model exists.
}, 60_000);

describe('Monaco text model line endings', () => {
  it('round-trips a consistently CRLF file byte-exact', () => {
    const text = fixture('crlf.txt');
    const model = monaco.editor.createModel(text, 'plaintext');
    expect(model.getEOL()).toBe('\r\n');
    expect(model.getValue()).toBe(text);
    model.dispose();
  });

  it('rewrites mixed line endings when the model is created', () => {
    const text = fixture('mixed-eol.txt');
    const model = monaco.editor.createModel(text, 'plaintext');
    expect(model.getValue()).not.toBe(text);
    expect(model.getValue()).toBe(normalizeEol(text, dominantEol(text)));
    model.dispose();
  });

  it('applies core text as an undoable minimal edit without echoing a touch', () => {
    const model = monaco.editor.createModel('one\r\ntwo\r\nthree\r\n', 'plaintext');
    const onTouch = vi.fn();
    const binding = bindModel(model, onTouch);

    model.applyEdits([{ range: new monaco.Range(2, 4, 2, 4), text: ' typed' }]);
    expect(onTouch).toHaveBeenCalledTimes(1);
    expect(binding.read()).toBe('one\r\ntwo typed\r\nthree\r\n');

    expect(binding.applyCoreText('one\r\ntwo\r\nthree\r\nfour\r\n')).toBe('crlf');
    expect(model.getValue()).toBe('one\r\ntwo\r\nthree\r\nfour\r\n');
    expect(onTouch).toHaveBeenCalledTimes(1);

    model.undo();
    expect(model.getValue()).toBe('one\r\ntwo typed\r\nthree\r\n');
    binding.dispose();
    model.dispose();
  });
});
