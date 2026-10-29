<script lang="ts">
  import { onDestroy, onMount, tick } from 'svelte';
  import { slide } from 'svelte/transition';
  import { app } from '../lib/app.svelte';
  import { ipc } from '../lib/ipc';
  import { run } from '../lib/notify.svelte';
  import { ask } from '../lib/confirm.svelte';
  import { DUR, ease, ms } from '../lib/motion';
  import { needsPlainHttpConfirmation, normalizeServerUrl, plainHttpRisk, plainHttpWarning } from '../lib/url';
  import { formatUptime } from '../lib/format';
  import { serverCompat, versionLabel } from '../lib/gameversion';
  import { isUp, runLabel } from '../lib/host';
  import { router } from '../lib/router.svelte';
  import PageHead from '../lib/ui/PageHead.svelte';
  import PlainHttpNotice from '../lib/ui/PlainHttpNotice.svelte';
  import Icon from '../lib/ui/Icon.svelte';
  import Chip from '../lib/ui/Chip.svelte';
  import Empty from '../lib/ui/Empty.svelte';
  import Spinner from '../lib/ui/Spinner.svelte';
  import AccountPanel from '../lib/ui/AccountPanel.svelte';
  import Tabs from '../lib/ui/Tabs.svelte';
  import ServerBrowser from '../lib/ui/ServerBrowser.svelte';
  import Drawer from '../lib/ui/Drawer.svelte';
  import HostPanel from '../lib/ui/HostPanel.svelte';
  import type { HostStatus, ProfileView as Profile } from '../lib/types';

  // Servers: the ones you saved, the one you host on this computer (the former Host page is the
  // "Create a server" / settings panel), and the public list.
  const tab = $derived(router.route.query.get('tab') === 'browse' ? 'browse' : 'mine');
  const tabs = $derived([
    { id: 'mine', label: 'My servers', icon: 'servers' as const, badge: String(app.profiles.length) },
    { id: 'browse', label: 'Browse public', icon: 'compass' as const },
  ]);

  // --- your hosted server -------------------------------------------------------------------
  let host = $state<HostStatus | null>(null);
  let hostTask = $state<'' | 'start' | 'stop'>('');
  const hostOpen = $derived(router.route.segments[0] === 'host' || router.route.query.get('host') === '1');
  const canHost = $derived(host !== null && /^(linux|windows)$/.test(host.platform));
  const hosted = $derived(host?.settings.configured ? host : null);
  const hostProfile = $derived(hosted ? app.profiles.find((p) => p.id === hosted.settings.profile_id) : undefined);
  const hostUp = $derived(hosted ? isUp(hosted.run) : false);
  const others = $derived(app.profiles.filter((p) => p.id !== hostProfile?.id));
  let hostTimer: ReturnType<typeof setInterval> | undefined;

  async function refreshHost() {
    if (app.official) return;
    try {
      host = await ipc.hostStatus();
    } catch {
      /* hosting is optional: no row */
    }
  }

  function openHost() {
    router.go('servers', { host: '1' });
  }
  function closeHost() {
    router.go('servers');
    void refreshHost();
  }

  async function hostStart() {
    hostTask = 'start';
    await run(() => ipc.hostStart());
    hostTask = '';
    await refreshHost();
  }
  async function hostStop() {
    hostTask = 'stop';
    await run(() => ipc.hostStop());
    hostTask = '';
    await refreshHost();
  }

  onMount(() => {
    void app.checkServers();
    void refreshHost();
    hostTimer = setInterval(() => void refreshHost(), 4000);
  });
  onDestroy(() => hostTimer && clearInterval(hostTimer));

  // --- add or edit by address ---------------------------------------------------------------
  let dialog: HTMLDialogElement | undefined = $state();
  let name = $state('');
  let url = $state('');
  let editing = $state<string | null>(null);
  let busy = $state<string | null>(null);
  /** The user ticked "I understand" for the plain http address in the form. */
  let agreeHttp = $state(false);

  const normalized = $derived(normalizeServerUrl(url));
  const warning = $derived(normalized ? plainHttpWarning(plainHttpRisk(normalized)) : null);
  const current = $derived(editing ? app.profiles.find((p) => p.id === editing) : undefined);
  // An address that was confirmed before stays confirmed while it is not changed.
  const alreadyAgreed = $derived(current !== undefined && current.url === normalized && current.plain_http_confirmed);
  const needsAgreement = $derived(normalized !== null && needsPlainHttpConfirmation(normalized) && !alreadyAgreed);
  const valid = $derived(name.trim().length > 0 && normalized !== null && (!needsAgreement || agreeHttp));

  async function openForm(p?: Profile) {
    editing = p?.id ?? null;
    name = p?.name ?? '';
    url = p?.url ?? '';
    agreeHttp = false;
    await tick();
    dialog?.showModal();
  }

  function closeForm() {
    dialog?.close();
    editing = null;
  }

  async function save(e: SubmitEvent) {
    e.preventDefault();
    const r = await run(
      () => (editing ? ipc.profileUpdate(editing, name, url, agreeHttp) : ipc.profileAdd(name, url, agreeHttp)),
      editing ? 'Server updated' : 'Server added',
    );
    if (r) {
      closeForm();
      await Promise.all([app.refreshProfiles(), app.refreshSettings()]);
      void app.checkServer(r.id);
    }
  }

  // --- rows -----------------------------------------------------------------------------------
  let open = $state<Record<string, boolean>>({});
  // A server you still have to log in to opens by itself: the login is what Play waits for.
  const isOpen = (p: Profile) => open[p.id] ?? (app.selected?.id === p.id && !p.logged_in);

  async function check(id: string) {
    busy = id;
    await app.checkServer(id);
    // Also re-read the account's role: a promotion shows without logging in again.
    if (app.profiles.find((p) => p.id === id)?.logged_in) {
      try {
        await ipc.accountRefresh(id);
        await app.refreshProfiles();
      } catch {
        /* the Check result is what matters */
      }
    }
    busy = null;
  }

  async function remove(id: string, label: string) {
    const ok = await ask({
      title: `Remove "${label}"?`,
      message: 'Its login is revoked and forgotten. The server itself is not touched.',
      confirmLabel: 'Remove server',
      danger: true,
    });
    if (!ok) return;
    if ((await run(() => ipc.profileRemove(id), 'Server removed')) !== undefined) {
      await Promise.all([app.refreshProfiles(), app.refreshSettings()]);
    }
  }
</script>

<PageHead title="Servers" sub={`For ${versionLabel(app.version)}`}>
  <button type="button" class="btn" onclick={() => openForm()}><Icon name="plus" size={14} /> Add by address</button>
  {#if canHost && !hosted}
    <button type="button" class="btn primary" onclick={openHost}><Icon name="host" size={14} /> Create a server</button>
  {/if}
</PageHead>

<Tabs {tabs} value={tab} label="Servers" onchange={(id) => router.go('servers', { tab: id === 'browse' ? 'browse' : undefined })} />

{#snippet row(p: Profile, mine: boolean)}
  {@const info = app.infos[p.id]}
  {@const compat = serverCompat(typeof info === 'object' ? info.version : null, app.version, app.versions)}
  {@const selected = app.selected?.id === p.id}
  <div class="srv" class:selected class:mine>
    <div class="srv-top">
      <span class="dot-state" class:on={typeof info === 'object'} class:bad={typeof info === 'string'}></span>
      <div class="srv-name">
        <b>{p.name}</b>
        <small>
          <code>{p.url}</code>
          {#if typeof info === 'object'} · {info.online} online{app.pings[p.id] !== undefined ? ` · ${app.pings[p.id]} ms` : ''}
          {:else if typeof info === 'string'} · unreachable{/if}
        </small>
      </div>
      <div class="srv-chips">
        {#if !p.logged_in}<Chip tone="warn">not logged in</Chip>{/if}
        {#if p.role === 'admin'}<Chip tone="info" icon="shield">admin</Chip>{/if}
        {#if p.plain_http === 'private' || p.plain_http === 'public'}<Chip tone="warn" icon="alert">plain http</Chip>{/if}
        {#if compat.kind === 'ok'}<Chip tone="ok">compatible</Chip>
        {:else if compat.kind === 'other'}<Chip tone="bad" icon="alert">needs {compat.version}</Chip>
        {:else if compat.kind === 'unsupported'}<Chip tone="bad" icon="alert">incompatible</Chip>{/if}
      </div>
      <div class="srv-actions">
        {#if selected}<Chip tone="accent" icon="check">selected</Chip>
        {:else}<button type="button" class="btn sm" onclick={() => app.select(p.id)}>Select</button>{/if}
        {#if p.role === 'admin'}
          <button type="button" class="btn sm" onclick={() => app.openWebAdmin(p.id)} title="Opens the server's web admin in your browser, already signed in">
            Web admin <Icon name="external" size={12} />
          </button>
        {/if}
        <button type="button" class="btn sm icon ghost" aria-expanded={isOpen(p)} aria-label="Details of {p.name}" title="Details, login, edit" onclick={() => (open[p.id] = !isOpen(p))}>
          <Icon name="chevron" size={15} class={isOpen(p) ? 'flip' : ''} />
        </button>
      </div>
    </div>
    {#if p.keyring_error}<p class="err">{p.keyring_error}</p>{/if}
    {#if compat.kind === 'other'}
      <p class="srv-warn"><Icon name="alert" size={13} /> This server runs client {compat.version}, you have {app.version} selected.
        <button type="button" class="link" onclick={() => app.switchVersion(compat.version)}>Switch to {compat.version}</button></p>
    {:else if compat.kind === 'unsupported'}
      <p class="srv-warn"><Icon name="alert" size={13} /> This server runs client {compat.version}, which this launcher does not support. Update the launcher or the server.</p>
    {/if}
    {#if isOpen(p)}
      <div class="srv-more" transition:slide={{ duration: ms(DUR.slow), easing: ease }}>
        <PlainHttpNotice profile={p} onchange={() => app.refreshProfiles()} />
        <AccountPanel profile={p} />
        {#if typeof info === 'string'}
          <p class="err">{info}</p>
        {:else if info}
          <dl class="kv">
            <dt>Realm</dt><dd>{info.name}</dd>
            <dt>Client</dt><dd>{info.version}</dd>
            <dt>Uptime</dt><dd>{formatUptime(info.uptime_secs)}</dd>
            <dt>Registration</dt><dd>{info.open_registration ? 'open' : 'closed (an admin creates accounts)'}</dd>
            <dt>Game address</dt><dd><code>{info.login_host}:{info.login_port}</code></dd>
          </dl>
        {/if}
        <div class="row srv-tools">
          <button type="button" class="btn sm" onclick={() => check(p.id)} disabled={busy === p.id}>
            {#if busy === p.id}<Spinner size={12} />{:else}<Icon name="refresh" size={13} />{/if} Check
          </button>
          <button type="button" class="btn sm" onclick={() => openForm(p)}><Icon name="edit" size={13} /> Edit</button>
          <span class="grow"></span>
          <button type="button" class="btn sm danger" onclick={() => remove(p.id, p.name)}><Icon name="trash" size={13} /> Remove</button>
        </div>
      </div>
    {/if}
  </div>
{/snippet}

{#if tab === 'browse'}
  <ServerBrowser />
{:else}
  {#if hosted}
    <h3 class="group">Hosted on this computer</h3>
    <div class="host-row">
      <div class="host-state">
        <Chip tone={hosted.info ? 'ok' : hostUp ? 'warn' : hosted.run.state === 'exited' && hosted.run.code !== 0 && hosted.run.code !== null ? 'bad' : 'neutral'} dot>
          {runLabel(hosted.run, hosted.info != null)}
        </Chip>
        {#if hosted.info}<span class="muted">{hosted.info.online} online · up {formatUptime(hosted.info.uptime_secs)}</span>{/if}
        {#if hosted.package}<span class="faint">server {hosted.package.server_version}</span>{/if}
        <span class="grow"></span>
        {#if hostUp}
          <button type="button" class="btn sm" disabled={hostTask !== '' || hosted.run.state === 'stopping'} onclick={hostStop}>
            {#if hostTask === 'stop'}<Spinner size={12} />{/if} Stop
          </button>
        {:else}
          <button type="button" class="btn sm primary" disabled={hostTask !== '' || !hosted.game_installed} onclick={hostStart}>
            {#if hostTask === 'start'}<Spinner size={12} />{/if} Start
          </button>
        {/if}
        <button type="button" class="btn sm" onclick={openHost} title="Program, settings, log and backups of your server"><Icon name="settings" size={13} /> Manage</button>
      </div>
      {#if hostProfile}
        {@render row(hostProfile, true)}
      {:else}
        <p class="hint" style="margin: 0">{hosted.settings.server_name} is not in your server list yet. Open Manage and apply its settings to add it.</p>
      {/if}
    </div>
    <h3 class="group">Saved</h3>
  {/if}

  {#if app.profiles.length === 0}
    <Empty icon="servers" title="No servers yet">
      Add a friend's server by its address, find one under <a href="#/servers?tab=browse">Browse public</a>{#if canHost}, or create your own on this computer{/if}.
      {#snippet action()}
        <div class="row" style="justify-content: center">
          <button type="button" class="btn" onclick={() => openForm()}><Icon name="plus" size={14} /> Add by address</button>
          {#if canHost}<button type="button" class="btn primary" onclick={openHost}><Icon name="host" size={14} /> Create a server</button>{/if}
        </div>
      {/snippet}
    </Empty>
  {:else if others.length === 0 && hosted}
    <p class="muted">No other servers. Add one by address or browse the public list.</p>
  {:else}
    <div class="srv-list">
      {#each others as p (p.id)}{@render row(p, false)}{/each}
    </div>
  {/if}
{/if}

<dialog bind:this={dialog} class="modal" aria-labelledby="srv-form-title" onclose={() => (editing = null)}>
  <form class="dialog-body" onsubmit={save}>
    <div class="dialog-head"><h2 id="srv-form-title">{editing ? 'Edit server' : 'Add a server'}</h2></div>
    <label class="field"><span>Name</span><input bind:value={name} maxlength="64" placeholder="My cluster" /></label>
    <label class="field">
      <span>Address (web API)</span>
      <input bind:value={url} placeholder="127.0.0.1:8080 or https://play.example" aria-invalid={url !== '' && !normalized} />
      {#if normalized && normalized !== url.trim()}<span class="hint">Will be saved as <code>{normalized}</code></span>{/if}
      {#if url !== '' && !normalized}<span class="err">Not an address</span>{/if}
    </label>
    {#if warning && needsAgreement}
      <div class="notice" role="alert">
        <span class="notice-title"><Icon name="alert" size={16} /> Unencrypted connection</span>
        <p>{warning}</p>
        <label class="check"><input type="checkbox" bind:checked={agreeHttp} /> <span>I understand and want to use plain http for this server</span></label>
      </div>
    {/if}
    {#if editing}<p class="hint">Changing the address signs you out of this server.</p>{/if}
    <div class="actions">
      <button type="button" class="btn" onclick={closeForm}>Cancel</button>
      <button class="btn primary" type="submit" disabled={!valid}>{editing ? 'Save' : 'Add server'}</button>
    </div>
  </form>
</dialog>

{#if hostOpen && !app.official}
  <Drawer title={hosted ? (hostProfile?.name ?? hosted.settings.server_name) : 'Create a server'} onclose={closeHost}>
    <HostPanel />
  </Drawer>
{/if}

<style>
  .group { font: 500 10.5px/1 var(--font); letter-spacing: 0.12em; text-transform: uppercase; color: var(--faint); margin: var(--sp-4) 0 var(--sp-2); }
  .srv-list { display: flex; flex-direction: column; }
  .srv { border: 1px solid var(--line); background: var(--surface); padding: 10px 14px; transition: border-color var(--dur) var(--ease), background var(--dur) var(--ease); }
  .srv-list .srv + .srv { border-top: 0; }
  .srv-list .srv:first-child { border-radius: var(--radius) var(--radius) 0 0; }
  .srv-list .srv:last-child { border-radius: 0 0 var(--radius) var(--radius); }
  .srv-list .srv:only-child { border-radius: var(--radius); }
  .srv.selected { box-shadow: inset 2px 0 0 var(--gold); }
  .srv-top { display: flex; align-items: center; gap: var(--sp-3); flex-wrap: wrap; }
  .srv-name { display: flex; flex-direction: column; min-width: 0; flex: 1 1 200px; }
  .srv-name b { font-weight: 600; font-size: var(--fs-md); }
  .srv-name small { color: var(--muted); font-size: var(--fs-xs); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .srv-name code { background: none; padding: 0; font-size: inherit; }
  .srv-chips, .srv-actions { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .srv-actions :global(.flip) { transform: rotate(180deg); }
  .srv-actions :global(.icon) { transition: transform var(--dur) var(--ease); }
  .srv-warn { display: flex; align-items: center; flex-wrap: wrap; gap: 6px; margin: 8px 0 0; font-size: var(--fs-sm); color: var(--danger); }
  .srv-more { margin-top: var(--sp-1); }
  .srv-more .kv { margin-top: var(--sp-3); font-size: var(--fs-sm); }
  .srv-tools { margin-top: var(--sp-3); }
  .dot-state { width: 8px; height: 8px; border-radius: 50%; background: var(--faint); flex: none; }
  .dot-state.on { background: var(--ok); box-shadow: 0 0 8px var(--ok); }
  .dot-state.bad { background: var(--danger); }
  .host-row { border: 1px solid color-mix(in srgb, var(--gold) 45%, var(--line)); border-radius: var(--radius); background: linear-gradient(90deg, color-mix(in srgb, var(--crimson) 20%, var(--surface)), var(--surface)); padding: var(--sp-3); display: flex; flex-direction: column; gap: var(--sp-3); }
  .host-row .srv { border-radius: var(--radius); }
  .host-state { display: flex; align-items: center; gap: var(--sp-3); flex-wrap: wrap; font-size: var(--fs-sm); }
</style>
