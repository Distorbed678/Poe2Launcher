<script lang="ts">
  import { onDestroy, onMount, tick } from 'svelte';
  import { open } from '@tauri-apps/plugin-dialog';
  import { app } from '../app.svelte';
  import { ipc } from '../ipc';
  import { run, toasts } from '../notify.svelte';
  import { listenHost, type HostProgress } from '../events';
  import { formatBytes, formatDateTime, formatUptime } from '../format';
  import { formatEta, formatSpeed } from '../transfer';
  import { RateTracker } from '../rate.svelte';
  import { ask } from '../confirm.svelte';
  import {
    formFromSettings, isUp, logLevel, playerAddress, portsToForward, pushLog, runLabel, settingsFromForm,
    updateHeadline, updatePlan, validateHostForm, type HostForm,
  } from '../host';
  import Tabs from './Tabs.svelte';
  import Icon from './Icon.svelte';
  import Chip from './Chip.svelte';
  import Spinner from './Spinner.svelte';
  import Progress from './Progress.svelte';
  import Markdown from './Markdown.svelte';
  import Fold from './Fold.svelte';
  import Switch from './Switch.svelte';
  import Sparkline from './Sparkline.svelte';
  import type { BackupStatus, HostMetrics, HostStatus, InstallOutcome, RunState, UpdateOutcome } from '../types';

  // Manage one server hosted on this computer: Overview (run, usage, log), Settings, Program
  // (the server package), Backups.
  let { id, ondelete, onchange }: { id: string; ondelete: () => void; onchange?: () => void } = $props();

  let status = $state<HostStatus | null>(null);
  let form = $state<HostForm | null>(null);
  let saved = $state<HostForm | null>(null);
  let log = $state<string[]>([]);
  let metrics = $state<HostMetrics | null>(null);
  let progress = $state<HostProgress | null>(null);
  let step = $state('');
  let task = $state<'' | 'download' | 'local' | 'setup' | 'start' | 'stop' | 'check' | 'update'>('');
  let pkgError = $state('');
  let outcome = $state<InstallOutcome | null>(null);
  let updated = $state<UpdateOutcome | null>(null);
  let ipNote = $state('');
  const dlRate = new RateTracker();
  let backups = $state<BackupStatus | null>(null);
  let backupBusy = $state<'' | 'create' | 'restore' | 'delete' | 'settings'>('');
  let keepText = $state('5');
  let logEl: HTMLElement | undefined = $state();
  let timer: ReturnType<typeof setInterval> | undefined;
  let unlisten: (() => void) | undefined;

  const configured = $derived(status?.settings.configured ?? false);
  const run_ = $derived<RunState>(status?.run ?? { state: 'stopped' });
  const up = $derived(isUp(run_));
  const ready = $derived(status?.info != null);
  const errors = $derived(form ? validateHostForm(form) : []);
  const rules = $derived(form ? portsToForward(form) : []);
  const hostProfile = $derived(app.profiles.find((p) => p.id === status?.settings.profile_id));
  const isAdmin = $derived(hostProfile?.logged_in === true && hostProfile.role === 'admin');
  const noGame = $derived(status !== null && !status.game_installed);
  const noPackage = $derived(status !== null && status.package === null);
  const dirty = $derived(!!form && !!saved && JSON.stringify(form) !== JSON.stringify(saved));
  const canApply = $derived(!!form && !up && task === '' && !status?.busy && errors.length === 0 && !noGame && !noPackage && (dirty || !configured));
  const upd = $derived(app.serverUpdates[id] ?? null);
  const headline = $derived(updateHeadline(upd));
  const pct = $derived(progress && progress.total > 0 ? Math.min(100, Math.round((progress.done / progress.total) * 100)) : null);

  type View = 'run' | 'settings' | 'program' | 'backups';
  let view = $state<View | ''>('');
  // A new server opens where it needs work: its program, then its settings.
  const shown = $derived<View>(view || (noPackage ? 'program' : !configured ? 'settings' : 'run'));
  const tabs = [
    { id: 'run', label: 'Overview', icon: 'play' as const },
    { id: 'settings', label: 'Settings', icon: 'settings' as const },
    { id: 'program', label: 'Program', icon: 'box' as const },
    { id: 'backups', label: 'Backups', icon: 'database' as const },
  ];
  let openSection = $state({ general: true, network: false, access: false, advanced: false, danger: false });

  async function refresh() {
    try {
      const s = await ipc.hostStatus(id);
      status = s;
      if (!form) {
        form = formFromSettings(s.settings, s.lan_ip);
        saved = formFromSettings(s.settings, s.lan_ip);
        // A fresh server: its proposed values still have to be applied once.
        if (!s.settings.configured) saved = { ...saved, public_host: s.settings.public_host };
      }
    } catch (e) {
      toasts.show('error', e instanceof Error ? e.message : String(e));
    }
  }

  async function refreshMetrics() {
    if (!status || !isUp(status.run)) {
      metrics = null;
      return;
    }
    try {
      metrics = await ipc.hostMetrics(id);
    } catch {
      /* the tiles keep their last values */
    }
  }

  onMount(async () => {
    unlisten = await listenHost(id, {
      onState: (s) => {
        if (status) status = { ...status, run: s, info: s.state === 'running' ? status.info : null };
      },
      onLog: (line) => (log = pushLog(log, line)),
      onProgress: (p) => {
        progress = p;
        dlRate.feed(p.done, p.total);
      },
      onStep: (s) => (step = s),
      onInstalled: () => void refresh(),
    });
    log = (await run(() => ipc.hostLogTail(id, 500))) ?? [];
    await refresh();
    void loadBackups();
    void refreshMetrics();
    // A newer server release: quiet, and not more than every 10 minutes.
    void app.checkServerUpdate(id);
    timer = setInterval(() => {
      void refresh();
      void refreshMetrics();
      if (backups && backups.server_running !== isUp(status?.run ?? { state: 'stopped' })) void loadBackups();
    }, 2500);
  });

  onDestroy(() => {
    if (timer) clearInterval(timer);
    unlisten?.();
  });

  $effect(() => {
    log.length;
    void tick().then(() => {
      if (logEl) logEl.scrollTop = logEl.scrollHeight;
    });
  });

  // --- backups ---------------------------------------------------------------------------------

  async function loadBackups() {
    try {
      const b = await ipc.backupStatus(id);
      backups = b;
      if (backupBusy !== 'settings') keepText = String(b.settings.keep);
    } catch (e) {
      toasts.show('error', e instanceof Error ? e.message : String(e));
    }
  }

  async function backupNow() {
    backupBusy = 'create';
    const b = await run(() => ipc.backupCreate(id), (r) => `Backup saved (${formatBytes(r.size)})`);
    backupBusy = '';
    if (b) await loadBackups();
  }

  async function restoreBackup(name: string, when: string) {
    const yes = await ask({
      title: 'Restore this backup?',
      message: `The server's database is replaced by the backup from ${when}. Accounts, characters and everything else go back to that moment.\n\nThe current database is backed up first (kind "before restore"), so you can undo this by restoring that one.`,
      confirmLabel: 'Restore',
      danger: true,
    });
    if (!yes) return;
    backupBusy = 'restore';
    const pre = await run(() => ipc.backupRestore(id, name), (r) => (r ? 'Restored. The previous database was kept as a backup.' : 'Restored.'));
    backupBusy = '';
    if (pre !== undefined) await loadBackups();
  }

  async function deleteBackup(name: string, when: string) {
    const yes = await ask({ title: 'Delete this backup?', message: `The backup from ${when} is deleted for good.`, confirmLabel: 'Delete', danger: true });
    if (!yes) return;
    backupBusy = 'delete';
    const r = await run(() => ipc.backupDelete(id, name), 'Backup deleted');
    backupBusy = '';
    if (r !== undefined) await loadBackups();
  }

  async function saveBackupSettings(auto: boolean) {
    const keep = Math.round(Number(keepText));
    if (!Number.isFinite(keep) || keep < 1 || keep > 100) {
      toasts.show('error', 'Keep between 1 and 100 automatic backups');
      return;
    }
    backupBusy = 'settings';
    const s = await run(() => ipc.backupSetSettings(id, auto, keep));
    backupBusy = '';
    if (s) await loadBackups();
  }

  const kindLabel = { manual: 'manual', auto: 'before start', 'pre-restore': 'before restore', 'pre-update': 'before update', other: 'other' } as const;

  // --- the package ---------------------------------------------------------------------------

  async function download() {
    task = 'download';
    pkgError = '';
    progress = null;
    dlRate.reset();
    outcome = null;
    try {
      outcome = await ipc.hostInstallLatest(id);
      toasts.show('ok', `Installed ${outcome.package.name}`);
    } catch (e) {
      pkgError = e instanceof Error ? e.message : String(e);
    }
    task = '';
    progress = null;
    dlRate.reset();
    await refresh();
    onchange?.();
    app.forgetServerUpdate(id);
    void app.checkServerUpdate(id, true);
  }

  async function checkUpdates() {
    task = 'check';
    pkgError = '';
    updated = null;
    await app.checkServerUpdate(id, true);
    task = '';
    const u = app.serverUpdates[id];
    if (u && !u.error && !u.update_available) toasts.show('ok', 'The server is the newest release for your game version');
  }

  async function applyUpdate() {
    const rel = upd?.release;
    if (!rel || task !== '') return;
    if (up) {
      const yes = await ask({
        title: `Update the server to ${rel.version}?`,
        message: `The running server is stopped (players are disconnected), its database is backed up, the new version is installed with your settings kept, and the server starts again. If the new version does not start, the old one is put back.`,
        confirmLabel: 'Update',
      });
      if (!yes) return;
    }
    task = 'update';
    step = 'starting';
    pkgError = '';
    outcome = null;
    updated = null;
    progress = null;
    dlRate.reset();
    try {
      updated = await ipc.hostUpdateApply(id);
      toasts.show('ok', updated.updated ? `Server updated to ${updated.to}` : 'Already the newest release');
    } catch (e) {
      pkgError = e instanceof Error ? e.message : String(e);
    }
    task = '';
    step = '';
    progress = null;
    dlRate.reset();
    await refresh();
    void loadBackups();
    app.forgetServerUpdate(id);
    void app.checkServerUpdate(id, true);
  }

  async function pickLocal(directory: boolean) {
    const p = await open(
      directory
        ? { directory: true }
        : { directory: false, filters: [{ name: 'Server package', extensions: ['gz', 'tgz', 'zip'] }] },
    );
    if (typeof p !== 'string') return;
    task = 'local';
    pkgError = '';
    outcome = null;
    try {
      outcome = await ipc.hostInstallLocal(id, p);
      toasts.show('ok', `Installed ${outcome.package.name}`);
    } catch (e) {
      pkgError = e instanceof Error ? e.message : String(e);
    }
    task = '';
    await refresh();
    onchange?.();
    app.forgetServerUpdate(id);
    void app.checkServerUpdate(id, true);
  }

  // --- network helpers -------------------------------------------------------------------------

  function useLan() {
    if (form && status?.lan_ip) form.public_host = status.lan_ip;
  }

  async function lookupPublicIp() {
    ipNote = 'Asking api.ipify.org...';
    try {
      const ip = await ipc.hostPublicIp();
      if (form) form.public_host = ip;
      ipNote = `Your public address is ${ip}. Check that it is the one players should use.`;
    } catch (e) {
      ipNote = e instanceof Error ? e.message : String(e);
    }
  }

  // --- settings, start, stop -------------------------------------------------------------------

  async function apply() {
    if (!form || !status || !canApply) return;
    task = 'setup';
    step = 'writing the config';
    const first = !configured;
    const res = await run(() => ipc.hostSetup(id, { settings: settingsFromForm(status!.settings, form!) }));
    step = '';
    task = '';
    if (res) {
      toasts.show('ok', first ? `${form.server_name} is set up. Start it, then open its web admin to create the admin account.` : 'Settings applied');
      await Promise.all([app.refreshProfiles(), app.refreshSettings()]);
      form = null;
      await refresh();
      onchange?.();
      if (first) view = 'run';
    }
  }

  function discard() {
    if (saved) form = { ...saved };
  }

  async function start() {
    task = 'start';
    await run(() => ipc.hostStart(id));
    task = '';
    await refresh();
    onchange?.();
  }

  async function stop() {
    task = 'stop';
    await run(() => ipc.hostStop(id));
    task = '';
    await refresh();
    onchange?.();
  }

  async function autostart(on: boolean) {
    const s = await run(() => ipc.hostSetAutostart(id, on), on ? 'Starts with the launcher' : 'No longer starts with the launcher');
    if (s && status) status = { ...status, settings: s };
  }

  function openWebAdmin() {
    void run(() => ipc.hostWebAdminOpen(id), 'Opening the web admin in your browser...');
  }

  const rate = (bps: number | null) => (bps === null ? '-' : `${formatBytes(bps)}/s`);
</script>

{#if status && !status.platform.match(/^(linux|windows)$/)}
  <p class="muted">Hosting from the launcher works on Linux and Windows only.</p>
{:else if status && form}
  <Tabs {tabs} value={shown} label="Your server" onchange={(v) => (view = v as View)} />
  <div class="stack">
    {#if shown === 'run'}
      <section class="card">
        <div class="runbar">
          <Chip tone={ready ? 'ok' : up ? 'warn' : run_.state === 'exited' && run_.code !== 0 && run_.code !== null ? 'bad' : 'neutral'} dot>
            {runLabel(run_, ready)}
          </Chip>
          {#if status.info}<span class="muted">{status.info.online} online</span>{/if}
          <span class="grow"></span>
          <button type="button" class="btn" disabled={!ready} onclick={openWebAdmin} title={ready ? 'Opens the web admin in your browser' : 'Start the server first'}>
            Open web admin <Icon name="external" size={13} />
          </button>
          {#if up}
            <button type="button" class="btn danger" disabled={task === 'stop' || run_.state === 'stopping'} onclick={stop}>{#if task === 'stop'}<Spinner size={13} />{/if} Stop</button>
          {:else}
            <button type="button" class="btn primary" disabled={!configured || task !== '' || noGame} onclick={start}>{#if task === 'start'}<Spinner size={13} />{/if} Start</button>
          {/if}
        </div>
        {#if !configured}
          <div class="notice info">
            <span class="notice-title"><Icon name="info" size={16} /> Not set up yet</span>
            <p>
              {#if noPackage}First get its <button type="button" class="link" onclick={() => (view = 'program')}>program</button>, then apply its{:else}Apply its{/if}
              <button type="button" class="link" onclick={() => (view = 'settings')}>settings</button>.
            </p>
          </div>
        {:else if ready && !isAdmin}
          <div class="notice accent">
            <span class="notice-title"><Icon name="shield" size={16} /> Admin account</span>
            <p>A new server's first admin is created in its web admin: <button type="button" class="link" onclick={openWebAdmin}>open it</button> and pick a name and password. Then log in on the Servers page.</p>
          </div>
        {/if}
        {#if noGame}<p class="err">The game is not installed: <a href="#/library">install it first</a>.</p>{/if}
      </section>

      {#if up}
        <section class="card">
          <div class="card-head"><span class="head-icon"><Icon name="chart" size={16} /></span><h2>Usage</h2></div>
          <div class="tiles">
            <div class="tile">
              <small>CPU</small><b>{metrics?.cpu_pct != null ? `${metrics.cpu_pct.toFixed(0)} %` : '-'}</b>
              <Sparkline values={metrics?.cpu_history ?? []} />
            </div>
            <div class="tile">
              <small>Memory</small><b>{metrics?.rss_bytes != null ? formatBytes(metrics.rss_bytes) : '-'}</b>
              <Sparkline values={metrics?.rss_history ?? []} color="var(--info)" />
            </div>
            <div class="tile">
              <small>Tick time</small><b>{metrics?.tick_avg_ms != null ? `${metrics.tick_avg_ms.toFixed(1)} ms` : '-'}</b>
              <i>{metrics?.tick_max_ms != null ? `longest ${metrics.tick_max_ms.toFixed(1)} ms · target 33 ms` : 'no frames while nobody plays'}</i>
            </div>
            <div class="tile">
              <small>Network</small><b>{rate(metrics?.net_in_bps != null && metrics.net_out_bps != null ? metrics.net_in_bps + metrics.net_out_bps : null)}</b>
              <i>in {rate(metrics?.net_in_bps ?? null)} · out {rate(metrics?.net_out_bps ?? null)}</i>
            </div>
            <div class="tile">
              <small>Uptime</small><b>{metrics?.uptime_secs != null ? formatUptime(metrics.uptime_secs) : status.info ? formatUptime(status.info.uptime_secs) : '-'}</b>
              <i>{metrics?.players != null ? `${metrics.players} in game` : ''}</i>
            </div>
            <div class="tile">
              <small>Version</small><b>{status.package?.server_version ?? '-'}</b>
              <i>game {status.game_version}</i>
            </div>
          </div>
          {#if metrics && !metrics.server_metrics}<p class="hint">Tick time and network come from the server's own metrics (<code>[metrics] enabled</code>; server 0.4.0 or newer).</p>{/if}
        </section>
      {/if}

      <section class="card">
        <div class="card-head"><span class="head-icon"><Icon name="file" size={16} /></span><h2>Server log</h2></div>
        <div class="log" bind:this={logEl} role="log" aria-label="Server log">
          {#if log.length === 0}<span class="muted">Nothing yet. The log appears when the server runs.</span>{/if}
          {#each log as line, i (i)}
            <div class={logLevel(line)}>{line}</div>
          {/each}
        </div>
        <p class="hint">Also written to <code>{status.log_file}</code> (replaced at each start).</p>
      </section>
    {/if}

    {#if shown === 'settings'}
      <form class="settings" onsubmit={(e) => { e.preventDefault(); void apply(); }}>
        <Fold title="General" bind:open={openSection.general} summary={`${form.server_name}${status.settings.autostart ? ' · starts with the launcher' : ''}`}>
          <label class="field"><span>Server name (shown to players)</span><input bind:value={form.server_name} maxlength="64" /></label>
          <div class="toggle">
            <Switch checked={status.settings.autostart} disabled={!configured} onchange={autostart}>
              <b>Start with the launcher</b>
              <small>Starts this server each time the launcher opens. Closing the launcher stops it; an admin's restart from the web admin starts it again by itself.</small>
            </Switch>
          </div>
          {#if !configured}<p class="hint">Available once the settings are applied.</p>{/if}
        </Fold>

        <Fold title="Network" bind:open={openSection.network} summary={`${form.public_host} · ports ${form.login_port} / ${form.instance_port} / ${form.web_port}`}>
          <label class="field">
            <span>Public host: the address players use to reach this computer</span>
            <input bind:value={form.public_host} placeholder="192.168.1.20 or play.example.com" />
          </label>
          <div class="row">
            <button type="button" class="btn sm" disabled={!status.lan_ip} onclick={useLan}>Use my LAN address{status.lan_ip ? ` (${status.lan_ip})` : ''}</button>
            <button type="button" class="btn sm" onclick={lookupPublicIp} title="Contacts api.ipify.org once, only when you click">Look up my public IP</button>
          </div>
          {#if ipNote}<p class="hint">{ipNote}</p>{/if}
          <p class="hint">A LAN party needs the LAN address. Over the internet use your public IP or a host name and forward the ports below in your router. The lookup asks a web service (api.ipify.org) and only happens when you press the button.</p>
          <div class="ports">
            <label class="field"><span>Login port (TCP)</span><input type="number" min="1024" max="65535" bind:value={form.login_port} /></label>
            <label class="field"><span>Game port (TCP + UDP)</span><input type="number" min="1024" max="65535" bind:value={form.instance_port} /></label>
            <label class="field"><span>Web port (TCP)</span><input type="number" min="1024" max="65535" bind:value={form.web_port} /></label>
          </div>
          <div class="notice info forward">
            <span class="notice-title"><Icon name="globe" size={16} /> Port forwarding <span class="hint">only for players outside your network</span></span>
            <ul>{#each rules as r (r.port)}<li><code>{r.port}</code> {r.proto}: {r.what}</li>{/each}</ul>
            <p class="hint">Players add the server as <code>{playerAddress(form)}</code>. <button type="button" class="link" onclick={() => ipc.hostOpenGuide()}>Open the hosting guide</button></p>
          </div>
        </Fold>

        <Fold title="Access" bind:open={openSection.access} summary={`registration ${form.open_registration ? 'open' : 'closed'} · web API ${form.web_public ? 'reachable from other computers' : 'this computer only'}`}>
          <div class="toggle">
            <Switch bind:checked={form.open_registration}>
              <b>Anyone who knows the address can create an account</b>
              <small>On the server's own register page (its web admin), also opened by the game's "Create account" button. Off: only admins create accounts, in the web admin.</small>
            </Switch>
          </div>
          <div class="toggle">
            <Switch bind:checked={form.web_public}>
              <b>Let other computers use this server's web API</b>
              <small>Players need it to log in with the launcher and to register. It uses plain http, so passwords travel unencrypted: fine on a trusted LAN, otherwise put a TLS proxy in front (hosting guide). Off: only this computer can reach it.</small>
            </Switch>
          </div>
        </Fold>

        <Fold title="Advanced" bind:open={openSection.advanced} summary={`cluster id ${form.cluster_id}`}>
          <label class="field">
            <span>Cluster id (login tokens belong to it; keep it when you re-install)</span>
            <input bind:value={form.cluster_id} maxlength="32" />
          </label>
          <p class="hint">Folder: <code>{status.folder}</code></p>
        </Fold>

        <Fold title="Danger zone" danger bind:open={openSection.danger} summary="delete this server">
          <div class="row danger-row">
            <p class="grow">Deletes the server for good: it is stopped, and its database (accounts, characters), settings, backups and log are removed.</p>
            <button type="button" class="btn danger" onclick={ondelete}><Icon name="trash" size={14} /> Delete server</button>
          </div>
        </Fold>

        {#if errors.length > 0}
          <ul class="errs">{#each errors as m (m)}<li>{m}</li>{/each}</ul>
        {/if}
        {#if dirty || !configured}
          <div class="applybar">
            <span class="muted">{#if task === 'setup'}{step || 'working...'}{:else if up}Stop the server to apply changes{:else if noPackage}Get the server program first (Program){:else if !configured}Not applied yet{:else}Unsaved changes{/if}</span>
            <span class="grow"></span>
            {#if configured}<button type="button" class="btn ghost" onclick={discard}>Discard</button>{/if}
            <button class="btn primary" type="submit" disabled={!canApply}>{#if task === 'setup'}<Spinner size={13} />{/if} Apply settings</button>
          </div>
        {/if}
      </form>
    {/if}

    {#if shown === 'program'}
      <section class="card">
        <div class="card-head"><span class="head-icon"><Icon name="box" size={16} /></span><h2>Server program</h2>{#if status.package}<span class="grow"></span><Chip tone="ok" icon="check">installed</Chip>{/if}</div>
        {#if status.package}
          <dl class="kv">
            <dt>Installed</dt><dd>{status.package.name}</dd>
            <dt>Server version</dt><dd>{status.package.server_version} for game {status.package.game_version}</dd>
            <dt>Folder</dt><dd><code>{status.package.dir}</code></dd>
          </dl>
        {:else}
          <p class="muted">No server program yet. Download the release for your game version ({status.game_version}), or use a package you have.</p>
          {#if status.package_problem}<p class="err">{status.package_problem}</p>{/if}
        {/if}
        {#if headline && upd?.release && status.package}
          <div class="notice accent">
            <span class="notice-title"><Icon name="download" size={16} /> {headline}</span>
            <p class="hint">You have {status.package.server_version}.</p>
            {#if upd.release.notes}<div class="notes"><Markdown source={upd.release.notes} compact /></div>{/if}
            <p class="hint">{updatePlan(up, !!backups?.database_size)}</p>
            <div class="row">
              <button type="button" class="btn primary" disabled={task !== '' || status.busy} onclick={applyUpdate}>
                {#if task === 'update'}<Spinner size={14} />{:else}<Icon name="download" size={14} />{/if} Update to {upd.release.version}
              </button>
              {#if upd.release.html_url}
                <button type="button" class="link" onclick={() => void run(() => ipc.openLink(upd!.release!.html_url!))}>Release page <Icon name="external" size={12} /></button>
              {/if}
            </div>
          </div>
        {:else if status.package && upd?.error}
          <p class="hint">Could not look for a server update: {upd.error}</p>
        {/if}
        <div class="row pkg-actions">
          {#if status.package}
            <button type="button" class="btn" disabled={task !== '' || status.busy} onclick={checkUpdates}>
              {#if task === 'check'}<Spinner size={14} />{:else}<Icon name="refresh" size={14} />{/if} Check for updates
            </button>
          {:else}
            <button type="button" class="btn primary" disabled={task !== '' || up || status.busy} onclick={download}>
              {#if task === 'download'}<Spinner size={14} />{:else}<Icon name="download" size={14} />{/if} Download the latest release
            </button>
          {/if}
          <button type="button" class="btn" disabled={task !== '' || up || status.busy} onclick={() => pickLocal(false)}>Use a package file...</button>
          <button type="button" class="btn" disabled={task !== '' || up || status.busy} onclick={() => pickLocal(true)}>Use a package folder...</button>
        </div>
        {#if task === 'download' || task === 'local' || task === 'update'}
          <p class="hint"><Spinner size={12} /> {step || 'Working...'}</p>
          {#if pct !== null && progress?.phase === 'download'}
            <Progress value={pct} />
            <p class="hint">{formatBytes(progress.done)} of {formatBytes(progress.total)} · {formatSpeed(dlRate.bps)} · {formatEta(dlRate.etaSecs)} left</p>
          {/if}
        {/if}
        {#if pkgError}<p class="err" role="alert" style="white-space: pre-wrap">{pkgError}</p>{/if}
        {#if updated}
          <p class="hint">
            {updated.updated
              ? `Updated ${updated.from} to ${updated.to}${updated.restarted ? '; the server is running again' : ''}. SHA-256 checked${updated.backup ? `; the database was backed up first (${updated.backup})` : ''}.`
              : 'Already the newest release for your game version.'}
          </p>
        {/if}
        {#if outcome}
          <p class="hint">
            {outcome.source === 'current'
              ? 'Already the newest release for your game version.'
              : outcome.source === 'shared'
              ? 'The same release was already installed for another server; it is shared.'
              : outcome.source === 'folder'
              ? 'Copied from a folder (no checksum to compare).'
              : outcome.checksum_verified
                ? `SHA-256 checked: ${outcome.sha256}`
                : `Not checked: there is no .sha256 file next to the package. Its SHA-256 is ${outcome.sha256}.`}
          </p>
        {/if}
        <p class="hint pkg-note">
          Packages come from the project's server releases (<code>poe-server-v…-game-{status.game_version}-{status.platform}-x86_64</code>). Downloads are checked against the release's SHA-256; a mismatch installs nothing. A package holds no game files and is shared by all your servers.
        </p>
        {#if noGame}
          <div class="notice">
            <span class="notice-title"><Icon name="alert" size={16} /> The game is not installed</span>
            <p>The server reads its game data from <code>{status.install_dir}</code>: <a href="#/library">install it first</a>.</p>
          </div>
        {/if}
        {#if !noPackage && !configured}
          <div class="actions"><button type="button" class="btn primary" onclick={() => (view = 'settings')}>Next: settings</button></div>
        {/if}
      </section>
    {/if}

    {#if shown === 'backups'}
      <section class="card">
        <div class="card-head"><span class="head-icon"><Icon name="database" size={16} /></span><h2>Backups</h2>
          <span class="grow"></span>
          <button type="button" class="btn primary" disabled={backupBusy !== '' || !backups?.database_size} onclick={backupNow}
            title={backups?.database_size ? 'Copies the server database now; works while the server runs' : 'The server has no database yet: start it once'}>
            {#if backupBusy === 'create'}<Spinner size={14} />{:else}<Icon name="database" size={14} />{/if} Back up now
          </button>
        </div>
        <p class="hint">
          A backup is a copy of the server's database (accounts and characters), consistent even while the server runs.
          {#if backups?.database_size}The database is {formatBytes(backups.database_size)}.{:else}There is no database yet.{/if}
          Backups live in <code>{backups?.dir ?? '...'}</code>.
        </p>
        <div class="toggle">
          <Switch checked={backups?.settings.auto ?? false} disabled={backupBusy !== ''} onchange={(v) => saveBackupSettings(v)}>
            <b>Back up each time the server starts from the launcher</b>
          </Switch>
          <label class="inline">Keep the last
            <input class="keep" type="number" min="1" max="100" bind:value={keepText} disabled={backupBusy !== ''} onchange={() => saveBackupSettings(backups?.settings.auto ?? false)} aria-label="Automatic backups to keep" />
            automatic ones
          </label>
        </div>
        {#if !backups || backups.backups.length === 0}
          <p class="muted">No backups yet.</p>
        {:else}
          <div class="table-wrap">
            <table>
              <tbody>
                {#each backups.backups as b (b.name)}
                  {@const when = formatDateTime(b.created)}
                  <tr>
                    <td>{when}</td>
                    <td><Chip tone={b.kind === 'manual' ? 'accent' : b.kind === 'auto' ? 'info' : 'neutral'}>{kindLabel[b.kind]}</Chip></td>
                    <td class="num">{formatBytes(b.size)}</td>
                    <td class="num">
                      <button type="button" class="btn sm" disabled={backupBusy !== '' || backups.server_running} title={backups.server_running ? 'Stop the server first' : 'Replace the database by this backup'} onclick={() => restoreBackup(b.name, when)}>
                        <Icon name="undo" size={13} /> Restore
                      </button>
                      <button type="button" class="btn sm danger" disabled={backupBusy !== ''} aria-label="Delete the backup from {when}" onclick={() => deleteBackup(b.name, when)}><Icon name="trash" size={13} /></button>
                    </td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
          {#if backups.server_running}<p class="hint">Restoring needs the server to be stopped.</p>{/if}
        {/if}
      </section>
    {/if}
  </div>
{:else}
  <div class="loading"><Spinner size={20} /> Loading...</div>
{/if}

<style>
  .stack { display: flex; flex-direction: column; gap: var(--sp-4); }
  .stack > .card { margin: 0; }
  .runbar { display: flex; align-items: center; gap: var(--sp-3); flex-wrap: wrap; }
  .runbar + .notice, .runbar ~ .err { margin-top: var(--sp-4); }
  .tiles { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: var(--sp-3); }
  .tile { border: 1px solid var(--line-soft); background: var(--bg-2); border-radius: var(--radius); padding: var(--sp-3) var(--sp-4); min-width: 0; }
  .tile small { display: block; color: var(--faint); font-size: 10.5px; letter-spacing: 0.1em; text-transform: uppercase; }
  .tile b { display: block; font-size: 20px; font-weight: 600; margin-top: 2px; font-variant-numeric: tabular-nums; }
  .tile i { display: block; font-style: normal; color: var(--muted); font-size: var(--fs-xs); margin-top: 2px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .log :global(.error) { color: var(--danger); }
  .log :global(.warn) { color: var(--warn); }
  .log :global(.launcher) { color: var(--link); }
  .log { height: 260px; }
  .settings { display: flex; flex-direction: column; gap: var(--sp-3); padding-bottom: var(--sp-2); }
  .toggle { margin-top: var(--sp-4); display: flex; flex-direction: column; gap: var(--sp-3); }
  .toggle :global(.switch-text b) { display: block; font-weight: 500; }
  .toggle :global(.switch-text small) { display: block; color: var(--faint); font-size: var(--fs-sm); margin-top: 2px; }
  .ports { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: var(--sp-3); }
  .forward { margin-top: var(--sp-4); }
  .danger-row { align-items: center; gap: var(--sp-4); margin-top: var(--sp-3); }
  .danger-row p { margin: 0; color: var(--muted); font-size: var(--fs-sm); }
  .applybar { position: sticky; bottom: calc(-1 * var(--sp-5)); display: flex; align-items: center; gap: var(--sp-3); padding: var(--sp-3) var(--sp-4); margin-top: var(--sp-2); background: var(--surface-2); border: 1px solid var(--line); border-radius: var(--radius-lg); box-shadow: var(--shadow-sm); }
  .pkg-actions { margin-top: var(--sp-4); }
  .pkg-note { margin-top: var(--sp-4); }
  .notes { max-height: 220px; overflow: auto; margin: 6px 0; }
  .card .notice { margin-top: var(--sp-4); }
  .inline { display: inline-flex; align-items: center; gap: 8px; font-size: 13px; white-space: nowrap; }
  .keep { width: 70px; }
  .table-wrap { margin-top: var(--sp-4); }
</style>
