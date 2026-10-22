<script lang="ts">
  import { onDestroy, onMount, tick } from 'svelte';
  import { open } from '@tauri-apps/plugin-dialog';
  import { app } from '../lib/app.svelte';
  import { ipc } from '../lib/ipc';
  import { run, toasts } from '../lib/notify.svelte';
  import { listenHost, type HostProgress } from '../lib/events';
  import { formatBytes } from '../lib/format';
  import {
    formFromSettings, isUp, logLevel, playerAddress, portsToForward, pushLog, runLabel, settingsFromForm,
    validateHostForm, type HostForm,
  } from '../lib/host';
  import PageHead from '../lib/ui/PageHead.svelte';
  import Icon from '../lib/ui/Icon.svelte';
  import Chip from '../lib/ui/Chip.svelte';
  import Empty from '../lib/ui/Empty.svelte';
  import Spinner from '../lib/ui/Spinner.svelte';
  import Progress from '../lib/ui/Progress.svelte';
  import type { HostStatus, InstallOutcome, RunState } from '../lib/types';

  let status = $state<HostStatus | null>(null);
  let form = $state<HostForm | null>(null);
  let log = $state<string[]>([]);
  let progress = $state<HostProgress | null>(null);
  let step = $state('');
  let task = $state<'' | 'download' | 'local' | 'setup' | 'start' | 'stop'>('');
  let pkgError = $state('');
  let outcome = $state<InstallOutcome | null>(null);
  let ipNote = $state('');
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
  const pct = $derived(progress && progress.total > 0 ? Math.min(100, Math.round((progress.done / progress.total) * 100)) : null);

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
      onProgress: (p) => (progress = p),
      onStep: (s) => (step = s),
      onInstalled: () => void refresh(),
    });
    log = (await run(() => ipc.hostLogTail(500))) ?? [];
    await refresh();
    // The "ready" light and a crashed server need a look now and then.
    timer = setInterval(() => void refresh(), 2500);
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

  // --- the package ---------------------------------------------------------------------------

  async function download() {
    task = 'download';
    pkgError = '';
    progress = null;
    outcome = null;
    try {
      outcome = await ipc.hostInstallLatest();
      toasts.show('ok', `Installed ${outcome.package.name}`);
    } catch (e) {
      pkgError = e instanceof Error ? e.message : String(e);
    }
    task = '';
    progress = null;
    await refresh();
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

<PageHead icon="host" title="Host a server" sub="Run a private PoE2 server on this computer and let friends join it." />

{#if status && !status.platform.match(/^(linux|windows)$/)}
  <Empty icon="host" title="Not available here">Hosting from the launcher works on Linux and Windows only.</Empty>
{:else if status && form}
  <div class="stack">
    <!-- 1. The server program -->
    <section class="card">
      <div class="card-head"><span class="head-icon"><Icon name="box" size={16} /></span><h2>1. Server program</h2>{#if status.package}<span class="grow"></span><Chip tone="ok" icon="check">installed</Chip>{/if}</div>
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
      <div class="row" style="margin-top: 8px">
        <button type="button" class="btn primary" disabled={task !== '' || up || status.busy} onclick={download}>
          {status.package ? 'Update to the latest release' : 'Download the latest release'}
        </button>
        <button type="button" class="btn" disabled={task !== '' || up || status.busy} onclick={() => pickLocal(false)}>Use a package file...</button>
        <button type="button" class="btn" disabled={task !== '' || up || status.busy} onclick={() => pickLocal(true)}>Use a package folder...</button>
      </div>
      {#if task === 'download' || task === 'local'}
        <p class="hint"><Spinner size={12} /> {step || 'Working...'}</p>
        {#if pct !== null && progress?.phase === 'download'}
          <Progress value={pct} />
          <p class="hint">{formatBytes(progress.done)} of {formatBytes(progress.total)}</p>
        {/if}
      {/if}
      {#if pkgError}<p class="err" role="alert" style="white-space: pre-wrap">{pkgError}</p>{/if}
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
          <p>The server reads its game data from <code>{status.install_dir}</code>: <a href="#/install">install it first</a>.</p>
        </div>
      {/if}
    </section>

    <!-- 2. Setup -->
    <form class="card" onsubmit={setup}>
      <div class="card-head"><span class="head-icon"><Icon name="settings" size={16} /></span><h2>2. {configured ? 'Settings' : 'Set up'}</h2></div>
      <div class="two">
        <div>
          <label class="field"><span>Server name (shown to players)</span><input bind:value={form.server_name} maxlength="64" /></label>
          <label class="field">
            <span>Cluster id (login tokens belong to it; keep it when you re-install)</span>
            <input bind:value={form.cluster_id} maxlength="32" />
          </label>
          <div class="row">
            <label class="field grow"><span>Login port (TCP)</span><input type="number" min="1024" max="65535" bind:value={form.login_port} /></label>
            <label class="field grow"><span>Game port (TCP + UDP)</span><input type="number" min="1024" max="65535" bind:value={form.instance_port} /></label>
            <label class="field grow"><span>Web port (TCP)</span><input type="number" min="1024" max="65535" bind:value={form.web_port} /></label>
          </div>
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
          <label class="check"><input type="checkbox" bind:checked={form.web_public} /> Let other computers log in through this server's web API</label>
          <p class="hint">
            Players need the web API to log in with the launcher. It uses plain http, so passwords travel unencrypted: fine on a trusted LAN,
            otherwise put a TLS proxy in front (see the hosting guide). Off: only this computer can reach it.
          </p>
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

    <!-- 3. Run -->
    <section class="card">
      <div class="card-head"><span class="head-icon"><Icon name="play" size={16} /></span><h2>3. Run</h2></div>
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
        The server runs while the launcher is open; closing the launcher stops it.
        {#if configured}It is listed on the Servers page as <b>{hostProfile?.name ?? status.settings.server_name}</b> (<code>{status.web_url}</code>).{/if}
        {#if configured && !hostProfile?.logged_in}Log in on the Account page to play on it.{/if}
      </p>
      {#if configured && ready}
        <p class="hint">Game address players' clients connect to: <code>{status.info?.login_host}:{status.info?.login_port}</code></p>
      {/if}
    </section>

    <!-- 4. Log -->
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
  </div>
{:else}
  <div class="loading"><Spinner size={20} /> Loading...</div>
{/if}

<style>
  .log :global(.error) { color: var(--danger); }
  .log :global(.warn) { color: var(--warn); }
  .log :global(.launcher) { color: var(--link); }
</style>
