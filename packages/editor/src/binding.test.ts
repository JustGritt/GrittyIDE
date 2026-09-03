import { describe, expect, it, vi } from 'vitest';
import { bindModel } from './binding';
import { FakeTextModel } from './testing/fake-model';

describe('bindModel', () => {
  it('reports touches without reading the text, and reads lazily', () => {
    const model = new FakeTextModel('one\ntwo\n');
    const onTouch = vi.fn();
    const binding = bindModel(model, onTouch);

    model.type('one\ntwo\nthree\n');
    model.type('one\ntwo\nthree\nfour\n');
    expect(onTouch).toHaveBeenCalledTimes(2);
    expect(model.getValueCalls).toBe(0);

    expect(binding.read()).toBe('one\ntwo\nthree\nfour\n');
    expect(model.getValueCalls).toBe(1);
  });

  it('applies core text as one minimal edit that is not echoed as a touch', () => {
    const model = new FakeTextModel('line one\nline two\nline three\n');
    const onTouch = vi.fn();
    const binding = bindModel(model, onTouch);

    expect(binding.applyCoreText('line one\nline 2\nline three\n')).toBe('lf');
    expect(model.value).toBe('line one\nline 2\nline three\n');
    expect(model.edits).toEqual([
      {
        range: { startLineNumber: 2, startColumn: 6, endLineNumber: 2, endColumn: 9 },
        text: '2',
      },
    ]);
    expect(onTouch).not.toHaveBeenCalled();
  });

  it('does nothing when the core text equals the model text', () => {
    const model = new FakeTextModel('same\n');
    const binding = bindModel(model, vi.fn());
    expect(binding.applyCoreText('same\n')).toBe('lf');
    expect(model.edits).toHaveLength(0);
  });

  it('keeps CRLF text byte-exact through a reload', () => {
    const model = new FakeTextModel('one\r\ntwo\r\n');
    const binding = bindModel(model, vi.fn());
    expect(binding.applyCoreText('one\r\ntwo changed\r\nthree\r\n')).toBe('crlf');
    expect(binding.read()).toBe('one\r\ntwo changed\r\nthree\r\n');
  });

  it('switches the model line ending when the core text changed kind', () => {
    const model = new FakeTextModel('a\nb\n');
    const onTouch = vi.fn();
    const binding = bindModel(model, onTouch);
    expect(binding.applyCoreText('a\r\nb\r\nc\r\n')).toBe('crlf');
    expect(model.eol).toBe('\r\n');
    expect(binding.read()).toBe('a\r\nb\r\nc\r\n');
    expect(onTouch).not.toHaveBeenCalled();
  });

  it('shows mixed line endings normalized and reports them as such', () => {
    const model = new FakeTextModel('a\nb\n');
    const binding = bindModel(model, vi.fn());
    expect(binding.applyCoreText('a\nb\r\nc\rd\n')).toBe('mixed');
    expect(binding.read()).toBe('a\nb\nc\nd\n');
  });

  it('stops forwarding touches once disposed', () => {
    const model = new FakeTextModel('x');
    const onTouch = vi.fn();
    const binding = bindModel(model, onTouch);
    binding.dispose();
    model.type('y');
    expect(onTouch).not.toHaveBeenCalled();
  });
});
