import { describe, expect, it } from 'vitest';
import { CoreError } from './errors';
import { PROTOCOL_VERSION } from './generated/version';
import { assertCompatible, majorVersion } from './version';

describe('protocol version guard', () => {
  it('parses the major component', () => {
    expect(majorVersion('1.0.0')).toBe(1);
    expect(majorVersion('12.3.4-beta')).toBe(12);
    expect(majorVersion('')).toBeUndefined();
    expect(majorVersion('x.1')).toBeUndefined();
  });

  it('accepts the same major with newer minor or patch', () => {
    expect(() => assertCompatible('1.0.0', '1.0.0')).not.toThrow();
    expect(() => assertCompatible('1.7.2', '1.0.0')).not.toThrow();
    expect(() => assertCompatible('1.0.0', '1.3.0')).not.toThrow();
    expect(() => assertCompatible(PROTOCOL_VERSION)).not.toThrow();
  });

  it('refuses a different major with a ProtocolMismatch envelope', () => {
    let caught: unknown;
    try {
      assertCompatible('2.0.0', '1.4.0');
    } catch (error) {
      caught = error;
    }
    expect(caught).toBeInstanceOf(CoreError);
    const error = caught as CoreError;
    expect(error.code).toBe('ProtocolMismatch');
    expect(error.retryable).toBe(false);
    expect(error.details).toEqual({ coreVersion: '2.0.0', clientVersion: '1.4.0' });
    expect(error.message).toContain('2.0.0');
  });

  it('refuses unparseable versions', () => {
    expect(() => assertCompatible('garbage', '1.0.0')).toThrow(CoreError);
  });
});
