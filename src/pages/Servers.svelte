<script lang="ts">
  import { app } from '../lib/app.svelte';
  import { ipc } from '../lib/ipc';
  import { run } from '../lib/notify.svelte';
  import { needsPlainHttpConfirmation, normalizeServerUrl, plainHttpRisk, plainHttpWarning } from '../lib/url';
  import { formatUptime } from '../lib/format';
  import PageHead from '../lib/ui/PageHead.svelte';
  import PlainHttpNotice from '../lib/ui/PlainHttpNotice.svelte';
  import type { ServerInfo } from '../lib/types';

  let name = $state('');
  let url = $state('');
  let editing = $state<string | null>(null);
  let infos = $state<Record<string, ServerInfo | string>>({});
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

  function edit(id: string, n: string, u: string) {
    editing = id;
    name = n;
    url = u;
    agreeHttp = false;
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
    }
  }

  async function check(id: string) {
    busy = id;
    try {
      infos[id] = await ipc.serverInfo(id);
      // Also re-read the account's role: a promotion shows without logging in again.
      if (app.profiles.find((p) => p.id === id)?.logged_in) {
        try {
          await ipc.accountRefresh(id);
          await app.refreshProfiles();
        } catch {
          /* the Check result above is what matters */
        }
      }
    } catch (e) {
      infos[id] = e instanceof Error ? e.message : String(e);
    }
    busy = null;
  }

  async function remove(id: string, label: string) {
    if (!confirm(`Remove the server "${label}"? Its login is revoked and forgotten.`)) return;
    if ((await run(() => ipc.profileRemove(id), 'Server removed')) !== undefined) {
      await Promise.all([app.refreshProfiles(), app.refreshSettings()]);
    }
  }
</script>

<PageHead title="Servers" sub="A server profile is the address of a cluster's web API." />

<div class="two">
  <section class="card">
    <h2>{editing ? 'Edit server' : 'Add a server'}</h2>
    <form onsubmit={save}>
      <label class="field"><span>Name</span><input bind:value={name} maxlength="64" placeholder="My cluster" /></label>
      <label class="field">
        <span>Address (web API)</span>
        <input bind:value={url} placeholder="127.0.0.1:8080 or https://play.example" aria-invalid={url !== '' && !normalized} />
        {#if normalized && normalized !== url.trim()}<span class="hint">Will be saved as {normalized}</span>{/if}
        {#if url !== '' && !normalized}<span class="err">Not an address</span>{/if}
      </label>
      {#if warning && needsAgreement}
        <div class="notice" role="alert">
          <p><b>Unencrypted connection.</b> {warning}</p>
          <label class="check"><input type="checkbox" bind:checked={agreeHttp} /> I understand and want to use plain http for this server</label>
        </div>
      {/if}
      {#if editing}<p class="hint">Changing the address signs you out of this server.</p>{/if}
      <div class="actions">
        {#if editing}<button type="button" onclick={reset}>Cancel</button>{/if}
        <button class="primary" type="submit" disabled={!valid}>{editing ? 'Save' : 'Add server'}</button>
      </div>
    </form>
  </section>

  <section class="stack">
    {#if app.profiles.length === 0}
      <div class="card empty muted">No servers yet. Add one on the left.</div>
    {/if}
    {#each app.profiles as p (p.id)}
      {@const info = infos[p.id]}
      <div class="card">
        <div class="row">
          <strong>{p.name}</strong>
          {#if app.selected?.id === p.id}<span class="pill ok">selected</span>{/if}
          {#if p.logged_in}<span class="pill ok">logged in as {p.account}</span>{:else}<span class="pill">not logged in</span>{/if}
          {#if p.role === 'admin'}<span class="pill">admin</span>{/if}
          {#if p.plain_http === 'private' || p.plain_http === 'public'}<span class="pill warn">plain http</span>{/if}
          <span class="grow"></span>
          <button type="button" onclick={() => app.select(p.id)} disabled={app.selected?.id === p.id}>Select</button>
          {#if p.role === 'admin'}
            <button type="button" onclick={() => app.openWebAdmin(p.id)} title="Opens the server's web admin in your browser, already signed in">Open web admin</button>
          {/if}
          <button type="button" onclick={() => check(p.id)} disabled={busy === p.id}>Check</button>
          <button type="button" onclick={() => edit(p.id, p.name, p.url)}>Edit</button>
          <button type="button" class="danger" onclick={() => remove(p.id, p.name)}>Remove</button>
        </div>
        <p class="muted"><code>{p.url}</code></p>
        {#if p.keyring_error}<p class="err">{p.keyring_error}</p>{/if}
        <PlainHttpNotice profile={p} onchange={() => app.refreshProfiles()} />
        {#if typeof info === 'string'}
          <p class="err">{info}</p>
        {:else if info}
          <dl class="kv">
            <dt>Realm</dt><dd>{info.name}</dd>
            <dt>Client version</dt>
            <dd>
              {info.version}
              {#if app.status && info.version !== app.status.version}
                <span class="pill warn">your install is {app.status.version}</span>
              {/if}
            </dd>
            <dt>Players online</dt><dd>{info.online}</dd>
            <dt>Registration</dt><dd>{info.open_registration ? 'open' : 'closed (an admin creates accounts)'}</dd>
            <dt>Game address</dt><dd><code>{info.login_host}:{info.login_port}</code></dd>
            <dt>Uptime</dt><dd>{formatUptime(info.uptime_secs)}</dd>
          </dl>
        {/if}
      </div>
    {/each}
  </section>
</div>
