<script lang="ts">
  import { onMount } from 'svelte';
  import { app } from './lib/app.svelte';
  import { router } from './lib/router.svelte';
  import { ipc } from './lib/ipc';
  import { run } from './lib/notify.svelte';
  import { updater } from './lib/update.svelte';
  import Toasts from './lib/ui/Toasts.svelte';
  import Servers from './pages/Servers.svelte';
  import Account from './pages/Account.svelte';
  import Install from './pages/Install.svelte';
  import Mods from './pages/Mods.svelte';
  import Host from './pages/Host.svelte';
  import Settings from './pages/Settings.svelte';

  const nav = [
    { page: 'servers', label: 'Servers' },
    { page: 'account', label: 'Account' },
    { page: 'install', label: 'Install' },
    { page: 'host', label: 'Host a server' },
    { page: 'mods', label: 'Mods' },
    { page: 'settings', label: 'Settings' },
  ];
  const page = $derived(nav.some((n) => n.page === router.page) ? router.page : 'servers');

  onMount(() => {
    void app.init();
  });

  const blocker = $derived(app.playBlocker);
  const stateText = $derived.by(() => {
    const p = app.play;
    if (p.state === 'preparing') return `Preparing: ${p.step}...`;
    if (p.state === 'running') return 'Game running';
    if (p.state === 'exited') return p.code === 0 || p.code === null ? 'Game closed' : `Game exited with code ${p.code}`;
    return blocker ?? 'Ready';
  });

  async function play() {
    if (!app.selected) return;
    await run(() => ipc.play(app.selected!.id));
  }
</script>

<div class="shell">
  <header class="topbar">
    <span class="brand">Poe2 <b>Launcher</b></span>
    <span class="grow"></span>
    {#if updater.state.kind === 'available'}
      <a class="updatelink" href="#/settings">Update {updater.state.version} available</a>
    {/if}
    <span class="muted">{app.info ? `client ${app.info.client_version}` : ''}</span>
  </header>
  <nav class="side" aria-label="Main">
    {#each nav as n (n.page)}
      <a href={'#/' + n.page} aria-current={page === n.page ? 'page' : undefined}>{n.label}</a>
    {/each}
  </nav>
  <main id="main" tabindex="-1">
    {#if !app.ready}
      <p class="muted">Loading...</p>
    {:else if page === 'servers'}
      <Servers />
    {:else if page === 'account'}
      <Account />
    {:else if page === 'install'}
      <Install />
    {:else if page === 'host'}
      <Host />
    {:else if page === 'mods'}
      <Mods />
    {:else}
      <Settings />
    {/if}
  </main>
  <footer class="playbar">
    <label class="inline">
      <span>Server</span>
      <select
        value={app.selected?.id ?? ''}
        disabled={app.profiles.length === 0}
        onchange={(e) => app.select(e.currentTarget.value)}
      >
        {#each app.profiles as p (p.id)}
          <option value={p.id}>{p.name}</option>
        {/each}
      </select>
    </label>
    <span class="state grow" class:muted={!blocker}>{stateText}</span>
    <button
      type="button"
      class="primary play"
      disabled={blocker !== null}
      title={blocker ?? 'Deploy the hook and start the game'}
      onclick={play}
    >
      Play
    </button>
  </footer>
</div>
<Toasts />
