import { ipc } from './ipc';
import { initialInstall, reduce, type InstallEvent } from './install';
import { listenInstall, listenPlay } from './events';
import { run, toasts } from './notify.svelte';
import type { AppInfo, InstallStatus, PlayState, ProfileView, Settings } from './types';

class AppState {
  info = $state<AppInfo | null>(null);
  profiles = $state<ProfileView[]>([]);
  settings = $state<Settings | null>(null);
  status = $state<InstallStatus | null>(null);
  play = $state<PlayState>({ state: 'idle' });
  install = $state(initialInstall());
  ready = $state(false);

  selected = $derived(this.profiles.find((p) => p.id === this.settings?.selected_profile) ?? this.profiles[0] ?? null);

  async init(): Promise<void> {
    await listenInstall((ev) => this.applyInstall(ev));
    await listenPlay(
      (s) => (this.play = s),
      (text) => toasts.show('error', text),
    );
    await Promise.all([this.refreshProfiles(), this.refreshSettings(), this.refreshStatus()]);
    this.info = (await run(() => ipc.appInfo())) ?? null;
    this.play = (await run(() => ipc.playState())) ?? this.play;
    this.ready = true;
  }

  applyInstall(ev: InstallEvent): void {
    this.install = reduce(this.install, ev);
    if (ev.type === 'state' && (ev.state === 'done' || ev.state === 'failed' || ev.state === 'cancelled')) {
      void this.refreshStatus();
    }
  }

  async refreshProfiles(): Promise<void> {
    this.profiles = (await run(() => ipc.profiles())) ?? this.profiles;
  }

  async refreshSettings(): Promise<void> {
    const s = await run(() => ipc.settings());
    if (s) {
      this.settings = s;
      document.documentElement.dataset.theme = s.theme;
    }
  }

  async refreshStatus(): Promise<void> {
    this.status = (await run(() => ipc.installStatus())) ?? this.status;
  }

  async select(id: string): Promise<void> {
    if (!this.settings) return;
    const s = await run(() => ipc.setSettings({ ...this.settings!, selected_profile: id }));
    if (s) this.settings = s;
  }

  /** Why Play is not possible right now, or null. */
  get playBlocker(): string | null {
    if (!this.status?.installed) return 'Install the game first (Install page)';
    if (!this.selected) return 'Add a server first (Servers page)';
    if (!this.selected.logged_in) return 'Log in to the server first (Account page)';
    if (this.play.state === 'preparing' || this.play.state === 'running') return 'The game is already starting or running';
    return null;
  }
}

export const app = new AppState();
