import { errorText, ipc } from './ipc';
import { initialInstall, reduce, type InstallEvent } from './install';
import { listenHook, listenInstall, listenPlay } from './events';
import { hookReady } from './hook';
import { run, toasts } from './notify.svelte';
import { updater } from './update.svelte';
import { ask } from './confirm.svelte';
import { isPlainHttpBlocked } from './url';
import { RateTracker } from './rate.svelte';
import { installBytesDone, installBytesTotal } from './transfer';
import { compatBlocker, isOfficial, serverCompat, versionLabel, type Compat } from './gameversion';
import type { NewsFeed, AppInfo, GameVersion, HookCheck, HookStatus, InstallStatus, PlayState, ProfileView, ServerInfo, Settings, SpaceCheck } from './types';

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
  /** Round-trip time of each server's last status request, measured by the launcher (ms). */
  pings = $state<Record<string, number>>({});
  switching = $state(false);

  /** Speed and time left of the game download and of the hook download. */
  installRate = new RateTracker();
  hookRate = new RateTracker();
  /** Project news for Home (loaded once per start, and when the user presses refresh). */
  news = $state<NewsFeed | null>(null);
  newsLoading = $state(false);
  /** The disk-space check shown before and during an install. */
  space = $state<SpaceCheck | null>(null);

  /** The client hook: what Play would use, the newest release GitHub has, and a running download. */
  hook = $state<HookStatus | null>(null);
  hookCheck = $state<HookCheck | null>(null);
  hookTask = $state<'' | 'check' | 'install'>('');
  hookProgress = $state<{ done: number; total: number } | null>(null);
  hookStep = $state('');
  hookError = $state('');

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
    await listenHook({
      onProgress: (p) => {
        this.hookProgress = { done: p.done, total: p.total };
        this.hookRate.feed(p.done, p.total);
      },
      onStep: (s) => (this.hookStep = s),
    });
    await Promise.all([this.refreshProfiles(), this.refreshSettings(), this.refreshStatus(), this.refreshHook()]);
    this.info = (await run(() => ipc.appInfo())) ?? null;
    this.play = (await run(() => ipc.playState())) ?? this.play;
    this.ready = true;
    // Roles may have changed on the servers since the last run: re-read them quietly.
    void this.refreshRoles();
    void this.checkServers();
    void this.loadNews();
    // A newer hook release: look once at start, quietly.
    if (this.hook?.installed) void this.checkHook(true);
    // Launcher self-update: look once at start; a failure is a quiet state, never a toast.
    void updater.check();
  }

  applyInstall(ev: InstallEvent): void {
    this.install = reduce(this.install, ev);
    if (ev.type === 'state' && ev.state === 'fetching_tool') this.installRate.reset();
    if (ev.type === 'progress' && ev.phase === 'depot' && this.status) {
      const total = this.status.required_bytes > 0 ? installBytesTotal(this.status.depots) : 0;
      this.installRate.feed(installBytesDone(this.status.depots, this.install.depots), total);
    }
    if (ev.type === 'state' && (ev.state === 'done' || ev.state === 'failed' || ev.state === 'cancelled' || ev.state === 'paused')) {
      this.installRate.reset();
      void this.refreshStatus();
      void this.refreshInfo();
    }
  }

  async refreshSpace(): Promise<void> {
    try {
      this.space = await ipc.installSpace();
    } catch {
      this.space = null;
    }
  }

  /** News from GitHub; quiet: the backend answers with the saved copy when offline. */
  async loadNews(force = false): Promise<void> {
    if (this.newsLoading) return;
    this.newsLoading = true;
    try {
      this.news = await ipc.newsGet(force);
    } catch {
      /* keep what is shown */
    }
    this.newsLoading = false;
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
      const st = await ipc.serverStatus(id);
      this.infos[id] = st.info;
      this.pings[id] = st.ping_ms;
      return st.info;
    } catch (e) {
      this.infos[id] = e instanceof Error ? e.message : String(e);
      delete this.pings[id];
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
      toasts.show('error', 'This server uses plain http://. Allow it first (Servers), then try again.');
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

  async refreshHook(): Promise<void> {
    try {
      this.hook = await ipc.hookStatus();
    } catch (e) {
      // Not worth a toast: the panel shows the hook as unknown and Play says why it is blocked.
      this.hookError = errorText(e);
    }
  }

  /** Asks GitHub for the newest hook release for the selected game version. `quiet`: no error text (start-up). */
  async checkHook(quiet = false): Promise<void> {
    if (this.hookTask) return;
    this.hookTask = 'check';
    if (!quiet) this.hookError = '';
    try {
      this.hookCheck = await ipc.hookCheck();
    } catch (e) {
      this.hookCheck = null;
      if (!quiet) this.hookError = errorText(e);
    }
    this.hookTask = '';
  }

  /** Downloads, verifies and installs the newest hook release (or tells that it is current). */
  async installHook(): Promise<void> {
    if (this.hookTask) return;
    this.hookTask = 'install';
    this.hookError = '';
    this.hookProgress = null;
    this.hookRate.reset();
    this.hookStep = '';
    try {
      const out = await ipc.hookInstallLatest();
      toasts.show('ok', out.source === 'current' ? `Client hook ${out.hook.version} is the newest` : `Client hook ${out.hook.version} installed`);
      await this.refreshHook();
      this.hookCheck = this.hookCheck && { ...this.hookCheck, update_available: false };
      void this.checkHook(true);
    } catch (e) {
      this.hookError = errorText(e);
    }
    this.hookTask = '';
    this.hookProgress = null;
    this.hookRate.reset();
    this.hookStep = '';
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
      void this.refreshHook();
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
        this.hookCheck = null;
        this.hookError = '';
        await Promise.all([this.refreshSettings(), this.refreshStatus(), this.refreshInfo(), this.refreshHook()]);
        if (this.hook?.installed) void this.checkHook(true);
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
    if (!this.status?.installed) return 'Install the game first (Library)';
    if (this.hook && !hookReady(this.hook)) return 'Get the client hook first (Library)';
    if (this.play.state === 'preparing' || this.play.state === 'running') return 'The game is already starting or running';
    if (this.official) return null;
    if (!this.selected) return 'Add a server first (Servers)';
    if (isPlainHttpBlocked(this.selected)) return 'Allow plain http for this server first (Servers)';
    if (!this.selected.logged_in) return 'Log in to the server first (Servers)';
    return compatBlocker(this.compat);
  }
}

export const app = new AppState();
