<script lang="ts">
  import { onMount } from 'svelte';
  import { open } from '@tauri-apps/plugin-dialog';
  import { app } from '../lib/app.svelte';
  import { ipc } from '../lib/ipc';
  import { run } from '../lib/notify.svelte';
  import { ask } from '../lib/confirm.svelte';
  import { versionLabel } from '../lib/gameversion';
  import { router } from '../lib/router.svelte';
  import PageHead from '../lib/ui/PageHead.svelte';
  import UpdatePanel from '../lib/ui/UpdatePanel.svelte';
  import Tabs from '../lib/ui/Tabs.svelte';
  import Icon from '../lib/ui/Icon.svelte';
  import Chip from '../lib/ui/Chip.svelte';
  import Switch from '../lib/ui/Switch.svelte';
  import Spinner from '../lib/ui/Spinner.svelte';
  import About from './About.svelte';
  import { formatBytes } from '../lib/format';
  import type { CacheStatus, DesktopEntry, Settings, SteamLogin, SyncModeName } from '../lib/types';

  // One tab per topic, one spaced column each. Every change saves on its own: switches and
  // choices at once, text fields when you leave them.
  type TabId = 'general' | 'game' | 'steam' | 'mods' | 'about' | 'advanced';
  const linux = $derived(app.info?.os === 'linux');
  const tabs = $derived([
    { id: 'general', label: 'General' },
    { id: 'game', label: linux ? 'Game & Proton' : 'Game' },
    { id: 'steam', label: 'Steam' },
    { id: 'mods', label: 'Mods' },
    { id: 'about', label: 'Updates & About' },
    { id: 'advanced', label: 'Advanced' },
  ]);
  const tab = $derived.by<TabId>(() => {
    if (router.route.segments[0] === 'about') return 'about';
    const t = router.route.query.get('tab');
    return (['general', 'game', 'steam', 'mods', 'about', 'advanced'] as const).find((x) => x === t) ?? 'general';
  });

  /** The settings as the backend stores them: empty install folders are dropped. */
  function clean(s: Settings): Settings {
    const dirs = Object.fromEntries(Object.entries(s.install_dirs).filter(([, v]) => v && v.trim() !== ''));
    return { ...s, install_dirs: dirs };
  }

  // Text fields edit a copy; it is saved when the field is left (and only if it changed).
  let draft = $state<Settings | null>(app.settings ? { ...app.settings, install_dirs: { ...app.settings.install_dirs } } : null);
  const ver = $derived(app.version);
  const verName = $derived(versionLabel(ver));

  async function commit(patch: Partial<Settings>) {
    if (!app.settings) return;
    const next = clean({ ...app.settings, ...patch, install_dirs: { ...(patch.install_dirs ?? app.settings.install_dirs) } });
    if (JSON.stringify(next) === JSON.stringify(clean(app.settings))) return;
    const s = await app.saveSettings(next, 'Saved');
    if (s) {
      draft = { ...s, install_dirs: { ...s.install_dirs } };
      if ('install_dirs' in patch || 'dll_dir' in patch) await Promise.all([app.refreshStatus(), app.refreshInfo()]);
    } else if (app.settings) {
      // Refused (e.g. a Steam folder): back to what is saved.
      draft = { ...app.settings, install_dirs: { ...app.settings.install_dirs } };
    }
  }

  let cache = $state<CacheStatus | null>(null);
  let cacheBusy = $state(false);
  const syncModes: { id: SyncModeName; label: string; help: string }[] = [
    { id: 'off', label: 'Off', help: 'Only show which mods a server requires. Play is blocked until you have them.' },
    { id: 'ask', label: 'Ask', help: 'A dialog lists the required mods (name, version, side, size, code) with a Download button.' },
    { id: 'auto', label: 'Auto', help: 'Download, verify and switch to the server\'s mods without asking - code mods included, also from a server you just added.' },
  ];
  async function clearCache() {
    cacheBusy = true;
    const r = await run(() => ipc.modCachePrune(), (p) => (p.removed ? `Removed ${p.removed} unused cached mods (${formatBytes(p.bytes)})` : 'Nothing unused to remove'));
    cacheBusy = false;
    if (r) cache = (await run(() => ipc.modCacheStatus())) ?? cache;
  }

  let steam = $state<SteamLogin | null>(null);
  let steamBusy = $state(false);
  let desktop = $state<DesktopEntry | null>(null);
  let desktopBusy = $state(false);

  onMount(() => {
    void run(() => ipc.steamLogin()).then((s) => (steam = s ?? null));
    void run(() => ipc.modCacheStatus()).then((c) => (cache = c ?? null));
    if (linux) void run(() => ipc.desktopEntry()).then((d) => (desktop = d ?? null));
  });

  async function pick(key: 'dll_dir' | 'steam_root') {
    const p = await open({ directory: true });
    if (typeof p === 'string') await commit({ [key]: p });
  }

  async function pickInstall() {
    const p = await open({ directory: true });
    if (typeof p === 'string' && app.settings) await commit({ install_dirs: { ...app.settings.install_dirs, [ver]: p } });
  }

  async function steamLogout() {
    const ok = await ask({
      title: 'Log out of Steam?',
      message:
        'The launcher forgets the Steam login that DepotDownloader remembers and the saved account name. The next download asks for your Steam password (and a Steam Guard code) again.\n\nYour Steam client and its own login are not touched.',
      confirmLabel: 'Log out of Steam',
      danger: true,
    });
    if (!ok) return;
    steamBusy = true;
    const s = await run(() => ipc.steamLogout(), 'Logged out of Steam');
    steamBusy = false;
    if (s) {
      steam = s;
      await app.refreshSettings();
      if (app.settings) draft = { ...app.settings, install_dirs: { ...app.settings.install_dirs } };
    }
  }

  async function desktopAction(kind: 'install' | 'uninstall') {
    desktopBusy = true;
    const d = await run(
      () => (kind === 'install' ? ipc.desktopEntryInstall() : ipc.desktopEntryUninstall()),
      kind === 'install' ? 'Desktop entry installed' : 'Desktop entry removed',
    );
    desktopBusy = false;
    if (d) desktop = d;
  }
</script>

<PageHead icon="settings" title="Settings" />

<Tabs {tabs} value={tab} label="Settings" onchange={(id) => router.go('settings', { tab: id === 'general' ? undefined : id })} />

{#if draft && app.settings}
  <div class="col">
    {#if tab === 'general'}
      <section class="card">
        <div class="card-head"><span class="head-icon"><Icon name="sun" size={16} /></span><h2>Appearance</h2></div>
        <div class="field">
          <span>Theme</span>
          <div class="pills" role="radiogroup" aria-label="Theme">
            <button type="button" class:on={app.settings.theme === 'dark'} role="radio" aria-checked={app.settings.theme === 'dark'} onclick={() => commit({ theme: 'dark' })}><Icon name="moon" size={14} /> Dark</button>
            <button type="button" class:on={app.settings.theme === 'light'} role="radio" aria-checked={app.settings.theme === 'light'} onclick={() => commit({ theme: 'light' })}><Icon name="sun" size={14} /> Light</button>
          </div>
        </div>
        <div class="toggle">
          <Switch checked={app.settings.custom_frame} onchange={(v) => commit({ custom_frame: v })}>
            <b>Use the launcher's own window frame</b><small>Turn off for your system's title bar and borders.</small>
          </Switch>
        </div>
      </section>
      <section class="card">
        <div class="card-head"><span class="head-icon"><Icon name="play" size={16} /></span><h2>While the game runs</h2></div>
        <div class="toggle first">
          <Switch checked={app.settings.minimize_while_playing} onchange={(v) => commit({ minimize_while_playing: v })}>
            <b>Minimize the launcher while the game is running</b>
            <small>The window minimizes when the game starts and comes back when the game closes.</small>
          </Switch>
        </div>
      </section>
    {/if}

    {#if tab === 'game'}
      <section class="card">
        <div class="card-head"><span class="head-icon"><Icon name="folder" size={16} /></span><h2>Game folder</h2></div>
        <div class="field">
          <span>Game install directory for {verName}</span>
          <div class="row">
            <input class="grow" bind:value={draft.install_dirs[ver]} placeholder="(default)" onchange={() => draft && commit({ install_dirs: { ...draft.install_dirs } })} />
            <button type="button" class="btn" onclick={pickInstall}>Browse</button>
          </div>
          <span class="hint">Empty: <code>{app.info?.data_dir ?? ''}/game/{ver}</code>. Each game version has its own folder; never a Steam library folder.</span>
        </div>
      </section>
      {#if linux}
        <section class="card">
          <div class="card-head"><span class="head-icon"><Icon name="portal" size={16} /></span><h2>Proton</h2></div>
          <div class="field">
            <span>Steam root (only to find Proton, read-only)</span>
            <div class="row">
              <input class="grow" bind:value={draft.steam_root} placeholder="~/.local/share/Steam" onchange={() => draft && commit({ steam_root: draft.steam_root || null })} />
              <button type="button" class="btn" onclick={() => pick('steam_root')}>Browse</button>
            </div>
          </div>
          <label class="field">
            <span>Proton (folder name under steamapps/common)</span>
            <input bind:value={draft.proton} onchange={() => draft && commit({ proton: draft.proton })} />
            <span class="hint">The prefixes are the launcher's own: <code>{app.info?.data_dir}/proton-prefix</code> (private servers) and <code>proton-prefix-official</code>. Steam must be running.</span>
          </label>
        </section>
      {/if}
    {/if}

    {#if tab === 'steam'}
      <section class="card">
        <div class="card-head"><span class="head-icon"><Icon name="key" size={16} /></span><h2>Steam account</h2>
          <span class="grow"></span>
          {#if steam === null}<Spinner size={14} />
          {:else if steam.remembered}<Chip tone="ok" dot>{steam.account ? `Logged in as ${steam.account}` : 'Login remembered'}</Chip>
          {:else}<Chip>Not logged in</Chip>{/if}
        </div>
        <p class="hint">Only used to download the game. With <b>Remember the Steam login</b> on, DepotDownloader keeps a login token (never your password) in the launcher's own data folder. Logging out deletes it and forgets the account name; your Steam client is not touched.</p>
        <div class="toggle">
          <Switch checked={app.settings.remember_steam_login} onchange={(v) => commit({ remember_steam_login: v })}>
            <b>Remember the Steam login</b><small>Also available in the Library.</small>
          </Switch>
          <Switch checked={app.settings.prefer_code_entry} onchange={(v) => commit({ prefer_code_entry: v })}>
            <b>Ask for a typed Steam Guard code</b><small>Instead of a confirmation in the Steam mobile app.</small>
          </Switch>
        </div>
        <div class="actions start">
          <button type="button" class="btn danger" disabled={steamBusy || !(steam?.remembered || app.settings.steam_username)} onclick={steamLogout}>
            {#if steamBusy}<Spinner size={14} />{:else}<Icon name="logout" size={14} />{/if} Log out of Steam
          </button>
        </div>
      </section>
    {/if}

    {#if tab === 'mods'}
      <section class="card">
        <div class="card-head"><span class="head-icon"><Icon name="mods" size={16} /></span><h2>Mod sync</h2></div>
        <div class="pills" role="radiogroup" aria-label="Mod sync">
          {#each syncModes as m (m.id)}
            <button type="button" class:on={app.settings.mod_sync === m.id} role="radio" aria-checked={app.settings.mod_sync === m.id} onclick={() => commit({ mod_sync: m.id })}>{m.label}</button>
          {/each}
        </div>
        <p class="hint">{syncModes.find((m) => m.id === app.settings!.mod_sync)?.help} What a server requires is checked when you select it and again at Play. Every download is checked against the SHA-256 the server announced; a mismatch installs nothing.</p>
      </section>
      <section class="card">
        <div class="card-head"><span class="head-icon"><Icon name="database" size={16} /></span><h2>Mod cache</h2></div>
        <dl class="kv">
          <dt>Cached</dt><dd>{#if cache}{cache.entries} mod versions, {formatBytes(cache.bytes)}{:else}...{/if}</dd>
          {#if cache}<dt>Folder</dt><dd><code>{cache.dir}</code></dd>{/if}
        </dl>
        <div class="actions start">
          <button type="button" class="btn" disabled={cacheBusy || !cache || cache.unused_entries === 0} onclick={clearCache}>
            {#if cacheBusy}<Spinner size={14} />{:else}<Icon name="trash" size={14} />{/if} Clear unused cached mods{cache && cache.unused_entries ? ` (${cache.unused_entries}, ${formatBytes(cache.unused_bytes)})` : ''}
          </button>
        </div>
        <p class="hint">Unused: not enabled for any server and not required by one. A server's own mods come back by themselves when needed.</p>
      </section>
    {/if}

    {#if tab === 'about'}
      <section class="card">
        <div class="card-head"><span class="head-icon"><Icon name="download" size={16} /></span><h2>Launcher updates</h2></div>
        <UpdatePanel />
      </section>
      <About embedded />
    {/if}

    {#if tab === 'advanced'}
      <section class="card">
        <div class="card-head"><span class="head-icon"><Icon name="box" size={16} /></span><h2>Hook DLL directory</h2></div>
        <div class="field">
          <span>Your own build of the client hook (override)</span>
          <div class="row">
            <input class="grow" bind:value={draft.dll_dir} placeholder="(default: the downloaded hook)" onchange={() => draft && commit({ dll_dir: draft.dll_dir || null })} />
            <button type="button" class="btn" onclick={() => pick('dll_dir')}>Browse</button>
          </div>
          <span class="hint">
            Leave empty: the launcher downloads the client hook itself (Library) and uses that. Set this only to use your own build of Poe2ClientHook
            (a folder with poe_hook.dll and poe_hook_core.dll); it then wins over the downloaded hook.
            {#if app.hook}Now using: <b>{app.hook.source.kind === 'downloaded' ? `downloaded ${app.hook.source.version}` : app.hook.source.kind === 'manual' ? 'your directory' : app.hook.source.kind === 'legacy' ? 'files in the default folder' : 'nothing yet'}</b>.{/if}
          </span>
        </div>
      </section>
      {#if linux && desktop?.supported}
        <section class="card">
          <div class="card-head"><span class="head-icon"><Icon name="portal" size={16} /></span><h2>Desktop entry</h2>
            <span class="grow"></span>
            {#if desktop.installed && desktop.stale}<Chip tone="warn" icon="alert">points elsewhere</Chip>
            {:else if desktop.installed}<Chip tone="ok" icon="check">installed</Chip>
            {:else}<Chip>not installed</Chip>{/if}
          </div>
          <p class="hint">On Wayland the window icon comes from a <code>.desktop</code> file that matches the window's app id (<code>org.poe2modding.launcher</code>). An AppImage has none on your system: this writes one, with the icon, to <code>~/.local/share</code>.</p>
          <dl class="kv">
            <dt>Starts</dt><dd><code>{desktop.installed ? (desktop.exec ?? '?') : (desktop.current_exec ?? '?')}</code></dd>
            {#if desktop.file}<dt>File</dt><dd><code>{desktop.file}</code></dd>{/if}
          </dl>
          {#if desktop.stale}<p class="hint err">The entry starts another program than the one running now (the AppImage was moved?). Update it.</p>{/if}
          <div class="actions start">
            <button type="button" class="btn" disabled={desktopBusy} onclick={() => desktopAction('install')}>
              <Icon name="download" size={14} /> {desktop.installed ? 'Update desktop entry' : 'Install desktop entry'}
            </button>
            {#if desktop.installed}
              <button type="button" class="btn danger" disabled={desktopBusy} onclick={() => desktopAction('uninstall')}><Icon name="trash" size={14} /> Remove</button>
            {/if}
          </div>
        </section>
      {/if}
    {/if}
  </div>
{/if}

<style>
  .col { max-width: 760px; display: flex; flex-direction: column; gap: var(--sp-4); }
  .col > :global(.card) { margin: 0; }
  .toggle { display: flex; flex-direction: column; gap: var(--sp-4); margin-top: var(--sp-4); }
  .toggle.first { margin-top: 0; }
  .toggle :global(.switch-text b) { display: block; font-weight: 500; }
  .toggle :global(.switch-text small) { display: block; color: var(--faint); font-size: var(--fs-sm); margin-top: 2px; }
  .actions.start { justify-content: flex-start; margin-top: var(--sp-4); }
  .card .hint { margin-top: var(--sp-3); }
  .card .kv { margin-top: var(--sp-2); }
</style>
