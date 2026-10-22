<script lang="ts">
  import { app } from '../lib/app.svelte';
  import { ipc } from '../lib/ipc';
  import { run } from '../lib/notify.svelte';
  import { defaultScope, scopeOptions } from '../lib/mods';
  import Tabs from '../lib/ui/Tabs.svelte';
  import ModsInstalled from '../lib/ui/ModsInstalled.svelte';
  import ModsBrowse from '../lib/ui/ModsBrowse.svelte';
  import type { ModsView } from '../lib/types';

  // Mods are per server: the page works on one server's set at a time (or on "All servers").
  let scope = $state(defaultScope(app.settings?.selected_profile, app.profiles));
  let view = $state<'installed' | 'browse'>('installed');
  let data = $state<ModsView | null>(null);

  const options = $derived(scopeOptions(app.profiles));

  async function load() {
    data = (await run(() => ipc.mods(scope))) ?? data;
  }
  $effect(() => {
    void scope;
    void app.status?.install_dir;
    void load();
  });
</script>

<div class="sechead">
  <p class="muted">
    Mods are kept in the launcher's cache (several versions side by side) and switched on <b>per server</b>: a server's required mods are always on for it,
    your own mods only where you enable them.
  </p>
  <label class="scope">
    <span class="muted">Mods for</span>
    <select bind:value={scope} aria-label="Mods for which server">
      {#each options as o (o.value)}<option value={o.value}>{o.label}</option>{/each}
    </select>
  </label>
</div>

<Tabs
  tabs={[
    { id: 'installed', label: 'Installed', icon: 'mods', badge: data ? String(new Set(data.entries.map((e) => e.id)).size) : undefined },
    { id: 'browse', label: 'Browse', icon: 'search' },
  ]}
  value={view}
  label="Mods"
  onchange={(id) => (view = id as 'installed' | 'browse')}
/>

{#if view === 'browse'}
  <ModsBrowse {data} {scope} reload={load} openInstalled={() => (view = 'installed')} />
{:else}
  <ModsInstalled {data} {scope} reload={load} openBrowse={() => (view = 'browse')} />
{/if}

<style>
  .scope { display: inline-flex; align-items: center; gap: 8px; flex: none; }
  .scope select { min-width: 190px; }
</style>
