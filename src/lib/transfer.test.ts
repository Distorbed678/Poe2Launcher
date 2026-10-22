import { describe, expect, it } from 'vitest';
import { formatEta, formatSpeed, installBytesDone, installBytesTotal, SpeedMeter } from './transfer';

describe('SpeedMeter', () => {
  it('needs two samples at least a second apart', () => {
    const m = new SpeedMeter();
    expect(m.rate()).toBeNull();
    m.sample(0, 0);
    expect(m.rate()).toBeNull();
    m.sample(500, 500);
    expect(m.rate()).toBeNull();
    m.sample(1000, 1000);
    expect(m.rate()).toBe(1000);
  });

  it('averages over the window and forgets old samples', () => {
    const m = new SpeedMeter(10_000);
    m.sample(0, 0);
    m.sample(10_000_000, 10_000); // 1 MB/s
    expect(m.rate()).toBe(1_000_000);
    // Twenty seconds later it only looks at the last ten.
    m.sample(10_000_000, 20_000);
    m.sample(40_000_000, 30_000); // 3 MB/s since t = 20 s
    const r = m.rate()!;
    expect(r).toBeGreaterThan(1_400_000);
    expect(r).toBeLessThan(3_100_000);
  });

  it('computes the time left, and restarts when the counter goes back', () => {
    const m = new SpeedMeter();
    m.sample(0, 0);
    m.sample(2000, 2000);
    expect(m.etaSecs(5000)).toBe(5);
    expect(m.etaSecs(0)).toBe(0);
    m.sample(10, 3000); // a new run
    expect(m.rate()).toBeNull();
    expect(m.etaSecs(100)).toBeNull();
  });

  it('a stalled transfer has no rate', () => {
    const m = new SpeedMeter();
    m.sample(100, 0);
    m.sample(100, 5000);
    expect(m.rate()).toBeNull();
  });
});

describe('formatting', () => {
  it('formats speeds in decimal units', () => {
    expect(formatSpeed(null)).toBe('-');
    expect(formatSpeed(0)).toBe('-');
    expect(formatSpeed(512)).toBe('512 B/s');
    expect(formatSpeed(12_345_678)).toBe('12.3 MB/s');
    expect(formatSpeed(150_000_000)).toBe('150 MB/s');
  });

  it('formats times left', () => {
    expect(formatEta(null)).toBe('-');
    expect(formatEta(0.2)).toBe('1 s');
    expect(formatEta(45)).toBe('45 s');
    expect(formatEta(180)).toBe('3 min');
    expect(formatEta(2 * 3600 + 5 * 60)).toBe('2 h 5 min');
    expect(formatEta(3 * 86400 + 4 * 3600)).toBe('3 d 4 h');
    expect(formatEta(Infinity)).toBe('-');
  });
});

describe('install progress', () => {
  const depots = [
    { id: 1, manifest: 'a', role: 'content', est_bytes: 1000 },
    { id: 2, manifest: 'b', role: 'exe', est_bytes: 100 },
  ];
  it('weights the depots by size', () => {
    expect(installBytesTotal(depots)).toBe(1100);
    expect(installBytesDone(depots, {})).toBe(0);
    expect(installBytesDone(depots, { 1: 50 })).toBe(500);
    expect(installBytesDone(depots, { 1: 100, 2: 50 })).toBe(1050);
    expect(installBytesDone(depots, { 1: 250, 2: -5 })).toBe(1000);
  });
});
