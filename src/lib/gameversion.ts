import type { GameVersion } from './types';

/** The pseudo version of the official mode (mirrors `OFFICIAL` in src-tauri/src/version.rs). */
export const OFFICIAL = 'official';

export function isOfficial(version: string | null | undefined): boolean {
  return version === OFFICIAL;
}

/** Name in the dropdown and headings. */
export function versionLabel(v: Pick<GameVersion, 'version' | 'official'> | string): string {
  const version = typeof v === 'string' ? v : v.version;
  return isOfficial(version) ? 'Official (read-only)' : version;
}

/** Second line of a dropdown entry. */
export function versionSub(v: GameVersion): string {
  if (v.official) return 'Official servers, the current game build, hook only observes';
  return v.newest ? 'Private server, newest supported build' : 'Private server';
}

/**
 * How a server's reported client version compares with the selected game version:
 * - `ok`: same version
 * - `other`: a version this launcher supports: switch the dropdown to play there
 * - `unsupported`: a version this launcher does not know (older/newer server than the launcher)
 * - `unknown`: nothing reported yet
 */
export type Compat =
  | { kind: 'ok' }
  | { kind: 'other'; version: string }
  | { kind: 'unsupported'; version: string }
  | { kind: 'unknown' };

export function serverCompat(reported: string | null | undefined, selected: string, known: GameVersion[]): Compat {
  if (!reported) return { kind: 'unknown' };
  if (reported === selected) return { kind: 'ok' };
  return known.some((v) => !v.official && v.version === reported)
    ? { kind: 'other', version: reported }
    : { kind: 'unsupported', version: reported };
}

/** Why Play is blocked by the server's version, or null. A server of a version this launcher
 *  does not know only warns (the check can be wrong); one of another supported version blocks. */
export function compatBlocker(c: Compat): string | null {
  return c.kind === 'other' ? `This server needs client ${c.version}: switch the game version (top of the window)` : null;
}

/** Private versions only (the official entry is not a server target). */
export function privateVersions(list: GameVersion[]): GameVersion[] {
  return list.filter((v) => !v.official);
}
