<script lang="ts">
  import { onMount } from 'svelte';
  import { fly, fade } from 'svelte/transition';
  import { flip } from 'svelte/animate';
  import { cubicOut } from 'svelte/easing';
  import { app } from '../lib/app.svelte';
  import { ipc } from '../lib/ipc';
  import { run } from '../lib/notify.svelte';
  import { ask } from '../lib/confirm.svelte';
  import { ms } from '../lib/motion';
  import { needsPlainHttpConfirmation, normalizeServerUrl, plainHttpRisk, plainHttpWarning } from '../lib/url';
  import { formatUptime } from '../lib/format';
  import { serverCompat } from '../lib/gameversion';
  import PageHead from '../lib/ui/PageHead.svelte';
  import PlainHttpNotice from '../lib/ui/PlainHttpNotice.svelte';
  import Icon from '../lib/ui/Icon.svelte';
  import Chip from '../lib/ui/Chip.svelte';
  import Empty from '../lib/ui/Empty.svelte';
  import Spinner from '../lib/ui/Spinner.svelte';
  import SetupChecklist from '../lib/ui/SetupChecklist.svelte';

  let name = $state('');
  let url = $state('');
  let editing = $state<string | null>(null);
  let busy = $state<string | null>(null);
  let formEl: HTMLElement | undefined = $state();

  /** The user ticked "I understand" for the plain http address in the form. */
  let agreeHttp = $state(false);

  const normalized = $derived(normalizeServerUrl(url));
  const warning = $derived(normalized ? plainHttpWarning(plainHttpRisk(normalized)) : null);
  const current = $derived(editing ? app.profiles.find((p) => p.id === editing) : undefined);
  // An address that was confirmed before stays confirmed while it is not changed.
  const alreadyAgreed = $derived(current !== undefined && current.url === normalized && current.plain_http_confirmed);
  const needsAgreement = $derived(normalized !== null && needsPlainHttpConfirmation(normalized) && !alreadyAgreed);
  const valid = $derived(name.trim().length > 0 && normalized !== null && (!needsAgreement || agreeHttp));

  onMount(() => void app.checkServers());

  function edit(id: string, n: string, u: string) {
    editing = id;
    name = n;
    url = u;
    agreeHttp = false;
    formEl?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
  }

  function reset() {
    editing = null;
    name = '';
    url = '';
    agreeHttp = false;
  }

  async function save(e: SubmitEvent) {
    e.preventDefault();
    const r = await run(
      () => (editing ? ipc.profileUpdate(editing, name, url, agreeHttp) : ipc.profileAdd(name, url, agreeHttp)),
      editing ? 'Server updated' : 'Server added',
    );
    if (r) {
      reset();
      await Promise.all([app.refreshProfiles(), app.refreshSettings()]);
      void app.checkServer(r.id);
    }
  }

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

<PageHead icon="servers" title="Servers" sub="A server profile is the address of a cluster's web API. Pick one in the bar below to play on it." />

<SetupChecklist />

<div class="two">
  <section class="card" id="add-server" bind:this={formEl}>
    <div class="card-head">
      <span class="head-icon"><Icon name={editing ? 'edit' : 'plus'} size={16} /></span>
      <h2>{editing ? 'Edit server' : 'Add a server'}</h2>
    </div>
    <form onsubmit={save}>
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
        {#if editing}<button type="button" class="btn" onclick={reset}>Cancel</button>{/if}
        <button class="btn primary" type="submit" disabled={!valid}>{editing ? 'Save' : 'Add server'}</button>
      </div>
    </form>
  </section>

  <section>
    {#if app.profiles.length === 0}
      <Empty icon="servers" title="No servers yet">
        Add the address of a server's web API on the left, or set one up on this computer with <a href="#/host">Host a server</a>.
      </Empty>
    {/if}
    {#each app.profiles as p (p.id)}
      {@const info = app.infos[p.id]}
      {@const compat = serverCompat(typeof info === 'object' ? info.version : null, app.version, app.versions)}
      <div
        class="card server-card hover"
        class:selected={app.selected?.id === p.id}
        in:fly={{ y: 14, duration: ms(260), easing: cubicOut }}
        out:fade={{ duration: ms(140) }}
        animate:flip={{ duration: ms(220) }}
      >
        <div class="server-top">
          <div class="grow">
            <div class="server-name">
              <strong>{p.name}</strong>
              {#if app.selected?.id === p.id}<Chip tone="accent" icon="check">selected</Chip>{/if}
              {#if p.logged_in}<Chip tone="ok" dot>{p.account}</Chip>{:else}<Chip>not logged in</Chip>{/if}
              {#if p.role === 'admin'}<Chip tone="info" icon="shield">admin</Chip>{/if}
              {#if p.plain_http === 'private' || p.plain_http === 'public'}<Chip tone="warn" icon="alert">plain http</Chip>{/if}
              {#if compat.kind === 'ok'}
                <Chip tone="ok" icon="check">compatible</Chip>
              {:else if compat.kind === 'other'}
                <Chip tone="bad" icon="alert">needs client {compat.version}</Chip>
              {:else if compat.kind === 'unsupported'}
                <Chip tone="bad" icon="alert">incompatible: client {compat.version}</Chip>
              {/if}
            </div>
            <p class="server-url muted"><code>{p.url}</code></p>
          </div>
          <div class="server-actions">
            <button type="button" class="btn sm" onclick={() => app.select(p.id)} disabled={app.selected?.id === p.id}>Select</button>
            {#if p.role === 'admin'}
              <button type="button" class="btn sm" onclick={() => app.openWebAdmin(p.id)} title="Opens the server's web admin in your browser, already signed in">
                <Icon name="external" size={13} /> Web admin
              </button>
            {/if}
            <button type="button" class="btn sm" onclick={() => check(p.id)} disabled={busy === p.id}>
              {#if busy === p.id}<Spinner size={12} />{:else}<Icon name="refresh" size={13} />{/if} Check
            </button>
            <button type="button" class="btn sm ghost" onclick={() => edit(p.id, p.name, p.url)}><Icon name="edit" size={13} /> Edit</button>
            <button type="button" class="btn sm danger" onclick={() => remove(p.id, p.name)}><Icon name="trash" size={13} /></button>
          </div>
        </div>
        {#if p.keyring_error}<p class="err" style="margin-top: 10px">{p.keyring_error}</p>{/if}
        <PlainHttpNotice profile={p} onchange={() => app.refreshProfiles()} />
        {#if compat.kind === 'other'}
          <div class="notice danger" style="margin: 12px 0 0">
            <span class="notice-title"><Icon name="alert" size={16} /> Another game version</span>
            <p>This server runs client {compat.version}, you have {app.version} selected.
              <button type="button" class="link" onclick={() => app.switchVersion(compat.version)}>Switch to {compat.version}</button></p>
          </div>
        {:else if compat.kind === 'unsupported'}
          <div class="notice danger" style="margin: 12px 0 0">
            <span class="notice-title"><Icon name="alert" size={16} /> Incompatible</span>
            <p>This server runs client {compat.version}, which this launcher does not support. You can still try to play, but the game will probably not work. Update the launcher or the server.</p>
          </div>
        {/if}
        {#if typeof info === 'string'}
          <p class="err server-info">{info}</p>
        {:else if info}
          <div class="server-info">
            <div class="stat-row">
              <div class="stat"><small>Online</small><b>{info.online}</b></div>
              <div class="stat"><small>Client</small><b>{info.version}</b></div>
              <div class="stat"><small>Uptime</small><b>{formatUptime(info.uptime_secs)}</b></div>
            </div>
            <dl class="kv">
              <dt>Realm</dt><dd>{info.name}</dd>
              <dt>Registration</dt><dd>{info.open_registration ? 'open' : 'closed (an admin creates accounts)'}</dd>
              <dt>Game address</dt><dd><code>{info.login_host}:{info.login_port}</code></dd>
            </dl>
          </div>
        {/if}
      </div>
    {/each}
  </section>
</div>
