<script lang="ts">
  import { onMount } from 'svelte';
  import { fade } from 'svelte/transition';
  import { app } from './lib/app.svelte';
  import { router } from './lib/router.svelte';
  import { frame } from './lib/frame';
  import { updater } from './lib/update.svelte';
  import { DUR, ease, ms } from './lib/motion';
  import { loadSeen, newestDate, saveSeen, unseenCount } from './lib/newsSeen';
  import TitleBar from './lib/ui/TitleBar.svelte';
  import ResizeHandles from './lib/ui/ResizeHandles.svelte';
  import Icon, { type IconName } from './lib/ui/Icon.svelte';
  import Spinner from './lib/ui/Spinner.svelte';
  import Toasts from './lib/ui/Toasts.svelte';
  import ConfirmDialog from './lib/ui/ConfirmDialog.svelte';
  import ModSyncDialog from './lib/ui/ModSyncDialog.svelte';
  import VersionPicker from './lib/ui/VersionPicker.svelte';
  import iconUrl from './assets/icon.png';
  import Home from './pages/Home.svelte';
  import Servers from './pages/Servers.svelte';
  import News from './pages/News.svelte';
  import Library from './pages/Library.svelte';
  import Settings from './pages/Settings.svelte';

  interface NavItem { page: string; label: string; icon: IconName; official: boolean }
  const nav: NavItem[] = [
    { page: 'home', label: 'Play', icon: 'play', official: true },
    { page: 'servers', label: 'Servers', icon: 'servers', official: false },
    { page: 'news', label: 'News', icon: 'news', official: true },
    { page: 'library', label: 'Library', icon: 'library', official: true },
    { page: 'settings', label: 'Settings', icon: 'settings', official: true },
  ];
  const visible = $derived(nav.filter((n) => !app.official || n.official));
  // About lives in Settings (Updates & About); the old #/about link still opens it.
  const page = $derived(visible.some((n) => n.page === router.page) ? router.page : 'home');
  const landing = 'home';

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

  // The News badge: items newer than the newest one seen on the News page. The very first start counts all as seen.
  let seen = $state(loadSeen());
  const newsDates = $derived((app.news?.items ?? []).map((n) => n.date));
  const unseen = $derived(page === 'news' ? 0 : unseenCount(newsDates, seen));
  $effect(() => {
    const newest = newestDate(newsDates);
    if (newest && (seen === null || page === 'news') && newest !== seen) {
      saveSeen(newest);
      seen = newest;
    }
  });
</script>

<div
  class="window"
  class:max={maximized}
  class:fullscreen
  class:preview={!native}
  data-mode={app.official ? 'official' : 'private'}
>
  {#if custom && native && !maximized && !fullscreen}<ResizeHandles />{/if}
  {#if custom && native}<TitleBar {maximized} />{/if}
  <div class="body">
    <nav class="side" aria-label="Main">
      <div class="side-brand" data-tauri-drag-region>
        <img src={iconUrl} alt="" width="30" height="30" />
        <span class="brand-text">
          <span class="brand-name">Path of Exile II</span>
          <small>{app.official ? 'official, read-only' : 'private launcher'}</small>
        </span>
      </div>
      <VersionPicker />
      {#each visible as n (n.page)}
        <a href={'#/' + n.page} aria-current={page === n.page ? 'page' : undefined}>
          <Icon name={n.icon} size={16} filled={n.icon === 'play' && page === n.page} />
          <span class="label">{n.label}</span>
          {#if n.page === 'news' && unseen > 0}<span class="nav-badge" aria-label="{unseen} new">{unseen}</span>{/if}
        </a>
      {/each}
      <span class="spacer"></span>
      {#if updater.state.kind === 'available'}
        <a class="side-update" href="#/settings"><Icon name="download" size={13} /> <span class="label">Update {updater.state.version}</span></a>
      {/if}
      {#if app.official}
        <div class="side-card official">
          <strong><Icon name="eye" size={13} /> Official mode</strong>
          <span class="muted">Read-only. No servers, hosting or mods.</span>
        </div>
      {/if}
      <div class="side-foot">
        <span class="ver label">Launcher{app.info?.launcher_version ? ` v${app.info.launcher_version}` : ''}</span>
      </div>
    </nav>
    <main id="main" tabindex="-1" class:play-main={page === 'home'}>
      {#if !app.ready}
        <div class="loading"><Spinner size={20} /> Loading...</div>
      {:else}
        {#key page + app.version}
          <div class="page" class:play-page={page === 'home'} in:fade={{ duration: ms(DUR.page), easing: ease }}>
            {#if page === 'home'}
              <Home />
            {:else if page === 'servers'}
              <Servers />
            {:else if page === 'news'}
              <News />
            {:else if page === 'library'}
              <Library />
            {:else}
              <Settings />
            {/if}
          </div>
        {/key}
      {/if}
    </main>
  </div>
</div>
<Toasts />
<ConfirmDialog />
<ModSyncDialog />
