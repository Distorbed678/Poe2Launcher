<script lang="ts">
  import { slide } from 'svelte/transition';
  import { app } from '../lib/app.svelte';
  import { ipc } from '../lib/ipc';
  import { run } from '../lib/notify.svelte';
  import { formatAgo, formatNewsDate } from '../lib/format';
  import { DUR, ease, ms } from '../lib/motion';
  import { newsPreview } from '../lib/markdown';
  import PageHead from '../lib/ui/PageHead.svelte';
  import Icon from '../lib/ui/Icon.svelte';
  import Spinner from '../lib/ui/Spinner.svelte';
  import Markdown from '../lib/ui/Markdown.svelte';
  import type { NewsItem } from '../lib/types';

  // One feed, newest first, with filter chips. A post shows its first lines; "Read more" opens
  // it in place. The selected server's message of the day is pinned on top.
  type Filter = 'all' | NewsItem['source'];
  const sel = $derived(app.selected);
  const info = $derived(sel ? app.infos[sel.id] : undefined);
  const motd = $derived(!app.official && typeof info === 'object' ? info.motd?.trim() || null : null);
  const items = $derived(app.news?.items ?? []);
  let filter = $state<Filter>('all');
  let expanded = $state<Record<string, boolean>>({});
  const shown = $derived(filter === 'all' ? items : items.filter((n) => n.source === filter));

  const sourceTitle: Record<NewsItem['source'], string> = { launcher: 'Launcher', server: 'Server', hook: 'Client hook', project: 'Project' };
  const filters: { id: Filter; label: string }[] = [
    { id: 'all', label: 'All' },
    { id: 'project', label: 'Project' },
    { id: 'server', label: 'Server' },
    { id: 'launcher', label: 'Launcher' },
    { id: 'hook', label: 'Client hook' },
  ];

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

<div class="chips" role="radiogroup" aria-label="Show">
  {#each filters as f (f.id)}
    {@const count = f.id === 'all' ? items.length : items.filter((n) => n.source === f.id).length}
    <button type="button" class="fchip" class:on={filter === f.id} role="radio" aria-checked={filter === f.id} disabled={count === 0 && f.id !== 'all'} onclick={() => (filter = f.id)}>
      {f.label}{#if f.id !== 'all'} <span class="n">{count}</span>{/if}
    </button>
  {/each}
</div>

<div class="feed">
  {#if motd && filter === 'all'}
    <article class="post pin">
      <div class="meta"><span class="pin-chip"><Icon name="news" size={12} /> Message of the day</span>{typeof info === 'object' ? info.name : sel?.name}</div>
      <Markdown source={motd} />
    </article>
  {/if}

  {#if !app.news && app.newsLoading}
    {#each [0, 1, 2] as i (i)}
      <div class="post"><div class="skeleton w40"></div><div class="skeleton h20 w70"></div><div class="skeleton"></div><div class="skeleton w60"></div></div>
    {/each}
  {:else if shown.length === 0}
    <div class="post"><p class="muted">{app.news?.offline ? 'No news yet, and GitHub could not be reached. The news appear when you are online.' : 'No news here yet.'}</p></div>
  {:else}
    {#each shown as n (n.id)}
      {@const open = expanded[n.id] ?? false}
      {@const preview = newsPreview(n.body)}
      <article class="post" class:open>
        <div class="meta">{sourceTitle[n.source]} · {formatNewsDate(n.date)}</div>
        <h2 class="title">{n.title}</h2>
        {#if open}
          <div transition:slide={{ duration: ms(DUR.slow), easing: ease }}><Markdown source={n.body} /></div>
        {:else if preview.text}
          <p class="preview">{preview.text}</p>
        {/if}
        <div class="post-foot">
          {#if preview.more || open}
            <button type="button" class="link" aria-expanded={open} onclick={() => (expanded[n.id] = !open)}>
              {open ? 'Show less' : 'Read more'} <Icon name="chevron" size={12} class={open ? 'flip' : ''} />
            </button>
          {/if}
          {#if n.link}
            <button type="button" class="link quiet" onclick={() => link(n.link!)}>Open on GitHub <Icon name="external" size={12} /></button>
          {/if}
        </div>
      </article>
    {/each}
  {/if}
</div>

<style>
  .chips { display: flex; gap: var(--sp-2); flex-wrap: wrap; margin-bottom: var(--sp-5); }
  .fchip { padding: 5px 13px; border: 1px solid var(--line); border-radius: var(--radius-pill); background: none; color: var(--muted); font: 500 var(--fs-sm) / 1.4 var(--font); cursor: pointer; transition: border-color var(--dur) var(--ease), color var(--dur) var(--ease), background var(--dur) var(--ease); }
  .fchip:hover:not(:disabled) { color: var(--text); border-color: color-mix(in srgb, var(--gold) 40%, var(--line)); }
  .fchip.on { border-color: var(--gold); color: var(--gold-2); background: color-mix(in srgb, var(--gold) 8%, transparent); }
  .fchip:disabled { opacity: 0.45; cursor: default; }
  .fchip .n { font-size: 11px; color: var(--faint); margin-left: 2px; }
  .feed { max-width: 780px; display: flex; flex-direction: column; gap: var(--sp-4); }
  .post { border: 1px solid var(--line); border-radius: var(--radius-lg); background: var(--surface); padding: var(--sp-5) var(--sp-5) var(--sp-4); display: flex; flex-direction: column; gap: var(--sp-2); }
  .post.pin { border-color: color-mix(in srgb, var(--gold) 50%, var(--line)); background: linear-gradient(90deg, color-mix(in srgb, var(--crimson) 22%, var(--surface)), var(--surface)); }
  .meta { display: flex; align-items: center; gap: var(--sp-3); font-size: 10.5px; letter-spacing: 0.1em; text-transform: uppercase; color: var(--faint); }
  .pin-chip { display: inline-flex; align-items: center; gap: 5px; padding: 1px 8px; border: 1px solid color-mix(in srgb, var(--gold) 50%, var(--line)); border-radius: var(--radius-pill); color: var(--gold-2); letter-spacing: 0.06em; }
  .title { font: 600 var(--fs-lg) / 1.3 var(--font-head); letter-spacing: 0.05em; text-transform: none; color: var(--text); margin: 0; }
  .post.open .title { color: var(--gold-2); }
  .preview { margin: 0; color: var(--muted); font-size: var(--fs-md); line-height: 1.6; }
  .post-foot { display: flex; align-items: center; gap: var(--sp-4); margin-top: var(--sp-1); }
  .post-foot :global(.flip) { transform: rotate(180deg); }
  .link.quiet { color: var(--faint); }
  .link.quiet:hover { color: var(--text); }
</style>
