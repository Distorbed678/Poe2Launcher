import { ipc } from './ipc';
import { initialInstall, reduce, type InstallEvent } from './install';
import { listenInstall, listenPlay } from './events';
import { run, toasts } from './notify.svelte';
import { updater } from './update.svelte';
import { ask } from './confirm.svelte';
import { isPlainHttpBlocked } from './url';
import { compatBlocker, isOfficial, serverCompat, versionLabel, type Compat } from './gameversion';
import type { AppInfo, GameVersion, InstallStatus, PlayState, ProfileView, ServerInfo, Settings } from './types';

class AppState {
  info = $state<AppInfo | null>(null);
  profiles = $state<ProfileView[]>([]);
  settings = $state<Settings | null>(null);
  status = $state<InstallStatus | null>(null);
  play = $state<PlayState>({ state: 'idle' });
  install = $state(initialInstall());
  ready = $state(false);
  /** What each server's `GET /server` last said (or the error), by profile id. */
  infos = $state<Record<string, ServerInfo | string>>({});
  switching = $state(false);

  selected = $derived(this.profiles.find((p) => p.id === this.settings?.selected_profile) ?? this.profiles[0] ?? null);
  /** The selected game version (`official` for the official mode). */
  version = $derived(this.info?.client_version ?? '');
  official = $derived(isOfficial(this.info?.client_version));
  versions = $derived<GameVersion[]>(this.info?.game_versions ?? []);

  /** The selected server's client version against the selected game version. */
  compat = $derived.by<Compat>(() => {
    const sel = this.selected;
    const info = sel ? this.infos[sel.id] : undefined;
    return serverCompat(typeof info === 'object' ? info.version : null, this.version, this.versions);
  });

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
    // Roles may have changed on the servers since the last run: re-read them quietly.
    void this.refreshRoles();
    void this.checkServers();
    // Launcher self-update: look once at start; a failure is a quiet state, never a toast.
    void updater.check();
  }

  applyInstall(ev: InstallEvent): void {
    this.install = reduce(this.install, ev);
    if (ev.type === 'state' && (ev.state === 'done' || ev.state === 'failed' || ev.state === 'cancelled')) {
      void this.refreshStatus();
      void this.refreshInfo();
    }
  }

  async refreshInfo(): Promise<void> {
    this.info = (await run(() => ipc.appInfo())) ?? this.info;
  }

  async refreshProfiles(): Promise<void> {
    this.profiles = (await run(() => ipc.profiles())) ?? this.profiles;
  }

  /** Asks one server what it is (and which client version it needs). Never toasts; the result is stored. */
  async checkServer(id: string): Promise<ServerInfo | null> {
    try {
      const info = await ipc.serverInfo(id);
      this.infos[id] = info;
      return info;
    } catch (e) {
      this.infos[id] = e instanceof Error ? e.message : String(e);
      return null;
    }
  }

  /** Quietly checks every server that is allowed to be contacted, so the version chips are right. */
  async checkServers(): Promise<void> {
    const ids = this.profiles.filter((p) => !isPlainHttpBlocked(p)).map((p) => p.id);
    await Promise.allSettled(ids.map((id) => this.checkServer(id)));
  }

  /** Re-reads each logged-in account's role (`GET /me`). Quiet: an unreachable server just keeps the old role. */
  async refreshRoles(): Promise<void> {
    const ids = this.profiles.filter((p) => p.logged_in && !isPlainHttpBlocked(p)).map((p) => p.id);
    if (ids.length === 0) return;
    await Promise.allSettled(ids.map((id) => ipc.accountRefresh(id)));
    try {
      this.profiles = await ipc.profiles();
    } catch {
      /* keep what is shown */
    }
  }

  /** "Open web admin": the backend checks the role, trades the token and opens the system browser. */
  async openWebAdmin(id: string): Promise<void> {
    const p = this.profiles.find((x) => x.id === id);
    if (!p) return;
    if (isPlainHttpBlocked(p)) {
      toasts.show('error', 'This server uses plain http://. Allow it first (Servers page), then try again.');
      return;
    }
    await run(() => ipc.webAdminOpen(id), 'Opening the web admin in your browser...');
    await this.refreshProfiles();
  }

  async refreshSettings(): Promise<void> {
    const s = await run(() => ipc.settings());
    if (s) {
      this.settings = s;
      document.documentElement.dataset.theme = s.theme;
      document.documentElement.dataset.frame = s.custom_frame ? 'custom' : 'system';
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

  /** Saves a change to the settings (whole object, as the backend takes it) and applies the visible parts. */
  async saveSettings(next: Settings, ok?: string): Promise<Settings | undefined> {
    const s = await run(() => ipc.setSettings({ ...next }), ok);
    if (s) {
      this.settings = s;
      document.documentElement.dataset.theme = s.theme;
      document.documentElement.dataset.frame = s.custom_frame ? 'custom' : 'system';
    }
    return s;
  }

  /** The version dropdown: everything version-dependent (install dir, status, mods, host, play) follows. */
  async switchVersion(version: string): Promise<void> {
    if (version === this.version || this.switching) return;
    this.switching = true;
    try {
      const list = await run(() => ipc.setGameVersion(version));
      if (list) {
        await Promise.all([this.refreshSettings(), this.refreshStatus(), this.refreshInfo()]);
        // A fresh install log and verification belong to the other version.
        this.install = initialInstall();
        toasts.show('ok', `Game version: ${versionLabel(version)}`);
        void this.checkServers();
      }
    } finally {
      this.switching = false;
    }
  }

  /** Play. The official mode asks once for the risk to be accepted. */
  async startPlay(): Promise<void> {
    if (this.official) {
      if (!this.settings?.official_ack) {
        const ok = await ask({
          title: 'Play on the official servers (read-only)',
          message:
            'The hook only observes: it logs packets and the game state, and sends, changes or injects nothing. But it is still a foreign DLL inside the game process, patching game code in memory, and the game or its anti-cheat can detect it. Your account is what is at risk: a warning or a ban is possible. Use an account you can afford to lose.\n\n' +
            'The launcher never logs in for you here: you sign in to Steam and to the game as usual. If the game was updated and the hook does not know its build, the hook switches its own features off (expected); nothing breaks, you just get no capture.',
          acknowledge: 'I understand the risk and want to play on the official servers with the read-only hook',
          confirmLabel: 'Accept and play',
        });
        if (!ok || !this.settings) return;
        const s = await this.saveSettings({ ...this.settings, official_ack: true });
        if (!s) return;
      }
      await run(() => ipc.play(null));
      return;
    }
    if (!this.selected) return;
    await run(() => ipc.play(this.selected!.id));
  }

  /** Why Play is not possible right now, or null. */
  get playBlocker(): string | null {
    if (!this.status?.installed) return 'Install the game first (Install page)';
    if (this.play.state === 'preparing' || this.play.state === 'running') return 'The game is already starting or running';
    if (this.official) return null;
    if (!this.selected) return 'Add a server first (Servers page)';
    if (isPlainHttpBlocked(this.selected)) return 'Allow plain http for this server first (Servers page)';
    if (!this.selected.logged_in) return 'Log in to the server first (Account page)';
    return compatBlocker(this.compat);
  }
}

export const app = new AppState();
