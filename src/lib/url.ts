/**
 * What the user typed into a server address box, as the API base URL (mirrors
 * `normalize_base_url` in src-tauri/src/profiles.rs, which is what actually stores it).
 * Returns null when it cannot be an address.
 */
export function normalizeServerUrl(input: string): string | null {
  let s = input.trim();
  if (!s) return null;
  if (!s.includes('://')) s = `http://${s}`;
  const idx = s.indexOf('://');
  const scheme = s.slice(0, idx).toLowerCase();
  if (scheme !== 'http' && scheme !== 'https') return null;
  let rest = s.slice(idx + 3).replace(/\/+$/, '');
  if (rest.endsWith('/api/v1')) rest = rest.slice(0, -'/api/v1'.length).replace(/\/+$/, '');
  const host = rest.split('/')[0] ?? '';
  if (!host || /\s/.test(host) || host.startsWith(':')) return null;
  if (/[?#]/.test(rest)) return null;
  return `${scheme}://${rest}`;
}
