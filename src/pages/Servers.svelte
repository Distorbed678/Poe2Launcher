<script lang="ts">
  import { app } from '../lib/app.svelte';
  import { ipc } from '../lib/ipc';
  import { run } from '../lib/notify.svelte';
  import { normalizeServerUrl } from '../lib/url';
  import { formatUptime } from '../lib/format';
  import PageHead from '../lib/ui/PageHead.svelte';
  import type { ServerInfo } from '../lib/types';

  let name = $state('');
  let url = $state('');
  let editing = $state<string | null>(null);
  let infos = $state<Record<string, ServerInfo | string>>({});
  let busy = $state<string | null>(null);

  const normalized = $derived(normalizeServerUrl(url));
  const valid = $derived(name.trim().length > 0 && normalized !== null);

  function edit(id: string, n: string, u: string) {
    editing = id;
    name = n;
    url = u;
  }

  function reset() {
    editing = null;
    name = '';
    url = '';
  }

  async function save(e: SubmitEvent) {
    e.preventDefault();
    const r = await run(
      () => (editing ? ipc.profileUpdate(editing, name, url) : ipc.profileAdd(name, url)),
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
          <span class="grow"></span>
          <button type="button" onclick={() => app.select(p.id)} disabled={app.selected?.id === p.id}>Select</button>
          <button type="button" onclick={() => check(p.id)} disabled={busy === p.id}>Check</button>
          <button type="button" onclick={() => edit(p.id, p.name, p.url)}>Edit</button>
          <button type="button" class="danger" onclick={() => remove(p.id, p.name)}>Remove</button>
        </div>
        <p class="muted"><code>{p.url}</code></p>
        {#if p.keyring_error}<p class="err">{p.keyring_error}</p>{/if}
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
