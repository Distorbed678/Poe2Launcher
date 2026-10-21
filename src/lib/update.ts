// Launcher self-update (Tauri updater plugin: the GitHub release's latest.json, signed).
// The logic is here and takes the plugin as a parameter, so it is testable without Tauri.

export interface UpdateHandle {
  version: string;
  currentVersion: string;
  date?: string;
  body?: string;
  downloadAndInstall(onEvent?: (e: DownloadEvent) => void): Promise<void>;
}

export type DownloadEvent =
  | { event: 'Started'; data: { contentLength?: number } }
  | { event: 'Progress'; data: { chunkLength: number } }
  | { event: 'Finished' };

export type UpdateState =
  | { kind: 'idle' }
  | { kind: 'checking' }
  | { kind: 'current' }
  /** No usable update info (no release yet, repository private, offline). Not an error for the user. */
  | { kind: 'unavailable'; detail: string }
  | { kind: 'available'; version: string; notes: string; date: string | null }
  | { kind: 'installing'; done: number; total: number | null; phase: 'downloading' | 'installing' }
  | { kind: 'restarting' }
  | { kind: 'error'; message: string };

export interface UpdateDeps {
  check(): Promise<UpdateHandle | null>;
  relaunch(): Promise<void>;
}

function detailOf(e: unknown): string {
  if (typeof e === 'string') return e;
  if (e && typeof e === 'object' && 'message' in e && typeof e.message === 'string') return e.message;
  return String(e);
}

/** One update check. Any failure of the check itself (404 on a private repo or before the first release, offline, bad JSON) is `unavailable`. */
export async function checkForUpdate(
  deps: UpdateDeps,
): Promise<{ state: UpdateState; handle: UpdateHandle | null }> {
  try {
    const u = await deps.check();
    if (!u) return { state: { kind: 'current' }, handle: null };
    return {
      state: { kind: 'available', version: u.version, notes: (u.body ?? '').trim(), date: u.date ?? null },
      handle: u,
    };
  } catch (e) {
    return { state: { kind: 'unavailable', detail: detailOf(e) }, handle: null };
  }
}

/** Downloads (the plugin verifies the signature against the embedded public key), installs and restarts. Failures are an `error` state. */
export async function installUpdate(
  handle: UpdateHandle,
  deps: UpdateDeps,
  set: (s: UpdateState) => void,
): Promise<void> {
  let done = 0;
  let total: number | null = null;
  set({ kind: 'installing', done, total, phase: 'downloading' });
  try {
    await handle.downloadAndInstall((e) => {
      if (e.event === 'Started') total = e.data.contentLength ?? null;
      else if (e.event === 'Progress') done += e.data.chunkLength;
      set({ kind: 'installing', done, total, phase: e.event === 'Finished' ? 'installing' : 'downloading' });
    });
  } catch (e) {
    set({ kind: 'error', message: `The update could not be installed: ${detailOf(e)}` });
    return;
  }
  set({ kind: 'restarting' });
  try {
    await deps.relaunch();
  } catch (e) {
    set({ kind: 'error', message: `The update is installed; restart the launcher yourself (${detailOf(e)})` });
  }
}
