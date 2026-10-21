import { describe, expect, it } from 'vitest';
import { formatBytes, formatExpiry, formatUptime } from './format';

describe('format', () => {
  it('formats bytes in decimal units', () => {
    expect(formatBytes(0)).toBe('0 B');
    expect(formatBytes(999)).toBe('999 B');
    expect(formatBytes(1500)).toBe('1.5 KB');
    expect(formatBytes(150_000_000_000)).toBe('150.0 GB');
    expect(formatBytes(-1)).toBe('-');
    expect(formatBytes(NaN)).toBe('-');
  });
  it('formats uptimes', () => {
    expect(formatUptime(5)).toBe('5s');
    expect(formatUptime(600)).toBe('10 min');
    expect(formatUptime(3 * 3600 + 120)).toBe('3 h 2 min');
    expect(formatUptime(3 * 86400 + 5 * 3600)).toBe('3 d 5 h');
  });
  it('formats token expiry', () => {
    expect(formatExpiry(null, 100)).toBe('');
    expect(formatExpiry(100, 100)).toBe('expired');
    expect(formatExpiry(160, 100)).toBe('in 1 min');
    expect(formatExpiry(100 + 7200, 100)).toBe('in 2 h');
    expect(formatExpiry(100 + 6 * 86400, 100)).toBe('in 6 days');
  });
});
