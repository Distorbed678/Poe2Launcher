<script lang="ts">
  import { open } from '@tauri-apps/plugin-dialog';
  import { app } from '../lib/app.svelte';
  import { ipc } from '../lib/ipc';
  import { run } from '../lib/notify.svelte';
  import PageHead from '../lib/ui/PageHead.svelte';
  import type { Settings } from '../lib/types';

  let draft = $state<Settings | null>(app.settings ? { ...app.settings } : null);
  const dirty = $derived(JSON.stringify(draft) !== JSON.stringify(app.settings));
  const linux = $derived(app.info?.os === 'linux');

  async function pick(key: 'install_dir' | 'dll_dir' | 'steam_root') {
    const p = await open({ directory: true });
    if (typeof p === 'string' && draft) draft[key] = p;
  }

  async function save() {
    if (!draft) return;
    const s = await run(() => ipc.setSettings({ ...draft! }), 'Settings saved');
    if (s) {
      app.settings = s;
      draft = { ...s };
      document.documentElement.dataset.theme = s.theme;
      await app.refreshStatus();
    }
  }
</script>

<PageHead title="Settings">
  <button class="primary" type="button" disabled={!dirty} onclick={save}>Save</button>
</PageHead>

{#if draft}
  <div class="two">
    <section class="card">
      <h2>Paths</h2>
      {#each [
        { key: 'install_dir', label: 'Game install directory', hint: `Empty: ${app.info?.data_dir ?? ''}/game/${app.info?.client_version ?? ''}. Never a Steam library folder.` },
        { key: 'dll_dir', label: 'Hook DLL directory', hint: 'Build output of Poe2ClientHook holding poe_hook.dll and poe_hook_core.dll, e.g. modules/client-hook/target/x86_64-pc-windows-msvc/release.' },
      ] as f (f.key)}
        <div class="field">
          <span>{f.label}</span>
          <div class="row">
            <input class="grow" bind:value={draft[f.key as 'install_dir' | 'dll_dir']} placeholder="(default)" />
            <button type="button" onclick={() => pick(f.key as 'install_dir' | 'dll_dir')}>Browse</button>
          </div>
          <span class="hint">{f.hint}</span>
        </div>
      {/each}
      {#if linux}
        <div class="field">
          <span>Steam root (only to find Proton, read-only)</span>
          <div class="row">
            <input class="grow" bind:value={draft.steam_root} placeholder="~/.local/share/Steam" />
            <button type="button" onclick={() => pick('steam_root')}>Browse</button>
          </div>
        </div>
        <label class="field">
          <span>Proton (folder name under steamapps/common)</span>
          <input bind:value={draft.proton} />
          <span class="hint">The prefix is the launcher's own: {app.info?.data_dir}/proton-prefix. Steam must be running.</span>
        </label>
      {/if}
    </section>
    <section class="card">
      <h2>Steam download</h2>
      <label class="check"><input type="checkbox" bind:checked={draft.prefer_code_entry} /> Ask for a typed Steam Guard code instead of a confirmation in the Steam mobile app</label>
      <label class="check" style="margin-top: 8px"><input type="checkbox" bind:checked={draft.remember_steam_login} /> Remember the Steam login (can also be set on the Install page)</label>
      <h2 style="margin-top: 16px">Appearance</h2>
      <label class="field">
        <span>Theme</span>
        <select bind:value={draft.theme}><option value="dark">Dark</option><option value="light">Light</option></select>
      </label>
      <h2 style="margin-top: 16px">About</h2>
      <dl class="kv">
        <dt>Launcher</dt><dd>{app.info?.launcher_version}</dd>
        <dt>Client</dt><dd>{app.info?.client_version}</dd>
        <dt>Config</dt><dd><code>{app.info?.config_dir}</code></dd>
        <dt>Data</dt><dd><code>{app.info?.data_dir}</code></dd>
      </dl>
    </section>
  </div>
{/if}
