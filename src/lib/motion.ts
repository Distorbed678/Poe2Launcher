/** Motion helpers: every animation duration goes through [`ms`], which is 0 when the system asks for reduced motion.
 *  The durations and the easing mirror the motion tokens in tokens.css (identical in the launcher and the web admin). */
export function reducedMotion(): boolean {
  try {
    return typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;
  } catch {
    return false;
  }
}

export function ms(n: number): number {
  return reducedMotion() ? 0 : n;
}

/** Durations in ms: hovers and small state changes, panels and menus, larger panels, page changes. */
export const DUR = { fast: 150, base: 200, slow: 250, page: 350 } as const;

/** The one easing curve, `cubic-bezier(0.22, 1, 0.36, 1)` (an ease-out quint), as a Svelte easing function. */
export function ease(t: number): number {
  return 1 - Math.pow(1 - t, 5);
}
