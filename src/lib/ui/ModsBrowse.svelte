<script lang="ts">
  import { onMount } from 'svelte';
  import { ipc } from '../ipc';
  import { run, toasts } from '../notify.svelte';
  import { ask } from '../confirm.svelte';
  import { formatAgo, formatBytes } from '../format';
  import { SCOPE_ALL, bestVersion, compatLabel, emptyFilter, filterIndex, indexTags, sideLabel, usableVersion } from '../mods';
  import Icon from './Icon.svelte';
  import Chip from './Chip.svelte';
  import Empty from './Empty.svelte';
  import Spinner from './Spinner.svelte';
  import type { IndexMod, IndexVersion, ModIndex, ModsView } from '../types';

  let { data, scope, reload, openInstalled }: { data: ModsView | null; scope: string; reload: () => Promise<void>; openInstalled: () => void } = $props();

  let index = $state<ModIndex | null>(null);
  let loading = $state(false);
  let filter = $state({ ...emptyFilter });
  /** The version picked per mod; default: the best one. */
  let picked = $state<Record<string, string>>({});
  let busy = $state<string>('');

  const shown = $derived(index ? filterIndex(index.mods, filter) : []);
  const tags = $derived(index ? indexTags(index.mods) : []);
  const scopeName = $derived(scope === SCOPE_ALL ? 'all servers' : (data?.scope_name ?? 'this server'));

  async function load(force = false) {
    if (loading) return;
    loading = true;
    try {
      index = await ipc.modIndexGet(force);
    } catch {
      /* the backend never fails here; keep what is shown */
    }
    loading = false;
  }
  onMount(() => {
    void load();
  });

  function version(m: IndexMod): IndexVersion | null {
    return m.versions.find((v) => v.version === picked[m.id]) ?? bestVersion(m);
  }

  function installed(m: IndexMod): string[] {
    return (data?.entries ?? []).filter((e) => e.id === m.id).map((e) => e.version);
  }

  async function install(m: IndexMod, v: IndexVersion, enable: boolean) {
    if (enable && m.versions.length && v.hook_api !== null) {
      const ok = await ask({
        title: `Enable ${m.name} for ${scopeName}?`,
        message: `${m.name} ${v.version} contains a DLL that runs inside the game process with your user's rights. The download is checked against the SHA-256 in the index, which proves it is the file the index lists, not that it is safe. Install it only if you trust ${m.author || 'its author'}.`,
        confirmLabel: 'Install and enable',
      });
      if (!ok) return;
    }
    busy = `${m.id}@${v.version}`;
    const r = await run(() => ipc.modIndexInstall(m.id, v.version, enable ? scope : null));
    busy = '';
    if (r) {
      toasts.show('ok', `${r.already_cached ? 'Already cached: ' : 'Installed '}${m.name} ${r.version}${enable ? ` and enabled for ${scopeName}` : ''}`);
      await Promise.all([load(), reload()]);
    }
  }
</script>

<div class="bar">
  <label class="search">
    <Icon name="search" size={15} />
    <input type="search" placeholder="Search name, description, author, tag" bind:value={filter.query} aria-label="Search mods" />
  </label>
  <div class="seg" role="radiogroup" aria-label="Side" style="margin-bottom: 0">
    {#each [['any', 'Any'], ['client', 'Client'], ['both', 'Both'], ['server', 'Server']] as [v, l] (v)}
      <button type="button" role="radio" aria-checked={filter.side === v} class:on={filter.side === v} onclick={() => (filter.side = v as typeof filter.side)}>{l}</button>
    {/each}
  </div>
  {#if tags.length}
    <select bind:value={filter.tag} aria-label="Tag">
      <option value="">All tags</option>
      {#each tags as t (t)}<option value={t}>{t}</option>{/each}
    </select>
  {/if}
  <label class="sel"><input type="checkbox" bind:checked={filter.gameOnly} /> Only for {data?.game_version ?? 'this game version'}</label>
  <button type="button" class="btn icon ghost" title="Refresh the index" aria-label="Refresh the index" disabled={loading} onclick={() => load(true)}>
    {#if loading}<Spinner size={14} />{:else}<Icon name="refresh" size={15} />{/if}
  </button>
</div>

{#if index}
  <div class="meta">
    <span class="muted">{shown.length} of {index.mods.length} mods</span>
    {#if index.offline}
      <Chip tone="warn" icon="alert">saved copy{index.fetched_at ? `, ${formatAgo(index.fetched_at)}` : ''}</Chip>
    {:else if index.fetched_at}
      <span class="muted">updated {formatAgo(index.fetched_at)}</span>
    {/if}
    {#if index.skipped.length}<span title={index.skipped.join('\n')}><Chip tone="warn">{index.skipped.length} entries skipped</Chip></span>{/if}
    <span class="grow"></span>
    <span class="hint">Installing enables for: <b>{scopeName}</b> (change it above)</span>
  </div>
  {#if index.offline && index.error}<p class="hint">The index could not be reached ({index.error}); what you see is the last copy. This is quiet: nothing else is affected.</p>{/if}
{/if}

{#if loading && !index}
  <div class="loading-row"><Spinner size={18} /> Reading the mod index</div>
{:else if !index || index.mods.length === 0}
  <Empty icon="mods" title={index?.offline ? 'The mod index is not available' : 'The mod index is empty'}>
    {#if index?.offline}Connect to the internet and refresh. Mods you already installed keep working.{:else}Nobody has published a mod to the index yet. See the guide to submit yours.{/if}
    {#snippet action()}
      <div class="row" style="justify-content: center">
        <button type="button" class="btn" onclick={() => load(true)}><Icon name="refresh" size={14} /> Refresh</button>
        <button type="button" class="btn" onclick={openInstalled}>Back to installed</button>
      </div>
    {/snippet}
  </Empty>
{:else if shown.length === 0}
  <Empty icon="search" title="No mod matches">Loosen the search, or turn off "Only for {data?.game_version}".</Empty>
{:else}
  <ul class="cards">
    {#each shown as m (m.id)}
      {@const v = version(m)}
      {@const side = sideLabel(m.side)}
      {@const have = installed(m)}
      <li class="card entry">
        <div class="top">
          <strong class="name">{m.name}</strong>
          <Chip tone={side.tone}><span title={side.title}>{side.text}</span></Chip>
          {#if v && v.hook_api !== null}<Chip tone="warn" icon="alert">code</Chip>{/if}
          <span class="grow"></span>
          {#if m.licence}<span class="muted">{m.licence}</span>{/if}
        </div>
        <p class="desc">{m.description}</p>
        <div class="chips">
          {#each m.tags as t (t)}<Chip>{t}</Chip>{/each}
          <span class="muted by">{m.author ? `by ${m.author}` : ''} <code>{m.id}</code></span>
        </div>
        {#if v}
          {@const lab = compatLabel(v.compat)}
          <div class="vrow">
            {#if m.versions.length > 1}
              <select aria-label="Version of {m.name}" value={v.version} onchange={(ev) => (picked[m.id] = ev.currentTarget.value)}>
                {#each m.versions as x (x.version)}<option value={x.version}>{x.version}{x.yanked ? ' (withdrawn)' : ''}{usableVersion(x) ? '' : ' - not usable here'}</option>{/each}
              </select>
            {:else}
              <span>{v.version}</span>
            {/if}
            <Chip tone={lab.tone === 'ok' ? 'ok' : lab.tone === 'bad' ? 'bad' : 'warn'}>{lab.text}</Chip>
            <span class="muted">{formatBytes(v.size)}{v.released ? `, ${v.released}` : ''}</span>
            {#if v.cached}<Chip tone="ok" icon="check">in the cache</Chip>{:else if have.length}<Chip>{have.join(', ')} cached</Chip>{/if}
          </div>
        {/if}
        <div class="foot">
          {#if m.side === 'server'}
            <span class="hint">A server mod: it goes into a server's mods folder (<code>poe-server mod install</code>), not into the game.</span>
          {:else if !v}
            <span class="hint">All versions are withdrawn.</span>
          {:else}
            <button type="button" class="btn sm primary" disabled={busy !== '' || v.yanked} onclick={() => install(m, v, true)}>
              {#if busy === `${m.id}@${v.version}`}<Spinner size={13} />{:else}<Icon name="download" size={13} />{/if} Install and enable
            </button>
            <button type="button" class="btn sm" disabled={busy !== '' || v.yanked || v.cached} onclick={() => install(m, v, false)}>Install only</button>
          {/if}
          {#if m.homepage}<button type="button" class="btn sm ghost" onclick={() => void ipc.openLink(m.homepage)}><Icon name="portal" size={13} /> Homepage</button>{/if}
        </div>
      </li>
    {/each}
  </ul>
{/if}

<style>
  .bar { display: flex; flex-wrap: wrap; align-items: center; gap: 10px 14px; margin: 14px 0 10px; }
  .search { display: flex; align-items: center; gap: 8px; flex: 1 1 200px; min-width: 200px; padding: 0 10px; border: 1px solid var(--line); border-radius: var(--radius-sm); background: var(--bg-2); color: var(--muted); }
  .search input { border: 0; background: transparent; flex: 1; min-width: 0; box-shadow: none !important; }
  .bar select { width: auto; flex: none; }
  .sel { display: flex; align-items: center; gap: 6px; font-size: 12.5px; color: var(--muted); }
  .meta { display: flex; flex-wrap: wrap; align-items: center; gap: 8px 16px; margin-bottom: 12px; }
  .loading-row { display: flex; align-items: center; gap: 10px; justify-content: center; padding: 40px; color: var(--muted); }
  .cards { list-style: none; margin: 0; padding: 0; display: grid; grid-template-columns: repeat(auto-fill, minmax(380px, 1fr)); gap: 14px; }
  .entry { display: flex; flex-direction: column; gap: 10px; }
  .top { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .name { font-size: 15px; }
  .desc { margin: 0; color: var(--muted); font-size: 13px; line-height: 1.5; }
  .chips { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; }
  .by { font-size: 12px; margin-left: auto; }
  .vrow { display: flex; flex-wrap: wrap; align-items: center; gap: 8px 10px; }
  .vrow select { width: auto; min-height: 28px; }
  .foot { display: flex; flex-wrap: wrap; align-items: center; gap: 8px 10px; margin-top: auto; padding-top: 10px; border-top: 1px solid var(--line-soft); }
</style>
