<script lang="ts">
  import { onDestroy, onMount, tick } from 'svelte';
  import { slide } from 'svelte/transition';
  import { app } from '../lib/app.svelte';
  import { ipc } from '../lib/ipc';
  import { run } from '../lib/notify.svelte';
  import { ask } from '../lib/confirm.svelte';
  import { DUR, ease, ms } from '../lib/motion';
  import { listenHost } from '../lib/events';
  import { normalizeServerUrl } from '../lib/url';
  import { formatUptime } from '../lib/format';
  import { serverCompat, versionLabel } from '../lib/gameversion';
  import { isUp, runLabel } from '../lib/host';
  import { router } from '../lib/router.svelte';
  import PageHead from '../lib/ui/PageHead.svelte';
  import Icon from '../lib/ui/Icon.svelte';
  import Chip from '../lib/ui/Chip.svelte';
  import Empty from '../lib/ui/Empty.svelte';
  import Spinner from '../lib/ui/Spinner.svelte';
  import Tabs from '../lib/ui/Tabs.svelte';
  import ServerBrowser from '../lib/ui/ServerBrowser.svelte';
  import Drawer from '../lib/ui/Drawer.svelte';
  import HostPanel from '../lib/ui/HostPanel.svelte';
  import type { HostStatus, ProfileView as Profile } from '../lib/types';

  // Servers: the ones you can play on (My servers), the public list (Browse public), and the ones
  // that run on this computer (Host: create, start, stop, manage, delete).
  type TabId = 'mine' | 'browse' | 'host';
  const tab = $derived<TabId>(
    ((t) => (t === 'browse' || t === 'host' ? t : 'mine'))(router.route.query.get('tab')),
  );

  // --- the servers hosted on this computer ----------------------------------------------------
  let hosts = $state<HostStatus[]>([]);
  let hostsLoaded = $state(false);
  let hostTask = $state<Record<string, '' | 'start' | 'stop'>>({});
  const canHost = $derived(!app.official);
  const manageId = $derived(router.route.query.get('manage'));
  const managed = $derived(hosts.find((h) => h.id === manageId) ?? null);
  /** Profile id -> its hosted server. */
  const hostedBy = $derived(new Map(hosts.filter((h) => h.settings.profile_id).map((h) => [h.settings.profile_id!, h])));
  let timer: ReturnType<typeof setInterval> | undefined;
  let unlisten: (() => void) | undefined;

  const tabs = $derived([
    { id: 'mine', label: 'My servers', icon: 'servers' as const, badge: String(app.profiles.length) },
    { id: 'browse', label: 'Browse public', icon: 'compass' as const },
    ...(canHost ? [{ id: 'host', label: 'Host', icon: 'host' as const, badge: String(hosts.length) }] : []),
  ]);

  async function refreshHosts() {
    if (app.official) return;
    try {
      hosts = await ipc.hostList();
      app.hostIds = hosts.map((h) => h.id);
    } catch {
      /* hosting is optional */
    }
    hostsLoaded = true;
  }

  function goTab(id: string) {
    router.go('servers', { tab: id === 'mine' ? undefined : id });
  }
  function openManage(id: string) {
    router.go('servers', { tab: 'host', manage: id });
  }
  function closeManage() {
    router.go('servers', { tab: 'host' });
    void refreshHosts();
  }

  async function hostStart(id: string) {
    hostTask = { ...hostTask, [id]: 'start' };
    await run(() => ipc.hostStart(id));
    hostTask = { ...hostTask, [id]: '' };
    await refreshHosts();
  }
  async function hostStop(id: string) {
    hostTask = { ...hostTask, [id]: 'stop' };
    await run(() => ipc.hostStop(id));
    hostTask = { ...hostTask, [id]: '' };
    await refreshHosts();
  }

  /** Deletes a hosted server for good, after the user typed its name. */
  async function hostDelete(h: HostStatus) {
    const name = h.settings.server_name;
    const ok = await ask({
      title: `Delete "${name}"?`,
      message: `The server is stopped and its folder is deleted: database (accounts, characters), settings, backups and log. It also leaves My servers.\n\nThis cannot be undone.`,
      confirmLabel: 'Delete server',
      danger: true,
      typeToConfirm: name,
    });
    if (!ok) return;
    if ((await run(() => ipc.hostDelete(h.id), `${name} deleted`)) !== undefined) {
      if (manageId === h.id) router.go('servers', { tab: 'host' });
      await Promise.all([refreshHosts(), app.refreshProfiles(), app.refreshSettings()]);
      app.forgetServerUpdate(h.id);
    }
  }

  // --- create ---------------------------------------------------------------------------------
  let createDialog: HTMLDialogElement | undefined = $state();
  let createName = $state('');
  let creating = $state(false);

  async function openCreate() {
    createName = hosts.length === 0 ? 'My server' : `My server ${hosts.length + 1}`;
    await tick();
    createDialog?.showModal();
  }

  async function create(e: SubmitEvent) {
    e.preventDefault();
    creating = true;
    const id = await run(() => ipc.hostCreate(createName.trim()));
    creating = false;
    if (id) {
      createDialog?.close();
      await refreshHosts();
      openManage(id);
    }
  }

  onMount(async () => {
    void app.checkServers();
    await refreshHosts();
    unlisten = await listenHost(null, { onState: () => void refreshHosts() });
    timer = setInterval(() => void refreshHosts(), 4000);
    if (router.route.query.get('create') === '1') {
      router.go('servers', { tab: 'host' });
      if (canHost) void openCreate();
    }
    if (router.route.query.get('add') === '1') {
      router.go('servers');
      void openForm();
    }
  });
  onDestroy(() => {
    if (timer) clearInterval(timer);
    unlisten?.();
  });

  // --- add or edit by address ---------------------------------------------------------------
  let dialog: HTMLDialogElement | undefined = $state();
  let name = $state('');
  let url = $state('');
  let editing = $state<string | null>(null);
  let busy = $state<string | null>(null);
  const normalized = $derived(normalizeServerUrl(url));
  const valid = $derived(name.trim().length > 0 && normalized !== null);

  async function openForm(p?: Profile) {
    editing = p?.id ?? null;
    name = p?.name ?? '';
    url = p?.url ?? '';
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
      () => (editing ? ipc.profileUpdate(editing, name, url, false) : ipc.profileAdd(name, url, false)),
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
  const isOpen = (p: Profile) => open[p.id] ?? false;

  async function check(id: string) {
    busy = id;
    await app.checkServer(id);
    busy = null;
  }

  async function remove(p: Profile) {
    const h = hostedBy.get(p.id);
    // A server hosted here is deleted, not just forgotten: no server is left behind unlisted.
    if (h) return hostDelete(h);
    const ok = await ask({
      title: `Remove "${p.name}"?`,
      message: 'It leaves your list. The server itself is not touched.',
      confirmLabel: 'Remove server',
      danger: true,
    });
    if (!ok) return;
    if ((await run(() => ipc.profileRemove(p.id), 'Server removed')) !== undefined) {
      await Promise.all([app.refreshProfiles(), app.refreshSettings()]);
    }
  }
</script>

<PageHead title="Servers" sub={`For ${versionLabel(app.version)}`} />

<div class="tabrow">
  <Tabs {tabs} value={tab} label="Servers" onchange={goTab} />
  <span class="grow"></span>
  {#if tab === 'mine' && app.profiles.length > 0}
    <button type="button" class="btn sm" onclick={() => openForm()}><Icon name="plus" size={14} /> Add by address</button>
  {:else if tab === 'host' && hosts.length > 0}
    <button type="button" class="btn sm primary" onclick={openCreate}><Icon name="plus" size={14} /> Create a server</button>
  {/if}
</div>

{#snippet row(p: Profile)}
  {@const info = app.infos[p.id]}
  {@const compat = serverCompat(typeof info === 'object' ? info.version : null, app.version, app.versions)}
  {@const selected = app.selected?.id === p.id}
  {@const hosted = hostedBy.get(p.id)}
  <div class="srv" class:selected>
    <div class="srv-top">
      <span class="dot-state" class:on={typeof info === 'object'} class:bad={typeof info === 'string' && !hosted}></span>
      <div class="srv-name">
        <b>{p.name}</b>
        <small>
          <code>{p.url}</code>
          {#if typeof info === 'object'} · {info.online} online{app.pings[p.id] !== undefined ? ` · ${app.pings[p.id]} ms` : ''}
          {:else if hosted && !isUp(hosted.run)} · stopped
          {:else if typeof info === 'string'} · unreachable{/if}
        </small>
      </div>
      <div class="srv-chips">
        {#if hosted}<Chip tone="accent" icon="host">hosted</Chip>{/if}
        {#if compat.kind === 'other'}<Chip tone="bad" icon="alert">needs {compat.version}</Chip>
        {:else if compat.kind === 'unsupported'}<Chip tone="bad" icon="alert">incompatible</Chip>{/if}
      </div>
      <div class="srv-actions">
        {#if hosted && !isUp(hosted.run)}
          <button type="button" class="btn sm primary" disabled={!!hostTask[hosted.id] || !hosted.settings.configured} onclick={() => hostStart(hosted.id)}>
            {#if hostTask[hosted.id] === 'start'}<Spinner size={12} />{/if} Start
          </button>
        {/if}
        {#if selected}<Chip tone="accent" icon="check">selected</Chip>
        {:else}<button type="button" class="btn sm" onclick={() => app.select(p.id)}>Select</button>{/if}
        <button type="button" class="btn sm icon ghost" aria-expanded={isOpen(p)} aria-label="Details of {p.name}" title="Details, web admin, edit" onclick={() => (open[p.id] = !isOpen(p))}>
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
        {#if typeof info === 'string'}
          {#if !hosted}<p class="err">{info}</p>{/if}
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
          <button type="button" class="btn sm" onclick={() => (hosted ? void run(() => ipc.hostWebAdminOpen(hosted.id)) : app.openServerPage(p.id, 'admin'))}
            title="The server's web admin in your browser (it asks for its own login)">Web admin <Icon name="external" size={12} /></button>
          {#if typeof info === 'object' && info.open_registration}
            <button type="button" class="btn sm" onclick={() => app.openServerPage(p.id, 'register')} title="The server's page for new accounts">Create account <Icon name="external" size={12} /></button>
          {/if}
          {#if hosted}
            <button type="button" class="btn sm" onclick={() => openManage(hosted.id)}><Icon name="settings" size={13} /> Manage</button>
          {:else}
            <button type="button" class="btn sm" onclick={() => openForm(p)}><Icon name="edit" size={13} /> Edit</button>
          {/if}
          <span class="grow"></span>
          <button type="button" class="btn sm danger" onclick={() => remove(p)}>
            <Icon name="trash" size={13} /> {hosted ? 'Delete server' : 'Remove'}
          </button>
        </div>
      </div>
    {/if}
  </div>
{/snippet}

{#if tab === 'browse'}
  <ServerBrowser />
{:else if tab === 'host'}
  {#if !hostsLoaded}
    <div class="loading"><Spinner size={18} /> Loading...</div>
  {:else if hosts.length === 0}
    <Empty icon="host" title="No server on this computer yet">
      Run a private PoE2 server here and let friends join it. You can run several, each on its own ports.
      {#snippet action()}
        <button type="button" class="btn primary" onclick={openCreate}><Icon name="plus" size={14} /> Create a server</button>
      {/snippet}
    </Empty>
  {:else}
    <div class="srv-list">
      {#each hosts as h (h.id)}
        {@const up = isUp(h.run)}
        {@const ready = h.info != null}
        <div class="srv host">
          <div class="srv-top">
            <span class="dot-state" class:on={ready} class:warm={up && !ready}></span>
            <div class="srv-name">
              <b>{h.settings.server_name}</b>
              <small>login {h.settings.login_port} · game {h.settings.instance_port} · web {h.settings.web_port}{h.package ? ` · server ${h.package.server_version}` : ''}</small>
            </div>
            <div class="srv-chips">
              <Chip tone={ready ? 'ok' : up ? 'warn' : h.run.state === 'exited' && h.run.code !== 0 && h.run.code !== null ? 'bad' : 'neutral'} dot>
                {runLabel(h.run, ready).replace(/ \(pid \d+\)/, '')}
              </Chip>
              {#if h.info}<span class="muted small">{h.info.online} online · up {formatUptime(h.info.uptime_secs)}</span>{/if}
              {#if !h.settings.configured}<Chip tone="warn">not set up</Chip>{/if}
              {#if app.serverUpdates[h.id]?.update_available}<Chip tone="accent" icon="download">update</Chip>{/if}
            </div>
            <div class="srv-actions">
              {#if up}
                <button type="button" class="btn sm" disabled={!!hostTask[h.id] || h.run.state === 'stopping'} onclick={() => hostStop(h.id)}>
                  {#if hostTask[h.id] === 'stop'}<Spinner size={12} />{/if} Stop
                </button>
              {:else}
                <button type="button" class="btn sm primary" disabled={!!hostTask[h.id] || !h.settings.configured || !h.game_installed} onclick={() => hostStart(h.id)}
                  title={!h.settings.configured ? 'Apply its settings first (Manage)' : !h.game_installed ? 'Install the game first (Library)' : ''}>
                  {#if hostTask[h.id] === 'start'}<Spinner size={12} />{/if} Start
                </button>
              {/if}
              <button type="button" class="btn sm" onclick={() => openManage(h.id)}><Icon name="settings" size={13} /> Manage</button>
            </div>
          </div>
        </div>
      {/each}
    </div>
    <p class="hint host-hint">Servers run while the launcher is open. Each one has its own folder: database, settings, backups and log. Downloaded server programs are shared.</p>
  {/if}
{:else if app.profiles.length === 0}
  <Empty icon="servers" title="No servers yet">
    Add a friend's server by its address, find one under <a href="#/servers?tab=browse">Browse public</a>{#if canHost}, or host your own under <a href="#/servers?tab=host">Host</a>{/if}.
    {#snippet action()}
      <button type="button" class="btn primary" onclick={() => openForm()}><Icon name="plus" size={14} /> Add by address</button>
    {/snippet}
  </Empty>
{:else}
  <div class="srv-list">
    {#each app.profiles as p (p.id)}{@render row(p)}{/each}
  </div>
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
    <p class="hint">The web address the server's owner gave you (its web API). Accounts are the server's: you sign in on the game's login screen.</p>
    <div class="actions">
      <button type="button" class="btn" onclick={closeForm}>Cancel</button>
      <button class="btn primary" type="submit" disabled={!valid}>{editing ? 'Save' : 'Add server'}</button>
    </div>
  </form>
</dialog>

<dialog bind:this={createDialog} class="modal" aria-labelledby="create-title">
  <form class="dialog-body" onsubmit={create}>
    <div class="dialog-head"><h2 id="create-title">Create a server</h2></div>
    <p class="muted">A new server on this computer. It gets free ports by itself; you can change them in its settings.</p>
    <label class="field"><span>Server name (shown to players)</span><input bind:value={createName} maxlength="64" /></label>
    <div class="actions">
      <button type="button" class="btn" onclick={() => createDialog?.close()}>Cancel</button>
      <button class="btn primary" type="submit" disabled={creating || !createName.trim()}>{#if creating}<Spinner size={14} />{/if} Create</button>
    </div>
  </form>
</dialog>

{#if manageId && managed && !app.official}
  <Drawer title={managed.settings.server_name} onclose={closeManage}>
    {#key manageId}
      <HostPanel id={manageId} ondelete={() => hostDelete(managed)} onchange={refreshHosts} />
    {/key}
  </Drawer>
{/if}

<style>
  .tabrow { display: flex; align-items: flex-end; gap: var(--sp-3); margin-bottom: var(--sp-5); border-bottom: 1px solid var(--line); }
  .tabrow :global(.tabs) { border-bottom: 0; margin: 0; }
  .tabrow > .btn { margin-bottom: var(--sp-2); }
  .srv-list { display: flex; flex-direction: column; }
  .srv { border: 1px solid var(--line); background: var(--surface); padding: 14px 18px; transition: border-color var(--dur) var(--ease), background var(--dur) var(--ease); }
  .srv-list .srv + .srv { border-top: 0; }
  .srv-list .srv:first-child { border-radius: var(--radius-lg) var(--radius-lg) 0 0; }
  .srv-list .srv:last-child { border-radius: 0 0 var(--radius-lg) var(--radius-lg); }
  .srv-list .srv:only-child { border-radius: var(--radius-lg); }
  .srv.selected { box-shadow: inset 2px 0 0 var(--gold); }
  .srv-top { display: flex; align-items: center; gap: var(--sp-3); flex-wrap: wrap; }
  .srv-name { display: flex; flex-direction: column; min-width: 0; flex: 1 1 200px; gap: 2px; }
  .srv-name b { font-weight: 600; font-size: var(--fs-md); }
  .srv-name small { color: var(--muted); font-size: var(--fs-xs); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .srv-name code { background: none; padding: 0; font-size: inherit; }
  .srv-chips, .srv-actions { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .srv-actions :global(.flip) { transform: rotate(180deg); }
  .srv-actions :global(.icon) { transition: transform var(--dur) var(--ease); }
  .srv-warn { display: flex; align-items: center; flex-wrap: wrap; gap: 6px; margin: 8px 0 0; font-size: var(--fs-sm); color: var(--danger); }
  .srv-more { margin-top: var(--sp-2); }
  .srv-more .kv { margin-top: var(--sp-3); font-size: var(--fs-sm); }
  .srv-tools { margin-top: var(--sp-3); }
  .dot-state { width: 8px; height: 8px; border-radius: 50%; background: var(--faint); flex: none; }
  .dot-state.on { background: var(--ok); box-shadow: 0 0 8px var(--ok); }
  .dot-state.warm { background: var(--warn); }
  .dot-state.bad { background: var(--danger); }
  .small { font-size: var(--fs-xs); }
  .host-hint { margin-top: var(--sp-4); }
</style>
