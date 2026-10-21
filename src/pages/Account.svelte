<script lang="ts">
  import { app } from '../lib/app.svelte';
  import { ipc } from '../lib/ipc';
  import { run, toasts } from '../lib/notify.svelte';
  import { formatExpiry } from '../lib/format';
  import PageHead from '../lib/ui/PageHead.svelte';
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

<PageHead title="Account" sub={p ? `Server: ${p.name}` : 'Pick a server first'} />

{#if !p}
  <div class="card empty muted">Add a server on the Servers page.</div>
{:else if p.logged_in}
  <section class="card">
    <h2>Signed in</h2>
    <dl class="kv">
      <dt>Account</dt><dd>{p.account}</dd>
      <dt>Server</dt><dd>{p.name} <span class="muted"><code>{p.url}</code></span></dd>
      <dt>Role</dt><dd>{p.role ?? 'unknown'}</dd>
      <dt>Login valid</dt><dd>{formatExpiry(p.expires_at)}</dd>
    </dl>
    <PlainHttpNotice profile={p} onchange={() => app.refreshProfiles()} />
    <p class="hint">
      The login token is kept in the system keyring, never in a file. Play signs the game in with it by itself.
    </p>
    <div class="actions">
      {#if p.role === 'admin'}
        <button type="button" onclick={() => app.openWebAdmin(p.id)} title="Opens the server's web admin in your browser, already signed in">Open web admin</button>
      {/if}
      <button class="danger" type="button" onclick={logout}>Log out</button>
    </div>
  </section>
{:else}
  <section class="card" style="max-width: 440px">
    {#if p.keyring_error}<p class="err">{p.keyring_error}</p>{/if}
    <PlainHttpNotice profile={p} onchange={() => app.refreshProfiles()} />
    {#if p.account}<p class="notice">Your saved login for {p.account} has expired. Log in again.</p>{/if}
    {#if infoError}<p class="err">{infoError}</p>{/if}
    <div class="row" style="margin-bottom: 10px">
      <button type="button" class:primary={mode === 'login'} onclick={() => (mode = 'login')}>Log in</button>
      <button
        type="button"
        class:primary={mode === 'register'}
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
        <div class="actions"><button class="primary" type="submit" disabled={busy || blocked || !username.trim() || !password}>Log in</button></div>
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
        <div class="actions"><button class="primary" type="submit" disabled={busy || blocked || !canRegister}>Create account</button></div>
      </form>
    {/if}
    <p class="hint">The password goes to the server you chose{p.plain_http === 'https' ? ' over https.' : p.plain_http === 'loopback' ? ' on this computer.' : ' (use https outside your own network).'}</p>
  </section>
{/if}
