<script lang="ts">
  import { app } from '../lib/app.svelte';
  import { ipc } from '../lib/ipc';
  import { run, toasts } from '../lib/notify.svelte';
  import { formatExpiry } from '../lib/format';
  import PageHead from '../lib/ui/PageHead.svelte';
  import Icon from '../lib/ui/Icon.svelte';
  import Chip from '../lib/ui/Chip.svelte';
  import Empty from '../lib/ui/Empty.svelte';
  import Spinner from '../lib/ui/Spinner.svelte';
  import PlainHttpNotice from '../lib/ui/PlainHttpNotice.svelte';
  import { isPlainHttpBlocked } from '../lib/url';
  import type { ServerInfo } from '../lib/types';

  let username = $state('');
  let password = $state('');
  let password2 = $state('');
  let mode = $state<'login' | 'register'>('login');
  let busy = $state(false);
  let info = $state<ServerInfo | null>(null);
  let infoError = $state('');

  const p = $derived(app.selected);
  const blocked = $derived(p ? isPlainHttpBlocked(p) : false);

  // The server's registration switch decides whether the register tab is offered.
  $effect(() => {
    const id = p?.id;
    info = null;
    infoError = '';
    if (!id) return;
    ipc
      .serverInfo(id)
      .then((i) => {
        if (p?.id === id) info = i;
      })
      .catch((e) => {
        if (p?.id === id) infoError = e instanceof Error ? e.message : String(e);
      });
  });

  const canRegister = $derived(password.length >= 8 && password === password2 && username.trim().length >= 3);

  async function login(e: SubmitEvent) {
    e.preventDefault();
    if (!p) return;
    busy = true;
    const r = await run(() => ipc.login(p.id, username, password), 'Logged in');
    busy = false;
    if (r) {
      password = '';
      await app.refreshProfiles();
    }
  }

  async function register(e: SubmitEvent) {
    e.preventDefault();
    if (!p) return;
    busy = true;
    const created = await run(() => ipc.register(p.id, username, password), 'Account created');
    if (created) {
      // Straight into a login with the same credentials.
      const r = await run(() => ipc.login(p.id, username, password), 'Logged in');
      if (r) await app.refreshProfiles();
      password = '';
      password2 = '';
      mode = 'login';
    }
    busy = false;
  }

  async function logout() {
    if (!p) return;
    const r = await run(() => ipc.logout(p.id));
    if (r) toasts.show(r.revoked ? 'ok' : 'error', r.message);
    await app.refreshProfiles();
  }
</script>

<PageHead icon="account" title="Account" sub={p ? `Server: ${p.name}` : 'Pick a server first'} />

{#if !p}
  <Empty icon="account" title="No server selected">
    Add a server on the <a href="#/servers">Servers page</a> first, then log in or create an account here.
  </Empty>
{:else if p.logged_in}
  <section class="card" style="max-width: 640px">
    <div class="card-head">
      <span class="head-icon"><Icon name="check" size={16} /></span>
      <h2>Signed in</h2>
      <span class="grow"></span>
      <Chip tone="ok" dot>{p.role ?? 'unknown role'}</Chip>
    </div>
    <dl class="kv">
      <dt>Account</dt><dd><strong>{p.account}</strong></dd>
      <dt>Server</dt><dd>{p.name} <span class="muted"><code>{p.url}</code></span></dd>
      <dt>Role</dt><dd>{p.role ?? 'unknown'}</dd>
      <dt>Login valid</dt><dd>{formatExpiry(p.expires_at)}</dd>
    </dl>
    <PlainHttpNotice profile={p} onchange={() => app.refreshProfiles()} />
    <p class="hint" style="margin-top: 14px">
      The login token is kept in the system keyring, never in a file. Play signs the game in with it by itself.
    </p>
    <div class="actions">
      {#if p.role === 'admin'}
        <button type="button" class="btn" onclick={() => app.openWebAdmin(p.id)} title="Opens the server's web admin in your browser, already signed in">
          <Icon name="external" size={14} /> Open web admin
        </button>
      {/if}
      <button class="btn danger" type="button" onclick={logout}><Icon name="logout" size={14} /> Log out</button>
    </div>
  </section>
{:else}
  <section class="card" style="max-width: 460px">
    {#if p.keyring_error}<p class="err">{p.keyring_error}</p>{/if}
    <PlainHttpNotice profile={p} onchange={() => app.refreshProfiles()} />
    {#if p.account}
      <div class="notice"><span class="notice-title"><Icon name="alert" size={16} /> Login expired</span><p>Your saved login for {p.account} has expired. Log in again.</p></div>
    {/if}
    {#if infoError}<p class="err">{infoError}</p>{/if}
    <div class="seg" role="tablist">
      <button type="button" class:on={mode === 'login'} role="tab" aria-selected={mode === 'login'} onclick={() => (mode = 'login')}>Log in</button>
      <button
        type="button"
        class:on={mode === 'register'}
        role="tab"
        aria-selected={mode === 'register'}
        disabled={info !== null && !info.open_registration}
        title={info && !info.open_registration ? 'This server has closed registration' : ''}
        onclick={() => (mode = 'register')}
      >
        Create account
      </button>
    </div>
    {#if info && !info.open_registration}<p class="hint">Registration is closed on this server: an admin creates accounts.</p>{/if}
    {#if mode === 'login'}
      <form onsubmit={login}>
        <label class="field"><span>Account name</span><input bind:value={username} autocomplete="username" /></label>
        <label class="field"><span>Password</span><input type="password" bind:value={password} autocomplete="current-password" /></label>
        <div class="actions">
          <button class="btn primary" type="submit" disabled={busy || blocked || !username.trim() || !password}>
            {#if busy}<Spinner size={14} />{/if} Log in
          </button>
        </div>
      </form>
    {:else}
      <form onsubmit={register}>
        <label class="field">
          <span>Account name (3-64 characters: letters, digits, _ . @ + -)</span>
          <input bind:value={username} autocomplete="username" />
        </label>
        <label class="field"><span>Password (at least 8 characters)</span><input type="password" bind:value={password} autocomplete="new-password" /></label>
        <label class="field">
          <span>Repeat the password</span>
          <input type="password" bind:value={password2} autocomplete="new-password" aria-invalid={password2 !== '' && password !== password2} />
        </label>
        <div class="actions">
          <button class="btn primary" type="submit" disabled={busy || blocked || !canRegister}>
            {#if busy}<Spinner size={14} />{/if} Create account
          </button>
        </div>
      </form>
    {/if}
    <p class="hint" style="margin-top: 14px">The password goes to the server you chose{p.plain_http === 'https' ? ' over https.' : p.plain_http === 'loopback' ? ' on this computer.' : ' (use https outside your own network).'}</p>
  </section>
{/if}
