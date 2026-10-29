<script lang="ts">
  import { app } from '../app.svelte';
  import { ipc } from '../ipc';
  import { run, toasts } from '../notify.svelte';
  import { formatExpiry } from '../format';
  import { isPlainHttpBlocked } from '../url';
  import Icon from './Icon.svelte';
  import Spinner from './Spinner.svelte';
  import type { ProfileView } from '../types';

  /** Log in or log out on one server (the account part of a server's row). Accounts are made on
   * the server's own register page, which "Create an account" opens in the browser. */
  let { profile }: { profile: ProfileView } = $props();

  let open = $state(false);
  let username = $state('');
  let password = $state('');
  let busy = $state(false);

  const p = $derived(profile);
  const info = $derived.by(() => {
    const i = app.infos[p.id];
    return typeof i === 'object' ? i : null;
  });
  const blocked = $derived(isPlainHttpBlocked(p));
  const registrationOpen = $derived(info?.open_registration ?? false);

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

  async function logout() {
    const r = await run(() => ipc.logout(p.id));
    if (r) toasts.show(r.revoked ? 'ok' : 'error', r.message);
    await app.refreshProfiles();
  }

  function registerPage() {
    void run(() => ipc.registerPageOpen(p.id), 'Opening the server\'s register page in your browser...');
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
        {open ? 'Close' : 'Log in'}
      </button>
    </div>
    {#if p.keyring_error}<p class="err">{p.keyring_error}</p>{/if}
    {#if open}
      <div class="form">
        {#if blocked}<p class="err">Allow plain http for this server first (above).</p>{/if}
        <form onsubmit={login}>
          <label class="field"><span>Account name</span><input bind:value={username} autocomplete="username" /></label>
          <label class="field"><span>Password</span><input type="password" bind:value={password} autocomplete="current-password" /></label>
          <div class="actions">
            {#if registrationOpen}
              <button type="button" class="link" onclick={registerPage}>No account yet? Create one <Icon name="external" size={12} /></button>
              <span class="grow"></span>
            {/if}
            <button class="btn primary" type="submit" disabled={busy || blocked || !username.trim() || !password}>
              {#if busy}<Spinner size={14} />{/if} Log in
            </button>
          </div>
        </form>
        {#if info && !registrationOpen}<p class="hint">Registration is closed on this server: an admin creates accounts in its web admin.</p>{/if}
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
  .actions { align-items: center; }
</style>
