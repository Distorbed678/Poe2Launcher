import { describe, expect, it } from 'vitest';
import { newestDate, unseenCount } from './newsSeen';

describe('newsSeen', () => {
  it('counts items newer than the last seen date', () => {
    expect(unseenCount(['2026-10-28', '2026-10-27', '2026-10-21'], '2026-10-26')).toBe(2);
    expect(unseenCount(['2026-10-21'], '2026-10-21')).toBe(0);
  });

  it('shows no badge before the page was ever opened', () => {
    expect(unseenCount(['2026-10-28'], null)).toBe(0);
  });

  it('finds the newest date', () => {
    expect(newestDate(['2026-10-21', '2026-10-28T10:00:00Z', '2026-10-27'])).toBe('2026-10-28T10:00:00Z');
    expect(newestDate([])).toBeNull();
  });
});
