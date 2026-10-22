<script lang="ts">
  import { onMount } from 'svelte';
  import { app } from './lib/app.svelte';
  import { router } from './lib/router.svelte';
  import { frame } from './lib/frame';
  import TitleBar from './lib/ui/TitleBar.svelte';
  import ResizeHandles from './lib/ui/ResizeHandles.svelte';
  import Icon, { type IconName } from './lib/ui/Icon.svelte';
  import Dropdown, { type DropItem } from './lib/ui/Dropdown.svelte';
  import Spinner from './lib/ui/Spinner.svelte';
  import Chip from './lib/ui/Chip.svelte';
  import Toasts from './lib/ui/Toasts.svelte';
  import ConfirmDialog from './lib/ui/ConfirmDialog.svelte';
  import Servers from './pages/Servers.svelte';
  import Account from './pages/Account.svelte';
  import Install from './pages/Install.svelte';
  import Mods from './pages/Mods.svelte';
  import Host from './pages/Host.svelte';
  import Settings from './pages/Settings.svelte';
  import About from './pages/About.svelte';

  interface NavItem { page: string; label: string; icon: IconName; official: boolean }
  const nav: NavItem[] = [
    { page: 'servers', label: 'Servers', icon: 'servers', official: false },
    { page: 'account', label: 'Account', icon: 'account', official: false },
    { page: 'install', label: 'Install', icon: 'install', official: true },
    { page: 'host', label: 'Host a server', icon: 'host', official: false },
    { page: 'mods', label: 'Mods', icon: 'mods', official: false },
    { page: 'settings', label: 'Settings', icon: 'settings', official: true },
    { page: 'about', label: 'About', icon: 'info', official: true },
  ];
  const visible = $derived(nav.filter((n) => !app.official || n.official));
  const page = $derived(visible.some((n) => n.page === router.page) ? router.page : (visible[0]?.page ?? 'servers'));
  // The official mode has no servers or mods: it opens on the install page.
  const landing = $derived(app.official ? 'install' : 'servers');

  let maximized = $state(false);
  let fullscreen = $state(false);
  const custom = $derived(app.settings?.custom_frame ?? true);
  const native = frame.available();

  async function syncWindow() {
    maximized = await frame.isMaximized();
    fullscreen = await frame.isFullscreen();
  }

  onMount(() => {
    void app.init();
    void syncWindow();
    let off = () => {};
    void frame.onResized(() => void syncWindow()).then((u) => (off = u));
    return () => off();
  });

  // The chosen frame is applied to the window itself (the backend does the same at start).
  let appliedFrame: boolean | null = null;
  $effect(() => {
    if (!app.settings) return;
    const want = app.settings.custom_frame;
    if (appliedFrame !== null && appliedFrame !== want) frame.setDecorations(!want);
    appliedFrame = want;
  });

  $effect(() => {
    if (app.ready && router.page !== page) router.go(landing);
  });

  const blocker = $derived(app.playBlocker);
  const busy = $derived(app.play.state === 'preparing');
  const running = $derived(app.play.state === 'running');
  const stateText = $derived.by(() => {
    const p = app.play;
    if (p.state === 'preparing') return `Preparing: ${p.step}...`;
    if (p.state === 'running') return 'Game running';
    if (p.state === 'exited') return p.code === 0 || p.code === null ? 'Game closed' : `Game exited with code ${p.code}`;
    return blocker ?? (app.official ? 'Ready: official servers, the hook only watches' : 'Ready to play');
  });
  const dot = $derived(busy ? 'busy' : running ? 'running' : blocker ? 'blocked' : 'ready');

  const serverItems = $derived<DropItem[]>(
    app.profiles.map((p) => ({
      value: p.id,
      label: p.name,
      sub: p.logged_in ? `signed in as ${p.account}` : 'not signed in',
      icon: 'servers',
      badges: p.logged_in ? [] : [{ text: 'log in', tone: 'warn' as const }],
    })),
  );
</script>

<div
  class="window"
  class:max={maximized}
  class:fullscreen
  class:preview={!native}
  data-mode={app.official ? 'official' : 'private'}
>
  {#if custom && native && !maximized && !fullscreen}<ResizeHandles />{/if}
  <TitleBar {custom} {maximized} />
  <div class="body">
    <nav class="side" aria-label="Main">
      {#each visible as n (n.page)}
        {#if n.page === 'about'}<span class="spacer"></span>{/if}
        <a href={'#/' + n.page} aria-current={page === n.page ? 'page' : undefined}>
          <Icon name={n.icon} size={18} />
          <span class="label">{n.label}</span>
        </a>
      {/each}
      {#if app.official}
        <div class="side-card official">
          <strong><Icon name="eye" size={14} /> Official mode</strong>
          Read-only observation. No servers, hosting or mods here.
        </div>
      {:else if app.selected?.logged_in}
        <div class="side-card">
          <strong><Icon name="account" size={14} /> {app.selected.account}</strong>
          <span class="muted">on {app.selected.name}</span>
        </div>
      {/if}
    </nav>
    <main id="main" tabindex="-1">
      {#if !app.ready}
        <div class="loading"><Spinner size={20} /> Loading...</div>
      {:else}
        {#key page + app.version}
          <div class="page">
            {#if page === 'servers'}
              <Servers />
            {:else if page === 'account'}
              <Account />
            {:else if page === 'install'}
              <Install />
            {:else if page === 'host'}
              <Host />
            {:else if page === 'mods'}
              <Mods />
            {:else if page === 'about'}
              <About />
            {:else}
              <Settings />
            {/if}
          </div>
        {/key}
      {/if}
    </main>
  </div>
  <footer class="playbar">
    {#if app.official}
      <Chip tone="warn" icon="eye">Official servers (read-only)</Chip>
    {:else}
      <div class="server-pick">
        <Dropdown
          items={serverItems}
          value={app.selected?.id ?? ''}
          label="Server"
          placement="up"
          disabled={app.profiles.length === 0}
          onselect={(id) => void app.select(id)}
        >
          {#snippet trigger(cur)}
            <Icon name="servers" size={16} />
            <span class="dd-label">{cur?.label ?? 'No server yet'}</span>
          {/snippet}
        </Dropdown>
      </div>
    {/if}
    <div class="state grow" class:muted={!!blocker && !busy && !running}>
      <span class="state-dot {dot}"></span>
      <span>{stateText}</span>
    </div>
    <button
      type="button"
      class="play-btn"
      class:official={app.official}
      class:ready={!blocker}
      class:busy
      disabled={blocker !== null}
      title={blocker ?? (app.official ? 'Deploy the read-only hook and start the game' : 'Deploy the hook and start the game')}
      onclick={() => void app.startPlay()}
    >
      {#if busy}<Spinner size={18} label="Starting" /> Starting{:else if running}<Icon name="check" size={18} /> Running{:else}<Icon name="play" size={18} filled /> Play{/if}
    </button>
  </footer>
</div>
<Toasts />
<ConfirmDialog />
