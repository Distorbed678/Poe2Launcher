<script lang="ts">
  import { fade } from 'svelte/transition';
  import { app } from '../lib/app.svelte';
  import { ipc } from '../lib/ipc';
  import { run } from '../lib/notify.svelte';
  import { formatAgo, formatNewsDate } from '../lib/format';
  import { DUR, ease, ms } from '../lib/motion';
  import PageHead from '../lib/ui/PageHead.svelte';
  import Icon from '../lib/ui/Icon.svelte';
  import Spinner from '../lib/ui/Spinner.svelte';
  import Markdown from '../lib/ui/Markdown.svelte';
  import type { NewsItem } from '../lib/types';

  // Reading layout: the list on the right, the chosen item in full on the left. The selected
  // server's message of the day is the first entry of the list.
  const sel = $derived(app.selected);
  const info = $derived(sel ? app.infos[sel.id] : undefined);
  const motd = $derived(!app.official && typeof info === 'object' ? info.motd?.trim() || null : null);
  const items = $derived(app.news?.items ?? []);
  const MOTD = '__motd__';
  let chosen = $state<string | null>(null);
  const current = $derived(chosen === MOTD && motd ? null : (items.find((n) => n.id === chosen) ?? items[0] ?? null));
  const showMotd = $derived(chosen === MOTD && motd !== null);

  const sourceTitle: Record<NewsItem['source'], string> = { launcher: 'Launcher', server: 'Server', hook: 'Client hook', project: 'Project' };

  function link(url: string) {
    void run(() => ipc.openLink(url));
  }
</script>

<PageHead title="News" sub="Project releases and your server's message of the day">
  {#if app.news?.offline}<span class="hint">offline{app.news.fetched_at ? `, saved ${formatAgo(app.news.fetched_at)}` : ''}</span>
  {:else if app.news?.fetched_at}<span class="hint">updated {formatAgo(app.news.fetched_at)}</span>{/if}
  <button type="button" class="btn sm" onclick={() => app.loadNews(true)} disabled={app.newsLoading} aria-label="Refresh the news">
    {#if app.newsLoading}<Spinner size={12} />{:else}<Icon name="refresh" size={13} />{/if} Refresh
  </button>
</PageHead>

{#if !app.news && app.newsLoading}
  <div class="reader">
    <div class="card feature"><div class="skeleton w40"></div><div class="skeleton h20 w70"></div><div class="skeleton"></div><div class="skeleton w90"></div><div class="skeleton w60"></div></div>
    <div class="list">{#each [0, 1, 2] as i (i)}<div class="entry"><div class="skeleton w40"></div><div class="skeleton w80"></div></div>{/each}</div>
  </div>
{:else if items.length === 0 && !motd}
  <div class="card"><p class="muted" style="margin: 0">{app.news?.offline ? 'No news yet, and GitHub could not be reached. The news appear when you are online.' : 'No news yet.'}</p></div>
{:else}
  <div class="reader">
    {#key showMotd ? MOTD : current?.id}
      <article class="card feature" in:fade={{ duration: ms(DUR.base), easing: ease }}>
        {#if showMotd && sel}
          <span class="meta">Message of the day · {typeof info === 'object' ? info.name : sel.name}</span>
          <h2 class="title">From your server</h2>
          <Markdown source={motd ?? ''} />
        {:else if current}
          <span class="meta">{sourceTitle[current.source]}{current.version ? ` ${current.version}` : ''} · {formatNewsDate(current.date)}</span>
          <h2 class="title">{current.title}</h2>
          {#if current.body}<Markdown source={current.body} />{/if}
          {#if current.link}
            <button type="button" class="link" onclick={() => link(current.link!)}>Open on GitHub <Icon name="external" size={12} /></button>
          {/if}
        {/if}
      </article>
    {/key}
    <div class="list" role="listbox" aria-label="News">
      {#if motd}
        <button type="button" class="entry" class:on={showMotd} role="option" aria-selected={showMotd} onclick={() => (chosen = MOTD)}>
          <span class="meta">Your server · message of the day</span>
          <b>{typeof info === 'object' ? info.name : sel?.name}</b>
        </button>
      {/if}
      {#each items as n (n.id)}
        {@const on = !showMotd && current?.id === n.id}
        <button type="button" class="entry" class:on role="option" aria-selected={on} onclick={() => (chosen = n.id)}>
          <span class="meta">{sourceTitle[n.source]} · {formatNewsDate(n.date)}</span>
          <b>{n.title}</b>
        </button>
      {/each}
    </div>
  </div>
{/if}

<style>
  .reader { display: grid; grid-template-columns: minmax(0, 1.6fr) minmax(220px, 1fr); gap: var(--sp-4); align-items: start; }
  .feature { display: flex; flex-direction: column; gap: var(--sp-2); min-height: 260px; background: linear-gradient(180deg, color-mix(in srgb, var(--crimson) 26%, var(--surface)), var(--surface) 55%); }
  .feature .title { font: 600 var(--fs-lg) / 1.3 var(--font-head); letter-spacing: 0.06em; text-transform: none; color: var(--gold-2); margin: 0 0 var(--sp-1); }
  .meta { font-size: 10.5px; letter-spacing: 0.08em; text-transform: uppercase; color: var(--faint); }
  .feature .link { align-self: flex-start; margin-top: var(--sp-2); }
  .list { display: flex; flex-direction: column; border: 1px solid var(--line); border-radius: var(--radius); overflow: hidden; background: var(--surface); }
  .entry { display: flex; flex-direction: column; gap: 2px; padding: 10px 12px; text-align: left; background: transparent; border: 0; border-bottom: 1px solid var(--line-soft); cursor: pointer; color: var(--muted); transition: background var(--dur-fast) var(--ease), color var(--dur-fast) var(--ease); }
  .entry:last-child { border-bottom: 0; }
  .entry b { font-weight: 600; color: var(--text); font-size: 13px; }
  .entry:hover { background: var(--surface-2); }
  .entry.on { background: var(--selected-bg); box-shadow: inset 2px 0 0 var(--gold); }
  .entry.on b { color: var(--gold-2); }
  @media (max-width: 900px) { .reader { grid-template-columns: 1fr; } }
</style>
