import type { PlainHttp, ProfileView } from './types';

/** This computer: localhost, 127.0.0.0/8, ::1 (mirrors `is_loopback_host` in src-tauri/src/profiles.rs). */
export function isLoopbackHost(host: string): boolean {
  const h = host.trim().replace(/\.$/, '').toLowerCase().replace(/^\[|\]$/g, '');
  if (h === 'localhost' || h.endsWith('.localhost')) return true;
  if (h === '::1') return true;
  return /^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(h);
}

/** Loopback, or a local-network address (mirrors `is_private_host`). */
export function isPrivateHost(host: string): boolean {
  if (isLoopbackHost(host)) return true;
  const h = host.trim().replace(/\.$/, '').toLowerCase().replace(/^\[|\]$/g, '');
  const m = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/.exec(h);
  if (m) {
    const [a, b] = [Number(m[1]), Number(m[2])];
    return a === 10 || (a === 172 && b >= 16 && b <= 31) || (a === 192 && b === 168) || (a === 169 && b === 254);
  }
  if (h.includes(':')) return /^f[cd][0-9a-f]{2}:/.test(h) || /^fe[89ab][0-9a-f]:/.test(h);
  return !h.includes('.') || h.endsWith('.local') || h.endsWith('.lan') || h.endsWith('.home.arpa');
}

/** The host part of an http(s) URL, without port (mirrors `url_host`). */
export function urlHost(url: string): string | null {
  const rest = url.split('://')[1];
  if (rest === undefined) return null;
  const authority = (rest.split('/')[0] ?? '').split('@').pop() ?? '';
  if (authority.startsWith('[')) return authority.slice(1).split(']')[0] ?? null;
  return authority.split(':')[0] ?? null;
}

/** How exposed an address is (mirrors `plain_http` + the `PlainHttp` enum of the backend). */
export function plainHttpRisk(url: string): PlainHttp {
  if (!url.toLowerCase().startsWith('http://')) return 'https';
  const host = urlHost(url) ?? '';
  if (isLoopbackHost(host)) return 'loopback';
  return isPrivateHost(host) ? 'private' : 'public';
}

/** True when the address needs the user's confirmation before a password or token is sent. */
export function needsPlainHttpConfirmation(url: string): boolean {
  const r = plainHttpRisk(url);
  return r === 'private' || r === 'public';
}

/** The warning text for a plain-http address, or null when there is nothing to warn about. */
export function plainHttpWarning(risk: PlainHttp): string | null {
  switch (risk) {
    case 'private':
      return 'This address uses plain http://. Your password and login token would be sent unencrypted and could be read by anyone on that network.';
    case 'public':
      return 'This address uses plain http:// over the internet. Your password and login token would be sent unencrypted and could be read on the way. Prefer an https:// address.';
    default:
      return null;
  }
}

/**
 * What the user typed into a server address box, as the API base URL (mirrors
 * `normalize_base_url` in src-tauri/src/profiles.rs, which is what actually stores it).
 * A bare `host:port` gets `http://` when it is this computer or a local network and
 * `https://` otherwise. Returns null when it cannot be an address.
 */
export function normalizeServerUrl(input: string): string | null {
  let s = input.trim();
  if (!s) return null;
  if (!s.includes('://')) s = `${isPrivateHost(urlHost(`x://${s}`) ?? '') ? 'http' : 'https'}://${s}`;
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

/** The backend refuses to send a password or token to this profile until the user agrees. */
export function isPlainHttpBlocked(p: Pick<ProfileView, 'plain_http' | 'plain_http_confirmed'>): boolean {
  return (p.plain_http === 'private' || p.plain_http === 'public') && !p.plain_http_confirmed;
}
