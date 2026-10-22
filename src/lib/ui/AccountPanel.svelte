<script lang="ts">
  import { app } from '../app.svelte';
  import { ipc } from '../ipc';
  import { run, toasts } from '../notify.svelte';
  import { formatExpiry } from '../format';
  import { isPlainHttpBlocked } from '../url';
  import Icon from './Icon.svelte';
  import Spinner from './Spinner.svelte';
  import type { ProfileView } from '../types';

  /** Log in, create an account or log out on one server (the account part of a server's card). */
  let { profile }: { profile: ProfileView } = $props();

  let open = $state(false);
  let mode = $state<'login' | 'register'>('login');
  let username = $state('');
  let password = $state('');
  let password2 = $state('');
  let busy = $state(false);

  const p = $derived(profile);
  const info = $derived.by(() => {
    const i = app.infos[p.id];
    return typeof i === 'object' ? i : null;
  });
  const blocked = $derived(isPlainHttpBlocked(p));
  const registrationClosed = $derived(info !== null && !info.open_registration);
  const canRegister = $derived(password.length >= 8 && password === password2 && username.trim().length >= 3);

  async function login(e: SubmitEvent) {
    e.preventDefault();
    busy = true;
    const r = await run(() => ipc.login(p.id, username, password), 'Logged in');
    busy = false;
    if (r) {
      password = '';
      open = false;
      await app.refreshProfiles();
    }
  }

  async function register(e: SubmitEvent) {
    e.preventDefault();
    busy = true;
    const created = await run(() => ipc.register(p.id, username, password), 'Account created');
    if (created) {
      // Straight into a login with the same credentials.
      const r = await run(() => ipc.login(p.id, username, password), 'Logged in');
      if (r) await app.refreshProfiles();
      password = '';
      password2 = '';
      mode = 'login';
      open = false;
    }
    busy = false;
  }

  async function logout() {
    const r = await run(() => ipc.logout(p.id));
    if (r) toasts.show(r.revoked ? 'ok' : 'error', r.message);
    await app.refreshProfiles();
  }
</script>

<div class="acct">
  {#if p.logged_in}
    <div class="row">
      <span class="who"><Icon name="account" size={15} /> Signed in as <b>{p.account}</b>
        <span class="muted">({p.role ?? 'unknown role'}, login valid {formatExpiry(p.expires_at)})</span></span>
      <span class="grow"></span>
      <button type="button" class="btn sm danger" onclick={logout}><Icon name="logout" size={13} /> Log out</button>
    </div>
    <p class="hint">The login token is kept in the system keyring, never in a file. Play signs the game in with it by itself.</p>
  {:else}
    <div class="row">
      <span class="who muted"><Icon name="account" size={15} />
        {#if p.account}Login expired for <b>{p.account}</b>{:else}Not logged in{/if}</span>
      <span class="grow"></span>
      <button type="button" class="btn sm primary" onclick={() => (open = !open)} aria-expanded={open}>
        {open ? 'Close' : 'Log in / Create account'}
      </button>
    </div>
    {#if p.keyring_error}<p class="err">{p.keyring_error}</p>{/if}
    {#if open}
      <div class="form">
        <div class="seg" role="tablist" aria-label="Login or register">
          <button type="button" class:on={mode === 'login'} role="tab" aria-selected={mode === 'login'} onclick={() => (mode = 'login')}>Log in</button>
          <button
            type="button"
            class:on={mode === 'register'}
            role="tab"
            aria-selected={mode === 'register'}
            disabled={registrationClosed}
            title={registrationClosed ? 'This server has closed registration' : ''}
            onclick={() => (mode = 'register')}
          >Create account</button>
        </div>
        {#if registrationClosed}<p class="hint">Registration is closed on this server: an admin creates accounts.</p>{/if}
        {#if blocked}<p class="err">Allow plain http for this server first (above).</p>{/if}
        {#if mode === 'login' || registrationClosed}
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
            <label class="field"><span>Account name (3-64 characters: letters, digits, _ . @ + -)</span><input bind:value={username} autocomplete="username" /></label>
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
        <p class="hint">The password goes to this server{p.plain_http === 'https' ? ' over https.' : p.plain_http === 'loopback' ? ' on this computer.' : ' (use https outside your own network).'}</p>
      </div>
    {/if}
  {/if}
</div>

<style>
  .acct { margin-top: 12px; padding-top: 12px; border-top: 1px solid var(--line-soft); }
  .who { display: inline-flex; align-items: center; gap: 8px; min-width: 0; flex-wrap: wrap; }
  .form { margin-top: 12px; max-width: 440px; }
  .acct .hint { margin: 6px 0 0; }
</style>
