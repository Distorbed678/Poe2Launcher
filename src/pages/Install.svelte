<script lang="ts">
  import { app } from '../lib/app.svelte';
  import { ipc } from '../lib/ipc';
  import { run } from '../lib/notify.svelte';
  import { onMount } from 'svelte';
  import { formatBytes } from '../lib/format';
  import { formatEta, formatSpeed, installBytesDone, installBytesTotal } from '../lib/transfer';
  import { isActive, promptText, validGuardCode } from '../lib/install';
  import Icon from '../lib/ui/Icon.svelte';
  import Chip from '../lib/ui/Chip.svelte';
  import Spinner from '../lib/ui/Spinner.svelte';
  import Progress from '../lib/ui/Progress.svelte';
  import HookPanel from '../lib/ui/HookPanel.svelte';
  import Fold from '../lib/ui/Fold.svelte';
  import { versionLabel } from '../lib/gameversion';

  let username = $state(app.settings?.steam_username ?? '');
  let password = $state('');
  let remember = $state(app.settings?.remember_steam_login ?? false);
  let code = $state('');
  let logEl = $state<HTMLElement | null>(null);

  const st = $derived(app.status);
  const ins = $derived(app.install);
  const active = $derived(isActive(ins.phase) || (st?.running ?? false));
  const paused = $derived(ins.phase === 'paused' || (st?.paused != null && !active));
  const space = $derived(app.space);
  const rate = app.installRate;
  const total = $derived(st ? installBytesTotal(st.depots) : 0);
  const doneBytes = $derived(st ? installBytesDone(st.depots, ins.depots) : 0);
  const overall = $derived(total > 0 ? Math.min(100, (doneBytes / total) * 100) : null);
  const noRoom = $derived(space !== null && !space.ok);

  onMount(() => void app.refreshSpace());
  // The folder or the version changed, or a download ended: look at the disk again.
  $effect(() => {
    void st?.install_dir;
    void ins.phase;
    void app.refreshSpace();
  });

  $effect(() => {
    void ins.log.length;
    if (logEl) logEl.scrollTop = logEl.scrollHeight;
  });

  async function start(validate: boolean) {
    if (app.settings) {
      const s = await run(() => ipc.setSettings({ ...app.settings!, steam_username: username.trim(), remember_steam_login: remember }));
      if (s) app.settings = s;
    }
    const ok = await run(() =>
      ipc.installStart({ steam_username: username, steam_password: password, remember_login: remember, validate }),
    );
    // The password has been handed over; do not keep it in the form.
    if (ok !== undefined) password = '';
  }

  async function sendCode(e: SubmitEvent) {
    e.preventDefault();
    const c = code.trim();
    if ((await run(() => ipc.installInput(c))) !== undefined) {
      app.applyInstall({ type: 'input_sent' });
      code = '';
    }
  }

  async function pause() {
    await run(() => ipc.installPause());
  }

  async function resume() {
    const ok = await run(() => ipc.installResume(password));
    if (ok !== undefined) password = '';
  }

  async function verify() {
    await run(() => ipc.installVerify());
  }

  const toolPct = $derived(ins.tool && ins.tool.total > 0 ? (ins.tool.done / ins.tool.total) * 100 : 0);
  const label = $derived(versionLabel(app.version));
  // The Steam form is only in the way once the game is installed: it opens by itself when it
  // is needed (nothing installed, a download running or paused).
  let steamOpen = $state(false);
  $effect(() => {
    if (!st?.installed || active || paused) steamOpen = true;
  });
  let detailsOpen = $state(false);
</script>

{#if app.official}
  <div class="notice accent official">
    <span class="notice-title"><Icon name="eye" size={16} /> Official (read-only): always the current official build</span>
    <ul>
      <li>This install is separate from the private-server versions and is downloaded <b>without pinned manifests</b>: Steam gives the newest build, which the official servers need. <b>Download again</b> updates it, <b>Repair</b> re-checks the files.</li>
      <li>Play starts the game with its own Steam login and the original realm list. The hook only observes; the launcher sends nothing and never logs in for you.</li>
      <li>The hook is built for one client build. If the game was patched and its signatures no longer match, the hook switches its features off by itself: expected, nothing breaks, you just get no capture.</li>
    </ul>
  </div>
{/if}

<div class="col">
  <section class="card status" class:ok={st?.installed}>
    <div class="status-row">
      <span class="status-icon" aria-hidden="true">
        {#if active}<Spinner size={22} />{:else if st?.installed}<Icon name="check" size={24} />{:else}<Icon name="install" size={24} />{/if}
      </span>
      <div class="grow status-text">
        <b class="big">
          {#if active}{ins.phase === 'verifying' ? 'Verifying the install' : ins.phase === 'fetching_tool' ? 'Getting the downloader' : 'Downloading from Steam'}
          {:else if paused}Download paused
          {:else if st?.installed}Game installed{:else}Game not installed{/if}
        </b>
        <span class="muted">{st ? `${app.official ? 'Official build' : `Client ${st.version}`} · ` : ''}<code>{st?.install_dir ?? ''}</code></span>
      </div>
      <button type="button" class="btn" onclick={verify} disabled={!st?.installed || active}><Icon name="shield" size={14} /> Verify files</button>
    </div>

    {#if active || paused || (ins.phase !== 'idle' && ins.phase !== 'done')}
      <div class="progress-block">
        {#if paused && !active}<p class="muted"><Chip tone="warn" icon="pause">Paused</Chip> Resume carries on from what is on disk and checks it first.</p>
        {:else if ins.phase === 'cancelled'}<p><Chip tone="warn">Cancelled</Chip></p>
        {:else if ins.phase === 'failed'}<p><Chip tone="bad" icon="alert">Failed</Chip> <span class="err">{ins.message}</span></p>{/if}
        {#if ins.tool && ins.phase === 'fetching_tool'}
          <div class="progrow"><span>DepotDownloader</span><Progress value={toolPct} /><span class="num">{formatBytes(ins.tool.done)}</span></div>
        {/if}
        {#if overall !== null && (ins.phase === 'downloading' || paused) && Object.keys(ins.depots).length > 0}
          <div class="progrow total">
            <span><b>Total</b> <span class="muted">(estimate)</span></span>
            <Progress value={overall} active={ins.phase === 'downloading'} />
            <span class="num">{overall.toFixed(1)}%</span>
          </div>
          <p class="rate">
            <span><b>{formatSpeed(rate.bps)}</b></span>
            <span>{#if ins.phase === 'downloading'}{formatEta(rate.etaSecs)} left{:else}paused{/if}</span>
            <span class="muted">about {formatBytes(doneBytes)} of {formatBytes(total)}</span>
          </p>
        {/if}
        {#each st?.depots ?? [] as d (d.id)}
          {#if d.id in ins.depots}
            <div class="progrow"><span>Depot {d.id} ({d.role})</span><Progress value={ins.depots[d.id] ?? 0} active={active} /><span class="num">{(ins.depots[d.id] ?? 0).toFixed(1)}%</span></div>
          {/if}
        {/each}
        {#if ins.phase === 'downloading' && Object.keys(ins.depots).length === 0}<Progress value={null} />{/if}
        <div class="row">
          {#if ins.phase === 'downloading'}
            <button class="btn" type="button" onclick={pause} title="Stops DepotDownloader. What is on disk stays; Resume carries on from it."><Icon name="pause" size={14} /> Pause</button>
          {/if}
          {#if active}<button class="btn danger" type="button" onclick={() => run(() => ipc.installCancel())}>Cancel</button>{/if}
        </div>
      </div>
    {/if}

    {#if ins.prompt}
      <form class="notice accent" onsubmit={sendCode}>
        <span class="notice-title"><Icon name="key" size={16} /> Steam Guard</span>
        <p>{promptText(ins.prompt)}</p>
        <div class="row">
          <input style="max-width: 200px" bind:value={code} placeholder="Steam Guard code" autocomplete="one-time-code" maxlength="8" />
          <button class="btn primary" type="submit" disabled={!validGuardCode(code)}>Send</button>
        </div>
      </form>
    {/if}
  </section>

  {#if ins.verify}
    <section class="card">
      <div class="card-head">
        <span class="head-icon"><Icon name="shield" size={16} /></span><h2>Verification {ins.verify.ok ? '' : '(failed)'}</h2>
        <span class="grow"></span>
        <span class="muted">{ins.verify.files.toLocaleString()} files, {formatBytes(ins.verify.bytes)}</span>
      </div>
      <div class="table-wrap">
        <table>
          <tbody>
            {#each ins.verify.checks as c (c.name)}
              <tr>
                <td><Chip tone={c.status === 'pass' ? 'ok' : c.status === 'fail' ? 'bad' : 'warn'}>{c.status}</Chip></td>
                <td>{c.name}</td>
                <td class="muted">{c.detail}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    </section>
  {/if}

  <HookPanel compact />

  <Fold title="Download or repair from Steam" bind:open={steamOpen} summary={st?.installed ? 'only needed to repair or update' : 'needed to install the game'}>
    <p class="hint">
      The launcher downloads the game with DepotDownloader {st?.tool_version ?? ''} using <b>your own Steam login</b>, into its own folder.
      It never touches the Steam install of the game. Your password is passed to the downloader once and is not stored.
    </p>
    <form
      class="steam-form"
      onsubmit={(e) => {
        e.preventDefault();
        void start(false);
      }}
    >
      <label class="field"><span>Steam account name</span><input bind:value={username} autocomplete="username" disabled={active} /></label>
      <label class="field">
        <span>Steam password</span>
        <input type="password" bind:value={password} autocomplete="current-password" disabled={active} />
        {#if remember}<span class="hint">Not needed again once the Steam login is remembered; leave it empty then.</span>{/if}
      </label>
      <label class="check">
        <input type="checkbox" bind:checked={remember} disabled={active} />
        <span>Remember the Steam login <span class="hint">(DepotDownloader keeps a login token of its own, not your password; forget it in Settings)</span></span>
      </label>
      {#if space}
        <div class="space" class:bad={!space.ok}>
          <Icon name={space.ok ? 'database' : 'alert'} size={15} />
          <span>{space.message}</span>
        </div>
      {/if}
      <div class="actions">
        {#if paused}
          <button class="btn danger" type="button" onclick={() => run(() => ipc.installCancel())}>Forget the pause</button>
          <button class="btn primary" type="button" onclick={resume} disabled={!username.trim() || noRoom}>
            <Icon name="play" size={14} filled /> Resume download
          </button>
        {:else}
          <button type="button" class="btn" onclick={() => start(true)} disabled={active || !username.trim() || noRoom} title="Check the files that are already there and fetch what differs">Repair</button>
          <button class="btn primary" type="submit" disabled={active || !username.trim() || noRoom}>
            {#if active}<Spinner size={14} />{/if}
            {st?.installed ? 'Download again' : 'Install'} {label}
          </button>
        {/if}
      </div>
    </form>
  </Fold>

  <Fold title="Details" bind:open={detailsOpen} summary="depots and the downloader's output">
    {#if st}
      <h3 class="sub">Depots (install order)</h3>
      <ol class="depot-list">
        {#each st.depots as d (d.id)}
          <li><b>{d.id}</b> <span class="muted">{d.role}</span> <span class="grow"></span>{#if d.manifest}<span class="muted">manifest</span> <code>{d.manifest}</code>{:else}<Chip tone="accent">current build</Chip>{/if}</li>
        {/each}
      </ol>
    {/if}
    <h3 class="sub">Downloader output</h3>
    <div class="log" bind:this={logEl} aria-live="off">
      {#each ins.log as l, i (i)}<div class={l.stream}>{l.text}</div>{/each}
      {#if ins.log.length === 0}<span class="muted">The downloader's output appears here.</span>{/if}
    </div>
  </Fold>
</div>

<style>
  .col { max-width: 820px; display: flex; flex-direction: column; gap: var(--sp-4); }
  .col > :global(.card) { margin: 0; }
  .official { margin-bottom: var(--sp-4); max-width: 820px; }
  .status-row { display: flex; align-items: center; gap: var(--sp-4); }
  .status-icon { flex: none; width: 46px; height: 46px; border-radius: 50%; display: grid; place-items: center; background: var(--surface-3); color: var(--muted); }
  .status.ok .status-icon { background: color-mix(in srgb, var(--ok) 18%, transparent); color: var(--ok); }
  .status-text { display: flex; flex-direction: column; gap: 3px; min-width: 0; }
  .status-text .big { font: 600 var(--fs-lg) / 1.25 var(--font-head); letter-spacing: 0.05em; }
  .status-text .muted { font-size: var(--fs-sm); overflow-wrap: anywhere; }
  .progress-block { margin-top: var(--sp-4); padding-top: var(--sp-4); border-top: 1px solid var(--line-soft); display: flex; flex-direction: column; gap: var(--sp-2); }
  .progress-block p { margin: 0; }
  .status .notice { margin-top: var(--sp-4); }
  .steam-form { margin-top: var(--sp-3); max-width: 520px; }
  .space { display: flex; align-items: flex-start; gap: 8px; margin-top: 10px; padding: 8px 10px; border-radius: var(--radius-sm); background: var(--bg-2); border: 1px solid var(--line-soft); font-size: 12.5px; color: var(--muted); }
  .space.bad { color: var(--text); border-color: color-mix(in srgb, var(--danger) 45%, var(--line)); background: var(--danger-bg); }
  .rate { display: flex; flex-wrap: wrap; gap: 4px 18px; margin: 2px 0 4px; font-variant-numeric: tabular-nums; }
  .sub { font: 600 11px / 1 var(--font); letter-spacing: 0.12em; text-transform: uppercase; color: var(--faint); margin: var(--sp-4) 0 var(--sp-2); }
  .log { height: 200px; }
</style>
