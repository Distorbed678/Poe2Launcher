<script lang="ts">
  import { open } from '@tauri-apps/plugin-dialog';
  import { app } from '../lib/app.svelte';
  import { errorText, ipc } from '../lib/ipc';
  import { run, toasts } from '../lib/notify.svelte';
  import { blocksEnable, compatLabel, dependentsOf, isValid, sortMods, stepTarget } from '../lib/mods';
  import PageHead from '../lib/ui/PageHead.svelte';
  import type { InstalledMod } from '../lib/types';

  let mods = $state<InstalledMod[]>([]);
  let dir = $state('');

  const sorted = $derived(sortMods(mods));

  async function load() {
    mods = (await run(() => ipc.mods())) ?? mods;
    dir = (await run(() => ipc.modsDir())) ?? dir;
  }
  $effect(() => {
    void app.status?.install_dir;
    void load();
  });

  async function install(path: string | null) {
    if (!path) return;
    let r;
    try {
      r = await ipc.modInstall(path, false);
    } catch (e) {
      const msg = errorText(e);
      if (msg.includes('already installed') && confirm(`${msg}\n\nReplace it?`)) {
        r = await run(() => ipc.modInstall(path, true));
      } else {
        toasts.show('error', msg);
      }
    }
    if (r) {
      toasts.show('ok', `Installed ${r.id}`);
      if (r.unknown_keys.length) toasts.show('ok', `Ignored unknown manifest keys: ${r.unknown_keys.join(', ')}`);
    }
    await load();
  }

  async function pickFolder() {
    const p = await open({ directory: true, title: 'Mod folder (with mod.toml)' });
    await install(typeof p === 'string' ? p : null);
  }

  async function pickZip() {
    const p = await open({ filters: [{ name: 'Mod archive', extensions: ['zip'] }], title: 'Mod archive' });
    await install(typeof p === 'string' ? p : null);
  }

  async function toggle(m: InstalledMod) {
    if (m.enabled) {
      const deps = dependentsOf(mods, m.id);
      if (deps.length && !confirm(`Disabling ${m.id} also disables: ${deps.join(', ')}. Continue?`)) return;
      const r = await run(() => ipc.modDisable(m.id));
      if (r) toasts.show('ok', `Disabled ${r.join(', ')}`);
    } else {
      let override = false;
      if (blocksEnable(m.compat)) {
        if (!confirm(`${m.id} is ${compatLabel(m.compat).text}. Enable it anyway?`)) return;
        override = true;
      }
      const r = await run(() => ipc.modEnable(m.id, override));
      if (r) toasts.show('ok', `Enabled ${r.join(', ')}`);
    }
    await load();
  }

  async function step(m: InstalledMod, delta: -1 | 1) {
    const to = stepTarget(mods, m.id, delta);
    if (to === null) return;
    await run(() => ipc.modMove(m.id, to));
    await load();
  }

  async function remove(m: InstalledMod) {
    if (!confirm(`Remove ${m.id} from the mods folder?`)) return;
    await run(() => ipc.modRemove(m.id), 'Removed');
    await load();
  }
</script>

<PageHead title="Mods" sub="Mod files live in the install's poe-hook-mods folder, never in the game files.">
  <button type="button" onclick={pickFolder}>Add folder</button>
  <button type="button" onclick={pickZip}>Add zip</button>
  <button type="button" onclick={async () => { await run(() => ipc.modAutoOrder(), 'Load order resolved'); await load(); }}>Auto order</button>
</PageHead>

<p class="notice">
  Heads up: the game does not load mods yet. The launcher only manages the files and the load order
  (<code>state.toml</code>); the hook's mod loader reads them once it exists.
</p>
<p class="hint">Folder: <code>{dir}</code></p>

{#if sorted.length === 0}
  <div class="card empty muted">No mods installed. Add a folder or zip that contains a <code>mod.toml</code>.</div>
{:else}
  <ul class="modlist">
    {#each sorted as m (m.dir_name)}
      {@const lab = compatLabel(m.compat)}
      <li>
        <input type="checkbox" checked={m.enabled} disabled={!isValid(m)} aria-label="Enable {m.id}" onchange={() => toggle(m)} />
        <div>
          <strong>{m.manifest?.name ?? m.dir_name}</strong>
          <span class="muted">{m.manifest ? ` ${m.manifest.version}` : ''}</span>
          {#if m.enabled}<span class="pill ok">#{(m.order ?? 0) + 1}</span>{/if}
          {#if m.manifest}<span class="pill {lab.tone}">{lab.text}</span>{/if}
          <div class="meta">
            <code>{m.id}</code>
            {#if m.manifest?.description} - {m.manifest.description}{/if}
            {#if m.manifest && Object.keys(m.manifest.dependencies).length}
              - needs {Object.entries(m.manifest.dependencies).map(([k, v]) => `${k} ${v}`).join(', ')}
            {/if}
          </div>
          {#if m.errors.length}<ul class="errs">{#each m.errors as e}<li>{e}</li>{/each}</ul>{/if}
        </div>
        <div class="row">
          {#if m.enabled}
            <button type="button" class="icon" title="Earlier" disabled={stepTarget(mods, m.id, -1) === null} onclick={() => step(m, -1)}>Up</button>
            <button type="button" class="icon" title="Later" disabled={stepTarget(mods, m.id, 1) === null} onclick={() => step(m, 1)}>Down</button>
          {/if}
          <button type="button" class="danger" onclick={() => remove(m)}>Remove</button>
        </div>
      </li>
    {/each}
  </ul>
{/if}
