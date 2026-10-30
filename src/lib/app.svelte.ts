import { errorText, ipc } from './ipc';
import { initialInstall, reduce, type InstallEvent } from './install';
import { listenHook, listenInstall, listenMods, listenPlay } from './events';
import { hookReady } from './hook';
import { run, toasts } from './notify.svelte';
import { updater } from './update.svelte';
import { ask } from './confirm.svelte';
import { RateTracker } from './rate.svelte';
import { installBytesDone, installBytesTotal } from './transfer';
import { compatBlocker, isOfficial, serverCompat, versionLabel, type Compat } from './gameversion';
import { needsDialog, syncBlocker } from './mods';
import { updateCheckDue } from './host';
import type { NewsFeed, ServerUpdateCheck, AppInfo, GameVersion, HookCheck, HookStatus, InstallStatus, PlayState, ProfileView, ServerInfo, Settings, SpaceCheck, SyncPlan } from './types';

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
  /** Is a newer server release on offer for a hosted server (by id)? Quiet; absent until asked. */
  serverUpdates = $state<Record<string, ServerUpdateCheck>>({});
  private serverUpdateLoading = new Set<string>();
  private serverUpdateLast: Record<string, { at: number; game: string }> = {};
  /** The hosted servers' ids (Servers > Host keeps it fresh). */
  hostIds = $state<string[]>([]);
  /** The disk-space check shown before and during an install. */
  space = $state<SpaceCheck | null>(null);

  /** The client hook: what Play would use, the newest release GitHub has, and a running download. */
  hook = $state<HookStatus | null>(null);
  hookCheck = $state<HookCheck | null>(null);
  hookTask = $state<'' | 'check' | 'install'>('');
  hookProgress = $state<{ done: number; total: number } | null>(null);
  hookStep = $state('');
  hookError = $state('');

  /** What each server requires of mods, compared with the mod cache (by profile id); `GET /server` is asked quietly. */
  modPlans = $state<Record<string, SyncPlan>>({});
  /** The Ask-mode dialog: the plan it lists, a running download, the last error. */
  modDialog = $state<{ plan: SyncPlan; busy: boolean; error: string } | null>(null);
  /** Progress of the mod package being downloaded. */
  modProgress = $state<{ id: string; version: string; done: number; total: number } | null>(null);
  private modChecking = new Set<string>();
  private modDialogDone: ((ok: boolean) => void) | null = null;

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
    await listenMods({
      onProgress: (p) => (this.modProgress = p),
      onSynced: (e) => {
        const name = this.profiles.find((p) => p.id === e.profile)?.name ?? 'the server';
        toasts.show('ok', `Mods for ${name}: ${e.mods.join(', ')} downloaded and verified`);
      },
    });
    await Promise.all([this.refreshProfiles(), this.refreshSettings(), this.refreshStatus(), this.refreshHook()]);
    this.info = (await run(() => ipc.appInfo())) ?? null;
    this.play = (await run(() => ipc.playState())) ?? this.play;
    this.ready = true;
    // Roles may have changed on the servers since the last run: re-read them quietly.
    void this.checkServers();
    void this.loadNews();
    // A newer server release for the hosted servers: quiet, once at start (and on Host / Home).
    void this.checkServerUpdates();
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

  /** The first hosted server with an update on offer (Home shows one line for it). */
  get serverUpdate(): ServerUpdateCheck | null {
    return Object.values(this.serverUpdates).find((u) => u.update_available) ?? null;
  }

  /**
   * Looks for a newer server release (a hosted server's installed package against GitHub).
   * Quiet and throttled: at start, and each time Host or Home opens, but GitHub is asked at
   * most every 10 minutes per server and game version unless `force` (a failure is in `error`).
   */
  async checkServerUpdate(id: string, force = false): Promise<void> {
    if (this.official || this.serverUpdateLoading.has(id)) return;
    const now = Math.floor(Date.now() / 1000);
    if (!updateCheckDue(this.serverUpdateLast[id] ?? null, this.version, now, force)) return;
    this.serverUpdateLoading.add(id);
    const game = this.version;
    try {
      this.serverUpdates = { ...this.serverUpdates, [id]: await ipc.hostUpdateCheck(id) };
      this.serverUpdateLast[id] = { at: now, game };
    } catch {
      /* quiet: keep what is shown */
    }
    this.serverUpdateLoading.delete(id);
  }

  /** Every hosted server's update check (quiet, throttled). */
  async checkServerUpdates(force = false): Promise<void> {
    if (this.official) return;
    try {
      this.hostIds = (await ipc.hostList()).map((h) => h.id);
    } catch {
      return;
    }
    await Promise.all(this.hostIds.map((id) => this.checkServerUpdate(id, force)));
  }

  /** A server's package changed (installed, updated): the old answer no longer holds. */
  forgetServerUpdate(id: string): void {
    const { [id]: _gone, ...rest } = this.serverUpdates;
    this.serverUpdates = rest;
    delete this.serverUpdateLast[id];
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

  /** Quietly checks every server, so the version chips are right (`GET /server`: nothing secret). */
  async checkServers(): Promise<void> {
    const ids = this.profiles.map((p) => p.id);
    await Promise.allSettled(ids.map((id) => this.checkServer(id)));
    if (this.selected && ids.includes(this.selected.id)) void this.checkMods(this.selected.id);
  }

  /**
   * Compares what a server requires with the mod cache (the backend also downloads in Auto mode).
   * Quiet: an unreachable server keeps the old plan; nothing happens for the official mode.
   */
  async checkMods(id: string): Promise<SyncPlan | null> {
    if (this.official || this.modChecking.has(id)) return this.modPlans[id] ?? null;
    this.modChecking.add(id);
    try {
      const plan = await ipc.modSyncCheck(id);
      this.modPlans[id] = plan;
      if (plan.applied.length) {
        toasts.show('ok', `Mods for ${plan.profile_name}: ${plan.applied.join(', ')} downloaded and verified`);
      }
      return plan;
    } catch {
      return this.modPlans[id] ?? null;
    } finally {
      this.modChecking.delete(id);
    }
  }

  /** Asks the player (Ask mode) to download the required mods; resolves true once they are in the cache. */
  openModDialog(plan: SyncPlan): Promise<boolean> {
    this.modDialogDone?.(false);
    this.modDialog = { plan, busy: false, error: '' };
    return new Promise((resolve) => (this.modDialogDone = resolve));
  }

  closeModDialog(ok: boolean): void {
    this.modDialog = null;
    this.modProgress = null;
    const done = this.modDialogDone;
    this.modDialogDone = null;
    done?.(ok);
  }

  /** The dialog's Download button: fetches and verifies; on a failure the dialog stays and says why. */
  async downloadMods(): Promise<void> {
    const d = this.modDialog;
    if (!d || d.busy) return;
    d.busy = true;
    d.error = '';
    try {
      const plan = await ipc.modSyncApply(d.plan.profile_id);
      this.modPlans[plan.profile_id] = plan;
      if (plan.errors.length) {
        d.plan = plan;
        d.error = plan.errors.join('\n');
        d.busy = false;
        return;
      }
      if (plan.status === 'ok' || plan.status === 'auto') {
        toasts.show('ok', `Downloaded and verified: ${plan.applied.join(', ') || 'nothing new'}`);
        this.closeModDialog(true);
        return;
      }
      d.plan = plan;
      d.error = plan.blocker ?? 'The mods are still not ready';
    } catch (e) {
      d.error = errorText(e);
    }
    d.busy = false;
  }

  /** A page of the server's web admin in the browser: `admin` (it asks for its own login) or
   * `register` (new accounts). The launcher has no login: accounts belong to the server. */
  async openServerPage(id: string, page: 'admin' | 'register'): Promise<void> {
    await run(() => ipc.serverPageOpen(id, page), page === 'admin' ? 'Opening the web admin in your browser...' : 'Opening the register page in your browser...');
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
    if (s) {
      this.settings = s;
      void this.checkMods(id);
    }
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
        this.serverUpdates = {};
        this.serverUpdateLast = {};
        await Promise.all([this.refreshSettings(), this.refreshStatus(), this.refreshInfo(), this.refreshHook()]);
        if (this.hook?.installed) void this.checkHook(true);
        // A fresh install log and verification belong to the other version.
        this.install = initialInstall();
        toasts.show('ok', `Game version: ${versionLabel(version)}`);
        void this.checkServers();
        void this.checkServerUpdates();
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
    const id = this.selected.id;
    // The server's mods first: Ask shows its dialog, Auto already downloaded them, Off refuses.
    let plan = await this.checkMods(id);
    if (needsDialog(plan) && !(await this.openModDialog(plan!))) return;
    plan = this.modPlans[id];
    const why = syncBlocker(plan);
    if (why) {
      toasts.show('error', why);
      return;
    }
    await run(() => ipc.play(id));
  }

  /** Why Play is not possible right now, or null. */
  get playBlocker(): string | null {
    if (!this.status?.installed) return 'Install the game first (Library)';
    if (this.hook && !hookReady(this.hook)) return 'Get the client hook first (Library)';
    if (this.play.state === 'preparing' || this.play.state === 'running') return 'The game is already starting or running';
    if (this.official) return null;
    if (!this.selected) return 'Add a server first (Servers)';
    return compatBlocker(this.compat) ?? syncBlocker(this.modPlans[this.selected.id]);
  }
}

export const app = new AppState();
