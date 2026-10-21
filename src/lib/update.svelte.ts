import { check } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';
import { checkForUpdate, installUpdate, type UpdateDeps, type UpdateHandle, type UpdateState } from './update';

const deps: UpdateDeps = { check, relaunch };

class Updater {
  state = $state<UpdateState>({ kind: 'idle' });
  private handle: UpdateHandle | null = null;

  get busy(): boolean {
    return ['checking', 'installing', 'restarting'].includes(this.state.kind);
  }

  /** Looks for a newer launcher release. Never throws; a failed check is the quiet `unavailable` state. */
  async check(): Promise<void> {
    if (this.busy) return;
    this.state = { kind: 'checking' };
    const r = await checkForUpdate(deps);
    this.handle = r.handle;
    this.state = r.state;
  }

  /** Downloads, verifies (signature), installs and restarts. */
  async install(): Promise<void> {
    if (!this.handle || this.busy) return;
    await installUpdate(this.handle, deps, (s) => (this.state = s));
  }
}

export const updater = new Updater();
