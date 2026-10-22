/** Hash routes (`#/accounts`, `#/characters/12?x=y`), so the server's index.html fallback is all it takes. */

export interface Route {
  /** Segments, e.g. `['characters', '12']`. */
  segments: string[];
  query: URLSearchParams;
}

export function parseHash(hash: string): Route {
  const raw = hash.replace(/^#/, '');
  const [path = '', qs = ''] = raw.split('?', 2) as [string, string?];
  const segments = path.split('/').filter(Boolean).map(decodeURIComponent);
  return { segments, query: new URLSearchParams(qs) };
}

export function hrefFor(path: string, query?: Record<string, string | undefined>): string {
  const q = new URLSearchParams();
  for (const [k, v] of Object.entries(query ?? {})) if (v) q.set(k, v);
  const qs = q.toString();
  return `#/${path.replace(/^\/+/, '')}${qs ? `?${qs}` : ''}`;
}

/** Pages that were merged into others keep working as links (`#/account` is now Servers). */
const LEGACY: Record<string, string> = { account: 'servers', install: 'library', mods: 'library' };

export function pageFor(segment: string | undefined): string {
  if (!segment) return 'home';
  return LEGACY[segment] ?? segment;
}

class Router {
  route = $state<Route>(parseHash(typeof location === 'undefined' ? '' : location.hash));

  constructor() {
    if (typeof window !== 'undefined') {
      window.addEventListener('hashchange', () => {
        this.route = parseHash(location.hash);
      });
    }
  }

  get page(): string {
    return pageFor(this.route.segments[0]);
  }

  go(path: string, query?: Record<string, string | undefined>): void {
    location.hash = hrefFor(path, query);
  }
}

export const router = new Router();
