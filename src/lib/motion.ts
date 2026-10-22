/** Motion helpers: every animation duration goes through [`ms`], which is 0 when the system asks for reduced motion. */
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
