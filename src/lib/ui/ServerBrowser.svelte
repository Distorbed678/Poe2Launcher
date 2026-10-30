<script lang="ts">
  import { onMount } from 'svelte';
  import { fly } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import { app } from '../app.svelte';
  import { ipc } from '../ipc';
  import { run, toasts } from '../notify.svelte';
  import { ms } from '../motion';
  import { formatAgo, formatUptime } from '../format';
  import { versionLabel } from '../gameversion';
  import {
    allRegions, allTags, contactLink, defaultFilter, effectiveVersion, existingProfile, filterServers, sortServers,
    type BrowseFilter, type SortKey,
  } from '../browse';
  import Icon from './Icon.svelte';
  import Chip from './Chip.svelte';
  import Empty from './Empty.svelte';
  import Spinner from './Spinner.svelte';
  import type { ListedServer, ProbeResult, ServerList } from '../types';

  let list = $state<ServerList | null>(null);
  let probes = $state<Record<string, ProbeResult>>({});
  let loading = $state(true);
  let probing = $state(false);
  let adding = $state<string | null>(null);
  let allVersions = $state(false);
  let filter = $state<BrowseFilter>(defaultFilter(null));

  const servers = $derived(list?.servers ?? []);
  const version = $derived(app.official ? null : app.version);
  const effective = $derived<BrowseFilter>({ ...filter, version: allVersions ? null : version });
  const shown = $derived(sortServers(filterServers(servers, probes, effective), probes, filter.sort));
  const tags = $derived(allTags(servers));
  const regions = $derived(allRegions(servers));
  const hiddenByVersion = $derived(
    allVersions || !version ? 0 : filterServers(servers, probes, { ...effective, version: null }).length - shown.length,
  );

  async function load(force: boolean) {
    loading = true;
    try {
      list = await ipc.serverListGet(force);
    } catch (e) {
      list = { servers: [], skipped: [], fetched_at: 0, offline: true, error: e instanceof Error ? e.message : String(e) };
    }
    loading = false;
    void probeAll();
  }

  async function probeAll() {
    if (!list || list.servers.length === 0) return;
    probing = true;
    try {
      const results = await ipc.serverListProbe(list.servers.map((s) => s.id));
      probes = Object.fromEntries(results.map((r) => [r.id, r]));
    } catch (e) {
      toasts.show('error', e instanceof Error ? e.message : String(e));
    }
    probing = false;
  }

  onMount(() => void load(false));

  async function add(s: ListedServer) {
    adding = s.id;
    const r = await run(() => ipc.profileAdd(s.name, s.address, false), `Added ${s.name} to your servers`);
    adding = null;
    if (r) {
      await Promise.all([app.refreshProfiles(), app.refreshSettings()]);
      void app.checkServer(r.id);
    }
  }

  function setSort(e: Event) {
    filter.sort = (e.currentTarget as HTMLSelectElement).value as SortKey;
  }
</script>

<div class="browse">
  <div class="bar">
    <label class="search">
      <Icon name="search" size={15} />
      <input type="search" placeholder="Search servers" bind:value={filter.search} aria-label="Search the server list" />
    </label>
    <label class="sel"><span>Sort</span>
      <select value={filter.sort} onchange={setSort} aria-label="Sort by">
        <option value="players">Players</option>
        <option value="ping">Ping</option>
        <option value="name">Name</option>
      </select>
    </label>
    {#if regions.length > 1}
      <label class="sel"><span>Region</span>
        <select bind:value={filter.region} aria-label="Region">
          <option value={null}>All</option>
          {#each regions as r (r)}<option value={r}>{r}</option>{/each}
        </select>
      </label>
    {/if}
    {#if tags.length > 0}
      <label class="sel"><span>Tag</span>
        <select bind:value={filter.tag} aria-label="Tag">
          <option value={null}>All</option>
          {#each tags as t (t)}<option value={t}>{t}</option>{/each}
        </select>
      </label>
    {/if}
    <label class="check"><input type="checkbox" bind:checked={filter.onlineOnly} /> <span>Online only</span></label>
    <span class="grow"></span>
    <button type="button" class="btn" onclick={() => load(true)} disabled={loading || probing}>
      {#if loading || probing}<Spinner size={13} />{:else}<Icon name="refresh" size={14} />{/if} Refresh
    </button>
  </div>

  <div class="meta hint">
    {#if version}
      <label class="check"><input type="checkbox" bind:checked={allVersions} /> <span>Show other game versions too (you have {versionLabel(version)} selected)</span></label>
    {:else}
      <span>Official (read-only) is selected: private servers are not played from here, all versions are shown.</span>
    {/if}
    <span class="grow"></span>
    {#if list && list.fetched_at > 0}<span>List updated {formatAgo(list.fetched_at)}</span>{/if}
  </div>

  {#if list?.offline}
    <div class="notice" role="status">
      <span class="notice-title"><Icon name="alert" size={16} /> {servers.length > 0 ? 'Showing the saved list' : 'The server list could not be loaded'}</span>
      <p>{list.error || 'No connection.'}</p>
    </div>
  {/if}
  {#if list && list.skipped.length > 0}
    <details class="hint skipped">
      <summary>{list.skipped.length} list {list.skipped.length === 1 ? 'entry was' : 'entries were'} ignored (invalid)</summary>
      <ul>{#each list.skipped as m (m)}<li>{m}</li>{/each}</ul>
    </details>
  {/if}

  {#if loading && !list}
    <div class="loading-row"><Spinner size={18} /> Loading the list...</div>
  {:else if servers.length === 0}
    <Empty icon="compass" title="No public servers listed yet">
      Public servers are added to the list by pull request to <b>Distorbed678/Poe2ServerList</b> on GitHub. Until then, add a friend's server by address under <a href="#/servers">My servers</a>, or <a href="#/servers?host=1">create your own</a>.
    </Empty>
  {:else if shown.length === 0}
    <Empty icon="search" title="No server matches">
      {#if hiddenByVersion > 0}{hiddenByVersion} {hiddenByVersion === 1 ? 'server runs' : 'servers run'} another game version. Tick "Show other game versions too" to see {hiddenByVersion === 1 ? 'it' : 'them'}.{:else}Change the search or the filters.{/if}
    </Empty>
  {:else}
    <div class="grid">
      {#each shown as s (s.id)}
        {@const p = probes[s.id]}
        {@const mine = existingProfile(s, app.profiles)}
        {@const ver = effectiveVersion(s, p)}
        {@const contact = contactLink(s.contact)}
        <article class="card hover entry" in:fly={{ y: 10, duration: ms(220), easing: cubicOut }}>
          <div class="top">
            <h3>{s.name}</h3>
            <span class="grow"></span>
            {#if p?.ok}<Chip tone="ok" dot>online</Chip>
            {:else if p}<Chip tone="bad" dot>unreachable</Chip>
            {:else if probing}<Chip><Spinner size={10} /> checking</Chip>{/if}
          </div>
          <div class="chips">
            <Chip tone={version && ver !== version ? 'warn' : 'info'}>client {ver}</Chip>
            {#if s.region}<Chip icon="globe">{s.region}</Chip>{/if}
            {#each s.tags as t (t)}<Chip>{t}</Chip>{/each}
            {#if s.plain_http}<Chip tone="warn" icon="alert">plain http</Chip>{/if}
            {#if s.mods.length > 0}<Chip tone="accent" icon="mods">{s.mods.length} {s.mods.length === 1 ? 'mod' : 'mods'}</Chip>{/if}
          </div>
          {#if s.description}<p class="desc">{s.description}</p>{/if}
          <div class="stats">
            <span title="Players online"><Icon name="users" size={14} /> {p?.ok && p.info ? p.info.online : '-'}</span>
            <span title="Round trip measured by the launcher"><Icon name="clock" size={14} /> {p?.ping_ms != null ? `${p.ping_ms} ms` : '-'}</span>
            {#if p?.ok && p.info}<span class="muted">up {formatUptime(p.info.uptime_secs)}</span>
              <span class="muted">{p.info.open_registration ? 'registration open' : 'registration closed'}</span>{/if}
          </div>
          {#if p && !p.ok}<p class="hint err">{p.error}</p>{/if}
          {#if s.mods.length > 0}
            <p class="hint">Needs mods: {s.mods.map((m) => (m.version ? `${m.id} ${m.version}` : m.id)).join(', ')}. The launcher does not install or sync mods for servers yet.</p>
          {/if}
          <div class="foot">
            <span class="hint contact">
              {#if contact}<button type="button" class="link" onclick={() => run(() => ipc.openLink(contact))}>{contact}</button>
              {:else if s.contact}{s.contact}{/if}
            </span>
            <span class="grow"></span>
            <code class="addr">{s.address}</code>
            {#if mine}
              <Chip tone="ok" icon="check">in your servers</Chip>
            {:else}
              <button type="button" class="btn primary sm" disabled={adding === s.id} onclick={() => add(s)}>
                {#if adding === s.id}<Spinner size={12} />{:else}<Icon name="plus" size={13} />{/if} Add to my servers
              </button>
            {/if}
          </div>
        </article>
      {/each}
    </div>
  {/if}
</div>

<style>
  .bar { display: flex; flex-wrap: wrap; align-items: center; gap: 10px 14px; margin-bottom: 10px; }
  .search { display: flex; align-items: center; gap: 8px; flex: 1 1 260px; min-width: 200px; padding: 0 10px; border: 1px solid var(--line); border-radius: var(--radius-sm); background: var(--bg-2); color: var(--muted); }
  .search input { border: 0; background: transparent; padding: 8px 0; box-shadow: none; flex: 1; min-width: 0; }
  .search:focus-within { border-color: var(--accent); }
  .sel { display: flex; align-items: center; gap: 6px; font-size: 12.5px; color: var(--muted); }
  .sel select { width: auto; }
  .meta { display: flex; flex-wrap: wrap; align-items: center; gap: 8px 16px; margin-bottom: 12px; }
  .skipped { margin-bottom: 12px; }
  .skipped ul { margin: 6px 0 0; padding-left: 18px; }
  .loading-row { display: flex; align-items: center; gap: 10px; justify-content: center; padding: 40px; color: var(--muted); }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(380px, 1fr)); gap: 14px; }
  .entry { display: flex; flex-direction: column; gap: 10px; }
  .top { display: flex; align-items: center; gap: 10px; }
  .top h3 { font-size: 16px; overflow-wrap: anywhere; }
  .chips { display: flex; flex-wrap: wrap; gap: 6px; }
  .desc { margin: 0; color: var(--text); overflow-wrap: anywhere; }
  .stats { display: flex; flex-wrap: wrap; align-items: center; gap: 4px 16px; font-variant-numeric: tabular-nums; }
  .stats span { display: inline-flex; align-items: center; gap: 6px; }
  .foot { display: flex; flex-wrap: wrap; align-items: center; gap: 8px 12px; margin-top: auto; padding-top: 10px; border-top: 1px solid var(--line-soft); }
  .contact { min-width: 0; overflow-wrap: anywhere; }
  .addr { max-width: 100%; }
  .entry .hint { margin: 0; }
</style>
