import { describe, expect, it } from 'vitest';
import { formatAgo, formatBytes, formatExpiry, formatNewsDate, formatUptime } from './format';

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

  it('formats news dates and ages', () => {
    expect(formatNewsDate('2026-10-22')).toBe('22 Oct 2026');
    expect(formatNewsDate('2026-01-05T10:00:00Z')).toBe('5 Jan 2026');
    expect(formatNewsDate('soon')).toBe('soon');
    expect(formatAgo(1000, 1010)).toBe('just now');
    expect(formatAgo(1000, 1000 + 300)).toBe('5 min ago');
    expect(formatAgo(1000, 1000 + 7200)).toBe('2 h ago');
    expect(formatAgo(1000, 1000 + 3 * 86400)).toBe('3 d ago');
  });
});
