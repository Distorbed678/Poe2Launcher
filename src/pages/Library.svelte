<script lang="ts">
  import { app } from '../lib/app.svelte';
  import { router } from '../lib/router.svelte';
  import { versionLabel } from '../lib/gameversion';
  import PageHead from '../lib/ui/PageHead.svelte';
  import Tabs from '../lib/ui/Tabs.svelte';
  import Install from './Install.svelte';
  import Mods from './Mods.svelte';
  import Logs from './Logs.svelte';

  // The official mode has no mods; the others get all three.
  const tabs = $derived([
    { id: 'game', label: 'Game', icon: 'install' as const },
    ...(app.official ? [] : [{ id: 'mods', label: 'Mods', icon: 'mods' as const }]),
    { id: 'logs', label: 'Logs', icon: 'file' as const },
  ]);
  const want = $derived(router.route.query.get('tab') ?? 'game');
  const tab = $derived(tabs.some((t) => t.id === want) ? want : 'game');
</script>

<PageHead icon="library" title="Library" sub={`The game for ${versionLabel(app.version)}, mods and logs`} />

<Tabs {tabs} value={tab} label="Library" onchange={(id) => router.go('library', { tab: id === 'game' ? undefined : id })} />

{#if tab === 'mods'}
  <Mods />
{:else if tab === 'logs'}
  <Logs />
{:else}
  <Install />
{/if}
