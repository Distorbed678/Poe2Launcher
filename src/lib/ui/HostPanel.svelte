<script lang="ts">
  import { onDestroy, onMount, tick } from 'svelte';
  import { open } from '@tauri-apps/plugin-dialog';
  import { app } from '../app.svelte';
  import { ipc } from '../ipc';
  import { run, toasts } from '../notify.svelte';
  import { listenHost, type HostProgress } from '../events';
  import { formatBytes, formatDateTime } from '../format';
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
  import Empty from './Empty.svelte';
  import Spinner from './Spinner.svelte';
  import Progress from './Progress.svelte';
  import Markdown from './Markdown.svelte';
  import type { BackupStatus, HostStatus, InstallOutcome, RunState, UpdateOutcome } from '../types';

  let status = $state<HostStatus | null>(null);
  let form = $state<HostForm | null>(null);
  let log = $state<string[]>([]);
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
  const errors = $derived(form ? validateHostForm(form, !configured) : []);
  const rules = $derived(form ? portsToForward(form) : []);
  const hostProfile = $derived(app.profiles.find((p) => p.id === status?.settings.profile_id));
  const noGame = $derived(status !== null && !status.game_installed);
  const noPackage = $derived(status !== null && status.package === null);
  const canSetup = $derived(!!form && !up && task === '' && !status?.busy && errors.length === 0 && !noGame && !noPackage);
  const upd = $derived(app.serverUpdate);
  const headline = $derived(updateHeadline(upd));
  const pct = $derived(progress && progress.total > 0 ? Math.min(100, Math.round((progress.done / progress.total) * 100)) : null);

  // The panel shows one part at a time. Before the first setup the parts are steps (program, settings,
  // start) and the panel follows the next one; afterwards they are tabs.
  type View = 'program' | 'settings' | 'run' | 'log' | 'backups';
  let view = $state<View | ''>('');
  const next = $derived<View>(noPackage ? 'program' : !configured ? 'settings' : 'run');
  const shown = $derived<View>(view || next);
  const steps = $derived([
    { id: 'program' as const, label: 'Program', done: !noPackage },
    { id: 'settings' as const, label: 'Settings', done: configured },
    { id: 'run' as const, label: 'Start', done: up },
  ]);
  const tabs = [
    { id: 'run', label: 'Overview', icon: 'play' as const },
    { id: 'program', label: 'Program', icon: 'box' as const },
    { id: 'settings', label: 'Settings', icon: 'settings' as const },
    { id: 'log', label: 'Log', icon: 'file' as const },
    { id: 'backups', label: 'Backups', icon: 'database' as const },
  ];

  async function refresh() {
    try {
      const s = await ipc.hostStatus();
      status = s;
      if (!form) form = formFromSettings(s.settings, s.lan_ip);
    } catch (e) {
      toasts.show('error', e instanceof Error ? e.message : String(e));
    }
  }

  onMount(async () => {
    unlisten = await listenHost({
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
    log = (await run(() => ipc.hostLogTail(500))) ?? [];
    await refresh();
    void loadBackups();
    // A newer server release: quiet, and not more than every 10 minutes.
    void app.checkServerUpdate();
    // The "ready" light and a crashed server need a look now and then.
    timer = setInterval(() => {
      void refresh();
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
      const b = await ipc.backupStatus();
      backups = b;
      if (backupBusy !== 'settings') keepText = String(b.settings.keep);
    } catch (e) {
      toasts.show('error', e instanceof Error ? e.message : String(e));
    }
  }

  async function backupNow() {
    backupBusy = 'create';
    const b = await run(() => ipc.backupCreate(), (r) => `Backup saved (${formatBytes(r.size)})`);
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
    const pre = await run(() => ipc.backupRestore(name), (r) => (r ? 'Restored. The previous database was kept as a backup.' : 'Restored.'));
    backupBusy = '';
    if (pre !== undefined) await loadBackups();
  }

  async function deleteBackup(name: string, when: string) {
    const yes = await ask({ title: 'Delete this backup?', message: `The backup from ${when} is deleted for good.`, confirmLabel: 'Delete', danger: true });
    if (!yes) return;
    backupBusy = 'delete';
    const r = await run(() => ipc.backupDelete(name), 'Backup deleted');
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
    const s = await run(() => ipc.backupSetSettings(auto, keep));
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
      outcome = await ipc.hostInstallLatest();
      toasts.show('ok', `Installed ${outcome.package.name}`);
    } catch (e) {
      pkgError = e instanceof Error ? e.message : String(e);
    }
    task = '';
    progress = null;
    dlRate.reset();
    await refresh();
    if (!configured) view = '';
    app.forgetServerUpdate();
    void app.checkServerUpdate(true);
  }

  async function checkUpdates() {
    task = 'check';
    pkgError = '';
    updated = null;
    await app.checkServerUpdate(true);
    task = '';
    if (app.serverUpdate && !app.serverUpdate.error && !app.serverUpdate.update_available) {
      toasts.show('ok', 'The server is the newest release for your game version');
    }
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
      updated = await ipc.hostUpdateApply();
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
    app.forgetServerUpdate();
    void app.checkServerUpdate(true);
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
      outcome = await ipc.hostInstallLocal(p);
      toasts.show('ok', `Installed ${outcome.package.name}`);
    } catch (e) {
      pkgError = e instanceof Error ? e.message : String(e);
    }
    task = '';
    await refresh();
    if (!configured) view = '';
    app.forgetServerUpdate();
    void app.checkServerUpdate(true);
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

  // --- setup, start, stop ----------------------------------------------------------------------

  async function waitReady(): Promise<boolean> {
    for (let i = 0; i < 600; i++) {
      await refresh();
      if (status?.info) return true;
      if (!status || !isUp(status.run)) return false;
      await new Promise((r) => setTimeout(r, 500));
    }
    return false;
  }

  async function setup(e: SubmitEvent) {
    e.preventDefault();
    if (!form || !status || !canSetup) return;
    task = 'setup';
    step = 'starting';
    let pw = form.admin_password;
    const admin = form.admin_name.trim();
    const res = await run(() =>
      ipc.hostSetup({ settings: settingsFromForm(status!.settings, form!), admin_password: pw }),
    );
    form.admin_password = '';
    form.admin_password2 = '';
    if (res) {
      toasts.show('ok', res.admin_existed ? `${admin} already existed and is now an admin (its password is unchanged)` : 'Server set up');
      await Promise.all([app.refreshProfiles(), app.refreshSettings()]);
      if (pw) {
        // First setup: start the server and log in with the account just made.
        step = 'starting the server';
        if ((await run(() => ipc.hostStart())) !== undefined && (await waitReady())) {
          if (!res.admin_existed) {
            step = 'logging in';
            await run(() => ipc.login(res.profile.id, admin, pw), `Logged in as ${admin}`);
            await app.refreshProfiles();
          }
        }
      }
    }
    pw = '';
    step = '';
    task = '';
    await refresh();
    if (configured) view = 'run';
  }

  async function start() {
    task = 'start';
    await run(() => ipc.hostStart());
    task = '';
    await refresh();
  }

  async function stop() {
    task = 'stop';
    await run(() => ipc.hostStop());
    task = '';
    await refresh();
  }

  async function autostart(on: boolean) {
    const s = await run(() => ipc.hostSetAutostart(on));
    if (s && status) status = { ...status, settings: s };
  }
</script>


{#if status && !status.platform.match(/^(linux|windows)$/)}
  <Empty icon="host" title="Not available here">Hosting from the launcher works on Linux and Windows only.</Empty>
{:else if status && form}
  {#if configured}
    <Tabs {tabs} value={shown} label="Your server" onchange={(id) => (view = id as View)} />
  {:else}
    <p class="muted intro">Run a private PoE2 server on this computer and let friends join it.</p>
    <ol class="stepper" aria-label="Create a server">
      {#each steps as st, i (st.id)}
        <li class:done={st.done} class:on={shown === st.id}>
          <button type="button" onclick={() => (view = st.id)} aria-current={shown === st.id ? 'step' : undefined}>
            <span class="n">{#if st.done}<Icon name="check" size={11} />{:else}{i + 1}{/if}</span> {st.label}
          </button>
        </li>
      {/each}
    </ol>
  {/if}
  <div class="stack">
    {#if shown === 'program'}
    <!-- The server program -->
    <section class="card">
      <div class="card-head"><span class="head-icon"><Icon name="box" size={16} /></span><h2>Server program</h2>{#if status.package}<span class="grow"></span><Chip tone="ok" icon="check">installed</Chip>{/if}</div>
      {#if status.package}
        <dl class="kv">
          <dt>Installed</dt><dd>{status.package.name}</dd>
          <dt>Server version</dt><dd>{status.package.server_version} for game {status.package.game_version}</dd>
          <dt>Folder</dt><dd><code>{status.package.dir}</code></dd>
        </dl>
      {:else}
        <p class="muted">No server program yet. Download the release that matches your game version ({status.game_version}), or use a package you already have.</p>
        {#if status.package_problem}<p class="err">{status.package_problem}</p>{/if}
      {/if}
      {#if headline && upd?.release && status.package}
        <div class="notice accent" style="margin-top: 12px">
          <span class="notice-title"><Icon name="download" size={16} /> {headline}</span>
          <p class="hint">
            You have {status.package.server_version}.
            {#if upd.release.repo !== 'Distorbed678/Poe2Server'}It comes from the project's older release page ({upd.release.repo}).{/if}
          </p>
          {#if upd.release.notes}
            <div class="notes"><Markdown source={upd.release.notes} compact /></div>
          {/if}
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
      <div class="row" style="margin-top: 8px">
        {#if status.package}
          <button type="button" class="btn" disabled={task !== '' || status.busy} onclick={checkUpdates}>
            {#if task === 'check'}<Spinner size={14} />{:else}<Icon name="refresh" size={14} />{/if} Check for updates
          </button>
        {:else}
          <button type="button" class="btn primary" disabled={task !== '' || up || status.busy} onclick={download}>Download the latest release</button>
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
            : outcome.source === 'folder'
            ? 'Copied from a folder (no checksum to compare).'
            : outcome.checksum_verified
              ? `SHA-256 checked: ${outcome.sha256}`
              : `Not checked: there is no .sha256 file next to the package. Its SHA-256 is ${outcome.sha256}.`}
        </p>
      {/if}
      <p class="hint">
        A package is a file named <code>poe-server-v…-game-{status.game_version}-{status.platform}-x86_64</code>, from the project's server releases.
        Downloads are checked against the release's SHA-256 file; a mismatch installs nothing. The package holds no game files.
      </p>
      {#if noGame}
        <div class="notice" style="margin-top: 12px">
          <span class="notice-title"><Icon name="alert" size={16} /> The game is not installed</span>
          <p>The server reads its game data from <code>{status.install_dir}</code>: <a href="#/library">install it first</a>.</p>
        </div>
      {/if}
      {#if !noPackage && !configured}
        <div class="actions"><button type="button" class="btn primary" onclick={() => (view = 'settings')}>Next: settings</button></div>
      {/if}
    </section>
    {/if}

    {#if shown === 'settings'}
    <!-- Setup -->
    <form class="card" onsubmit={setup}>
      <div class="card-head"><span class="head-icon"><Icon name="settings" size={16} /></span><h2>{configured ? 'Settings' : 'Set up'}</h2></div>
      <div class="two">
        <div>
          <label class="field"><span>Server name (shown to players)</span><input bind:value={form.server_name} maxlength="64" /></label>
          <label class="field">
            <span>Public host: the address players use to reach this computer</span>
            <input bind:value={form.public_host} placeholder="192.168.1.20 or play.example.com" />
          </label>
          <div class="row" style="margin: -4px 0 8px">
            <button type="button" class="btn" disabled={!status.lan_ip} onclick={useLan}>
              Use my LAN address{status.lan_ip ? ` (${status.lan_ip})` : ''}
            </button>
            <button type="button" class="btn" onclick={lookupPublicIp} title="Contacts api.ipify.org once, only when you click">Look up my public IP</button>
          </div>
          {#if ipNote}<p class="hint">{ipNote}</p>{/if}
          <p class="hint">
            A LAN party needs the LAN address. Over the internet use your public IP or a host name, and forward the ports listed on the right in your router.
            The public IP lookup asks a web service (api.ipify.org) and only happens when you press the button.
          </p>
        </div>
        <div>
          <label class="check"><input type="checkbox" bind:checked={form.open_registration} /> Anyone who knows the address can create an account</label>
          <p class="hint">Off: only admins create accounts (the Web admin or the server commands).</p>
          {#if configured}
            <p class="muted">Admin account: <b>{status.settings.admin_name || '(unknown)'}</b>. Leave the password empty to keep your accounts as they are.</p>
          {:else}
            <h2 style="margin-top: 14px">Admin account</h2>
          {/if}
          <label class="field"><span>Admin account name</span><input bind:value={form.admin_name} autocomplete="off" /></label>
          <label class="field">
            <span>{configured ? 'New admin account password (optional)' : 'Password (at least 8 characters)'}</span>
            <input type="password" bind:value={form.admin_password} autocomplete="new-password" />
          </label>
          <label class="field">
            <span>Repeat the password</span>
            <input type="password" bind:value={form.admin_password2} autocomplete="new-password" aria-invalid={form.admin_password2 !== '' && form.admin_password !== form.admin_password2} />
          </label>
          <p class="hint">The password is used once to create the account on your server and is not stored or written to any file.</p>
        </div>
      </div>
      <details class="advanced">
        <summary>Advanced: ports, cluster id, remote login</summary>
        <div class="two">
        <div>
          <label class="field">
            <span>Cluster id (login tokens belong to it; keep it when you re-install)</span>
            <input bind:value={form.cluster_id} maxlength="32" />
          </label>
          <div class="row">
            <label class="field grow"><span>Login port (TCP)</span><input type="number" min="1024" max="65535" bind:value={form.login_port} /></label>
            <label class="field grow"><span>Game port (TCP + UDP)</span><input type="number" min="1024" max="65535" bind:value={form.instance_port} /></label>
            <label class="field grow"><span>Web port (TCP)</span><input type="number" min="1024" max="65535" bind:value={form.web_port} /></label>
          </div>
          <label class="check"><input type="checkbox" bind:checked={form.web_public} /> Let other computers log in through this server's web API</label>
          <p class="hint">
            Players need the web API to log in with the launcher. It uses plain http, so passwords travel unencrypted: fine on a trusted LAN,
            otherwise put a TLS proxy in front (see the hosting guide). Off: only this computer can reach it.
          </p>
        </div>
        <div>
          <div class="notice info" style="margin-top: 8px">
            <span class="notice-title"><Icon name="globe" size={16} /> Port forwarding</span> <span class="hint">only for players outside your network</span>
            <ul style="margin: 4px 0 0; padding-left: 18px">
              {#each rules as r (r.port)}<li><code>{r.port}</code> {r.proto}: {r.what}</li>{/each}
            </ul>
            <p class="hint" style="margin-top: 4px">
              Players add the server as <code>{playerAddress(form)}</code> (with the Public host above).
              <button type="button" class="link" onclick={() => ipc.hostOpenGuide()}>Open the hosting guide</button>
            </p>
          </div>
        </div>
        </div>
      </details>
      {#if errors.length > 0}
        <ul class="errs">{#each errors as m (m)}<li>{m}</li>{/each}</ul>
      {/if}
      <div class="actions">
        {#if task === 'setup'}<span class="muted">{step || 'working...'}</span>{/if}
        <button class="btn primary" type="submit" disabled={!canSetup}>
          {configured ? 'Apply settings' : 'Set up and start'}
        </button>
      </div>
      {#if up}<p class="hint">Stop the server to change its settings.</p>{/if}
    </form>
    {/if}

    {#if shown === 'run'}
    <!-- Run -->
    <section class="card">
      <div class="card-head"><span class="head-icon"><Icon name="play" size={16} /></span><h2>{configured ? 'Run' : 'Start'}</h2></div>
      <div class="row">
        <Chip tone={ready ? 'ok' : up ? 'warn' : run_.state === 'exited' && run_.code !== 0 && run_.code !== null ? 'bad' : 'neutral'} dot>
          {runLabel(run_, ready)}
        </Chip>
        {#if status.info}<span class="muted">{status.info.online} online</span>{/if}
        <span class="grow"></span>
        {#if hostProfile?.role === 'admin' && ready}
          <button type="button" class="btn" onclick={() => app.openWebAdmin(hostProfile!.id)} title="Opens the web admin in your browser, already signed in">Open web admin</button>
        {/if}
        <button type="button" class="btn primary" disabled={!configured || up || task !== '' || noGame} onclick={start}>Start</button>
        <button type="button" class="btn danger" disabled={!up || task === 'stop'} onclick={stop}>Stop</button>
      </div>
      <label class="check" style="margin-top: 8px">
        <input type="checkbox" checked={status.settings.autostart} disabled={!configured} onchange={(e) => autostart(e.currentTarget.checked)} />
        Start this server when the launcher starts
      </label>
      <p class="hint">
        The server runs while the launcher is open; closing the launcher stops it. When an admin restarts it from the web admin, the launcher starts it again by itself.
        {#if configured}It is listed under Servers as <b>{hostProfile?.name ?? status.settings.server_name}</b> (<code>{status.web_url}</code>).{/if}
        {#if configured && !hostProfile?.logged_in}Log in on the Servers page to play on it.{/if}
      </p>
      {#if configured && ready}
        <p class="hint">Game address players' clients connect to: <code>{status.info?.login_host}:{status.info?.login_port}</code></p>
      {/if}
      {#if !configured}<p class="hint">Set the server up first: <button type="button" class="link" onclick={() => (view = 'settings')}>Settings</button>.</p>{/if}
    </section>
    {/if}

    {#if shown === 'log' || (shown === 'run' && configured)}
    <!-- Log -->
    <section class="card">
      <div class="card-head"><span class="head-icon"><Icon name="info" size={16} /></span><h2>Server log</h2></div>
      <div class="log" bind:this={logEl} role="log" aria-label="Server log">
        {#if log.length === 0}<span class="muted">Nothing yet. The log appears when the server runs.</span>{/if}
        {#each log as line, i (i)}
          <div class={logLevel(line)}>{line}</div>
        {/each}
      </div>
      <p class="hint">Also written to <code>{status.log_file}</code> (replaced at each start).</p>
    </section>
    {/if}

    {#if shown === 'backups'}
    <!-- Backups -->
    <section class="card">
      <div class="card-head"><span class="head-icon"><Icon name="database" size={16} /></span><h2>Backups</h2>
        <span class="grow"></span>
        <button type="button" class="btn primary" disabled={backupBusy !== '' || !backups?.database_size} onclick={backupNow}
          title={backups?.database_size ? 'Copies the server database now; works while the server runs' : 'The server has no database yet: start it once'}>
          {#if backupBusy === 'create'}<Spinner size={14} />{:else}<Icon name="database" size={14} />{/if} Back up now
        </button>
      </div>
      <p class="hint">
        A backup is a copy of the server's database (accounts and characters). It is made with SQLite's backup function, so it is consistent
        while the server runs. {#if backups?.database_size}The database is {formatBytes(backups.database_size)}.{:else}There is no database yet.{/if}
        Backups live in <code>{backups?.dir ?? '...'}</code>.
      </p>
      <div class="row" style="margin: 10px 0">
        <label class="check">
          <input type="checkbox" checked={backups?.settings.auto ?? false} disabled={backupBusy !== ''} onchange={(e) => saveBackupSettings(e.currentTarget.checked)} />
          <span>Back up automatically each time the server starts from the launcher</span>
        </label>
        <span class="grow"></span>
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
  .log :global(.error) { color: var(--danger); }
  .log :global(.warn) { color: var(--warn); }
  .log :global(.launcher) { color: var(--link); }
  .inline { display: inline-flex; align-items: center; gap: 8px; font-size: 13px; white-space: nowrap; }
  .keep { width: 70px; }
  .notes { max-height: 220px; overflow: auto; margin: 6px 0; }
  .intro { margin: 0 0 var(--sp-3); }
  .stepper { list-style: none; margin: 0 0 var(--sp-4); padding: 0; display: flex; gap: var(--sp-2); }
  .stepper li { flex: 1; border-top: 2px solid var(--line); transition: border-color var(--dur) var(--ease); }
  .stepper li.done { border-color: var(--ok); }
  .stepper li.on { border-color: var(--gold); }
  .stepper button { display: flex; align-items: center; gap: 8px; width: 100%; padding: 8px 0 0; background: none; border: 0; cursor: pointer; color: var(--faint); font-size: var(--fs-sm); text-align: left; }
  .stepper li.done button { color: var(--ok); }
  .stepper li.on button { color: var(--gold-2); }
  .stepper .n { width: 18px; height: 18px; border-radius: 50%; border: 1px solid currentColor; display: grid; place-items: center; font-size: 10.5px; }
  .advanced { margin-top: var(--sp-2); border-top: 1px solid var(--line-soft); padding-top: var(--sp-3); }
  .advanced summary { cursor: pointer; color: var(--gold); font-size: var(--fs-sm); margin-bottom: var(--sp-3); }
  .log { height: 220px; }
</style>
