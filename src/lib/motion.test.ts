import { afterEach, describe, expect, it, vi } from 'vitest';
import { ms, reducedMotion } from './motion';

afterEach(() => vi.unstubAllGlobals());

describe('motion', () => {
  it('keeps durations unless the system asks for reduced motion', () => {
    vi.stubGlobal('matchMedia', () => ({ matches: false }));
    expect(reducedMotion()).toBe(false);
    expect(ms(200)).toBe(200);
    vi.stubGlobal('matchMedia', (q: string) => ({ matches: q.includes('reduce') }));
    expect(reducedMotion()).toBe(true);
    expect(ms(200)).toBe(0);
  });

  it('survives a missing matchMedia', () => {
    vi.stubGlobal('matchMedia', undefined);
    expect(ms(120)).toBe(120);
  });
});
