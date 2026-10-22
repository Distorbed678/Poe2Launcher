<script lang="ts">
  import { open } from '@tauri-apps/plugin-dialog';
  import { app } from '../lib/app.svelte';
  import { errorText, ipc } from '../lib/ipc';
  import { run, toasts } from '../lib/notify.svelte';
  import { blocksEnable, compatLabel, dependentsOf, isValid, sortMods, stepTarget } from '../lib/mods';
  import { fly, fade } from 'svelte/transition';
  import { flip } from 'svelte/animate';
  import { cubicOut } from 'svelte/easing';
  import { ask } from '../lib/confirm.svelte';
  import { ms } from '../lib/motion';
  import PageHead from '../lib/ui/PageHead.svelte';
  import Icon from '../lib/ui/Icon.svelte';
  import Chip from '../lib/ui/Chip.svelte';
  import Empty from '../lib/ui/Empty.svelte';
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
      if (msg.includes('already installed') && (await ask({ title: 'Replace the mod?', message: msg, confirmLabel: 'Replace', danger: true }))) {
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
      if (deps.length && !(await ask({ title: `Disable ${m.id}?`, message: `This also disables: ${deps.join(', ')}.`, confirmLabel: 'Disable all' }))) return;
      const r = await run(() => ipc.modDisable(m.id));
      if (r) toasts.show('ok', `Disabled ${r.join(', ')}`);
    } else {
      let override = false;
      if (blocksEnable(m.compat)) {
        if (!(await ask({ title: `Enable ${m.id} anyway?`, message: `It is ${compatLabel(m.compat).text}.`, confirmLabel: 'Enable anyway', danger: true }))) return;
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
    if (!(await ask({ title: `Remove ${m.id}?`, message: 'The mod folder is deleted from the install\'s poe-hook-mods folder.', confirmLabel: 'Remove', danger: true }))) return;
    await run(() => ipc.modRemove(m.id), 'Removed');
    await load();
  }
</script>

<PageHead icon="mods" title="Mods" sub="Mod files live in the install's poe-hook-mods folder, never in the game files.">
  <button type="button" class="btn" onclick={pickFolder}><Icon name="folder" size={14} /> Add folder</button>
  <button type="button" class="btn" onclick={pickZip}><Icon name="box" size={14} /> Add zip</button>
  <button type="button" class="btn" onclick={async () => { await run(() => ipc.modAutoOrder(), 'Load order resolved'); await load(); }}><Icon name="refresh" size={14} /> Auto order</button>
</PageHead>

<div class="notice info">
  <span class="notice-title"><Icon name="info" size={16} /> The game does not load mods yet</span>
  <p>The launcher only manages the files and the load order (<code>state.toml</code>); the hook's mod loader reads them once it exists.</p>
</div>
<p class="hint" style="margin-bottom: 14px">Folder: <code>{dir}</code></p>

{#if sorted.length === 0}
  <Empty icon="mods" title="No mods installed">
    Add a folder or zip that contains a <code>mod.toml</code>.
    {#snippet action()}
      <div class="row" style="justify-content: center">
        <button type="button" class="btn" onclick={pickFolder}><Icon name="folder" size={14} /> Add folder</button>
        <button type="button" class="btn" onclick={pickZip}><Icon name="box" size={14} /> Add zip</button>
      </div>
    {/snippet}
  </Empty>
{:else}
  <ul class="modlist">
    {#each sorted as m (m.dir_name)}
      {@const lab = compatLabel(m.compat)}
      <li in:fly={{ y: 12, duration: ms(240), easing: cubicOut }} out:fade={{ duration: ms(140) }} animate:flip={{ duration: ms(220) }}>
        <input type="checkbox" checked={m.enabled} disabled={!isValid(m)} aria-label="Enable {m.id}" onchange={() => toggle(m)} />
        <div>
          <strong>{m.manifest?.name ?? m.dir_name}</strong>
          <span class="muted">{m.manifest ? ` ${m.manifest.version}` : ''}</span>
          {#if m.enabled}<Chip tone="ok">#{(m.order ?? 0) + 1}</Chip>{/if}
          {#if m.manifest}<Chip tone={lab.tone === 'ok' ? 'ok' : lab.tone === 'bad' ? 'bad' : 'warn'}>{lab.text}</Chip>{/if}
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
            <button type="button" class="btn icon sm ghost" title="Earlier" aria-label="Move earlier" disabled={stepTarget(mods, m.id, -1) === null} onclick={() => step(m, -1)}><Icon name="up" size={15} /></button>
            <button type="button" class="btn icon sm ghost" title="Later" aria-label="Move later" disabled={stepTarget(mods, m.id, 1) === null} onclick={() => step(m, 1)}><Icon name="down" size={15} /></button>
          {/if}
          <button type="button" class="btn sm danger" onclick={() => remove(m)}><Icon name="trash" size={13} /> Remove</button>
        </div>
      </li>
    {/each}
  </ul>
{/if}
