<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { open } from '@tauri-apps/plugin-dialog';
  import { app } from '../lib/app.svelte';
  import { ipc } from '../lib/ipc';
  import { run } from '../lib/notify.svelte';
  import { ask } from '../lib/confirm.svelte';
  import { versionLabel } from '../lib/gameversion';
  import PageHead from '../lib/ui/PageHead.svelte';
  import UpdatePanel from '../lib/ui/UpdatePanel.svelte';
  import Icon from '../lib/ui/Icon.svelte';
  import Chip from '../lib/ui/Chip.svelte';
  import Switch from '../lib/ui/Switch.svelte';
  import Spinner from '../lib/ui/Spinner.svelte';
  import type { DesktopEntry, Settings, SteamLogin } from '../lib/types';

  /** The settings as the backend stores them: empty install folders are dropped. */
  function clean(s: Settings): Settings {
    const dirs = Object.fromEntries(Object.entries(s.install_dirs).filter(([, v]) => v && v.trim() !== ''));
    return { ...s, install_dirs: dirs };
  }

  let draft = $state<Settings | null>(app.settings ? { ...app.settings, install_dirs: { ...app.settings.install_dirs } } : null);
  const dirty = $derived(draft !== null && JSON.stringify(clean(draft)) !== JSON.stringify(clean(app.settings!)));
  const linux = $derived(app.info?.os === 'linux');
  const ver = $derived(app.version);
  const verName = $derived(versionLabel(ver));

  let steam = $state<SteamLogin | null>(null);
  let steamBusy = $state(false);
  let desktop = $state<DesktopEntry | null>(null);
  let desktopBusy = $state(false);

  // The theme previews as soon as it is picked; leaving without saving puts the saved one back.
  $effect(() => {
    if (draft) document.documentElement.dataset.theme = draft.theme;
  });
  onDestroy(() => {
    if (app.settings) document.documentElement.dataset.theme = app.settings.theme;
  });

  onMount(() => {
    void run(() => ipc.steamLogin()).then((s) => (steam = s ?? null));
    if (linux) void run(() => ipc.desktopEntry()).then((d) => (desktop = d ?? null));
  });

  async function pick(key: 'dll_dir' | 'steam_root') {
    const p = await open({ directory: true });
    if (typeof p === 'string' && draft) draft[key] = p;
  }

  async function pickInstall() {
    const p = await open({ directory: true });
    if (typeof p === 'string' && draft) draft.install_dirs[ver] = p;
  }

  async function save() {
    if (!draft) return;
    const s = await app.saveSettings(clean(draft), 'Settings saved');
    if (s) {
      draft = { ...s, install_dirs: { ...s.install_dirs } };
      await Promise.all([app.refreshStatus(), app.refreshInfo()]);
    }
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
      if (draft && app.settings) {
        draft.steam_username = app.settings.steam_username;
        draft.remember_steam_login = app.settings.remember_steam_login;
      }
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

<PageHead icon="settings" title="Settings">
  <button class="btn primary" type="button" disabled={!dirty} onclick={save}><Icon name="check" size={14} /> Save changes</button>
</PageHead>

{#if draft}
  <div class="two">
    <div class="stack">
      <section class="card">
        <div class="card-head"><span class="head-icon"><Icon name="folder" size={16} /></span><h2>Paths</h2></div>
        <div class="field">
          <span>Game install directory for {verName}</span>
          <div class="row">
            <input class="grow" bind:value={draft.install_dirs[ver]} placeholder="(default)" />
            <button type="button" class="btn" onclick={pickInstall}>Browse</button>
          </div>
          <span class="hint">Empty: <code>{app.info?.data_dir ?? ''}/game/{ver}</code>. Each game version has its own folder. Never a Steam library folder.</span>
        </div>
        <div class="field">
          <span>Hook DLL directory <span class="muted">(advanced override)</span></span>
          <div class="row">
            <input class="grow" bind:value={draft.dll_dir} placeholder="(default)" />
            <button type="button" class="btn" onclick={() => pick('dll_dir')}>Browse</button>
          </div>
          <span class="hint">
            Leave empty: the launcher downloads the client hook itself (Library, Game) and uses that. Set this only to use your own build of
            Poe2ClientHook (a folder holding poe_hook.dll and poe_hook_core.dll, e.g. modules/client-hook/target/x86_64-pc-windows-msvc/release);
            it then wins over the downloaded hook.
            {#if app.hook}Now using: <b>{app.hook.source.kind === 'downloaded' ? `downloaded ${app.hook.source.version}` : app.hook.source.kind === 'manual' ? 'your directory' : app.hook.source.kind === 'legacy' ? 'files in the default folder' : 'nothing yet'}</b>.{/if}
          </span>
        </div>
        {#if linux}
          <div class="field">
            <span>Steam root (only to find Proton, read-only)</span>
            <div class="row">
              <input class="grow" bind:value={draft.steam_root} placeholder="~/.local/share/Steam" />
              <button type="button" class="btn" onclick={() => pick('steam_root')}>Browse</button>
            </div>
          </div>
          <label class="field">
            <span>Proton (folder name under steamapps/common)</span>
            <input bind:value={draft.proton} />
            <span class="hint">The prefixes are the launcher's own: {app.info?.data_dir}/proton-prefix (private servers) and proton-prefix-official. Steam must be running.</span>
          </label>
        {/if}
      </section>

      <section class="card">
        <div class="card-head"><span class="head-icon"><Icon name="key" size={16} /></span><h2>Steam account</h2>
          <span class="grow"></span>
          {#if steam === null}<Spinner size={14} />
          {:else if steam.remembered}<Chip tone="ok" dot>{steam.account ? `Logged in as ${steam.account}` : 'Login remembered'}</Chip>
          {:else}<Chip>Not logged in</Chip>{/if}
        </div>
        <p class="hint">
          Only used to download the game. When <b>Remember the Steam login</b> is on, DepotDownloader keeps a login token (never your password) in the launcher's own data folder.
          Logging out deletes it and forgets the account name; your Steam client is not touched.
        </p>
        <div style="margin-top: 12px"></div>
        <Switch bind:checked={draft.remember_steam_login}>Remember the Steam login <small>Also available in the Library.</small></Switch>
        <div class="actions" style="justify-content: flex-start">
          <button type="button" class="btn danger" disabled={steamBusy || !(steam?.remembered || app.settings?.steam_username)} onclick={steamLogout}>
            {#if steamBusy}<Spinner size={14} />{:else}<Icon name="logout" size={14} />{/if} Log out of Steam
          </button>
        </div>
        <hr class="sep" />
        <Switch bind:checked={draft.prefer_code_entry}>Ask for a typed Steam Guard code <small>Instead of a confirmation in the Steam mobile app.</small></Switch>
      </section>

      {#if linux && desktop?.supported}
        <section class="card">
          <div class="card-head"><span class="head-icon"><Icon name="portal" size={16} /></span><h2>Desktop entry</h2>
            <span class="grow"></span>
            {#if desktop.installed && desktop.stale}<Chip tone="warn" icon="alert">points elsewhere</Chip>
            {:else if desktop.installed}<Chip tone="ok" icon="check">installed</Chip>
            {:else}<Chip>not installed</Chip>{/if}
          </div>
          <p class="hint">
            On Wayland the window icon comes from a <code>.desktop</code> file that matches the window's app id (<code>org.poe2modding.launcher</code>).
            An AppImage has none of its own on your system: this writes one, with the icon, to <code>~/.local/share</code> so the launcher shows its icon and name in the taskbar and the application menu.
          </p>
          <dl class="kv" style="margin-top: 10px">
            <dt>Starts</dt><dd><code>{desktop.installed ? (desktop.exec ?? '?') : (desktop.current_exec ?? '?')}</code></dd>
            {#if desktop.file}<dt>File</dt><dd><code>{desktop.file}</code></dd>{/if}
          </dl>
          {#if desktop.stale}
            <p class="hint err" style="margin-top: 8px">The entry starts another program than the one running now (the AppImage was moved?). Update it.</p>
          {/if}
          <div class="actions" style="justify-content: flex-start">
            <button type="button" class="btn" disabled={desktopBusy} onclick={() => desktopAction('install')}>
              <Icon name="download" size={14} /> {desktop.installed ? 'Update desktop entry' : 'Install desktop entry'}
            </button>
            {#if desktop.installed}
              <button type="button" class="btn danger" disabled={desktopBusy} onclick={() => desktopAction('uninstall')}><Icon name="trash" size={14} /> Remove</button>
            {/if}
          </div>
        </section>
      {/if}
    </div>

    <div class="stack">
      <section class="card">
        <div class="card-head"><span class="head-icon"><Icon name="sun" size={16} /></span><h2>Appearance</h2></div>
        <div class="field">
          <span>Theme</span>
          <div class="seg" style="margin-bottom: 0; align-self: flex-start" role="radiogroup" aria-label="Theme">
            <button type="button" class:on={draft.theme === 'dark'} role="radio" aria-checked={draft.theme === 'dark'} onclick={() => draft && (draft.theme = 'dark')}><Icon name="moon" size={14} /> Dark</button>
            <button type="button" class:on={draft.theme === 'light'} role="radio" aria-checked={draft.theme === 'light'} onclick={() => draft && (draft.theme = 'light')}><Icon name="sun" size={14} /> Light</button>
          </div>
        </div>
        <Switch bind:checked={draft.custom_frame}>Use the launcher's own window frame <small>Turn off for your system's title bar and borders.</small></Switch>
      </section>

      <section class="card">
        <div class="card-head"><span class="head-icon"><Icon name="play" size={16} /></span><h2>While the game runs</h2></div>
        <Switch bind:checked={draft.minimize_while_playing}>
          Minimize the launcher while the game is running
          <small>The launcher window minimizes when the game starts and is shown again when the game closes. Off by default.</small>
        </Switch>
      </section>

      <section class="card">
        <div class="card-head"><span class="head-icon"><Icon name="download" size={16} /></span><h2>Launcher updates</h2></div>
        <UpdatePanel />
      </section>
    </div>
  </div>
{/if}
