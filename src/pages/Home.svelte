<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { app } from '../lib/app.svelte';
  import { ipc } from '../lib/ipc';
  import { run } from '../lib/notify.svelte';
  import { formatAgo, formatNewsDate, formatUptime } from '../lib/format';
  import { serverCompat, versionLabel } from '../lib/gameversion';
  import { isPlainHttpBlocked } from '../lib/url';
  import { plainText } from '../lib/markdown';
  import PageHead from '../lib/ui/PageHead.svelte';
  import Icon from '../lib/ui/Icon.svelte';
  import Chip from '../lib/ui/Chip.svelte';
  import Empty from '../lib/ui/Empty.svelte';
  import Spinner from '../lib/ui/Spinner.svelte';
  import Markdown from '../lib/ui/Markdown.svelte';
  import SetupChecklist from '../lib/ui/SetupChecklist.svelte';
  import type { NewsItem } from '../lib/types';

  const sel = $derived(app.selected);
  const info = $derived(sel ? app.infos[sel.id] : undefined);
  const ok = $derived(typeof info === 'object' ? info : null);
  const ping = $derived(sel ? app.pings[sel.id] : undefined);
  const compat = $derived(serverCompat(ok?.version, app.version, app.versions));
  const blocker = $derived(app.playBlocker);
  const busy = $derived(app.play.state === 'preparing');
  const running = $derived(app.play.state === 'running');
  const motd = $derived(ok?.motd?.trim() || null);
  let checking = $state(false);
  let timer: ReturnType<typeof setInterval> | undefined;
  let open = $state<Record<string, boolean>>({});

  async function check() {
    if (!sel || isPlainHttpBlocked(sel)) return;
    checking = true;
    await app.checkServer(sel.id);
    checking = false;
  }

  // The status follows the selected server and is looked at again every 30 s while Home is open.
  $effect(() => {
    void sel?.id;
    void check();
  });
  onMount(() => {
    timer = setInterval(() => void check(), 30_000);
  });
  onDestroy(() => timer && clearInterval(timer));

  /** Where to fix whatever blocks Play. */
  const fixLink = $derived.by(() => {
    if (!blocker) return null;
    if (/game first|client hook/i.test(blocker)) return { href: '#/library', label: 'Open Library' };
    if (/server first|plain http|Log in|client /i.test(blocker)) return { href: '#/servers', label: 'Open Servers' };
    return null;
  });

  const newsItems = $derived(app.news?.items ?? []);
  const sourceTitle: Record<NewsItem['source'], string> = { launcher: 'Launcher', server: 'Server', hook: 'Client hook', project: 'Project' };
  const sourceTone: Record<NewsItem['source'], 'accent' | 'info' | 'ok' | 'neutral'> = { launcher: 'accent', server: 'info', hook: 'ok', project: 'neutral' };

  function link(url: string) {
    void run(() => ipc.openLink(url));
  }
</script>

<PageHead icon="home" title="Home" sub={app.official ? 'Official servers, read-only' : 'Your server, setup and what is new'} />

<SetupChecklist />

{#if app.official}
  <section class="card hero official">
    <div class="hero-main">
      <span class="hero-badge"><Icon name="eye" size={22} /></span>
      <div class="grow">
        <h2>Official servers</h2>
        <p>The game starts with its own Steam login and the original realm list. The hook only watches; the launcher sends nothing and never logs in for you.</p>
      </div>
      <button type="button" class="big-play official" class:ready={!blocker} disabled={blocker !== null} onclick={() => void app.startPlay()}>
        {#if busy}<Spinner size={22} /> Starting{:else if running}<Icon name="check" size={22} /> Running{:else}<Icon name="play" size={22} filled /> Play{/if}
      </button>
    </div>
    {#if blocker && !busy && !running}<p class="why"><Icon name="alert" size={14} /> {blocker}
      {#if fixLink}<a href={fixLink.href}>{fixLink.label}</a>{/if}</p>{/if}
  </section>
{:else if !sel}
  <section class="card">
    <Empty icon="servers" title="No server selected">
      Find one in the public list, add one by address, or run your own on this computer.
      {#snippet action()}
        <div class="row" style="justify-content: center">
          <a class="btn primary" href="#/servers?tab=browse"><Icon name="compass" size={14} /> Browse servers</a>
          <a class="btn" href="#/servers"><Icon name="plus" size={14} /> Add by address</a>
          <a class="btn" href="#/host"><Icon name="host" size={14} /> Host a server</a>
        </div>
      {/snippet}
    </Empty>
  </section>
{:else}
  <section class="card hero" class:selected={!blocker}>
    <div class="hero-main">
      <span class="hero-badge"><Icon name="servers" size={22} /></span>
      <div class="grow">
        <div class="name">
          <h2>{ok?.name ?? sel.name}</h2>
          {#if ok}<Chip tone="ok" dot>online</Chip>
          {:else if typeof info === 'string'}<Chip tone="bad" dot>unreachable</Chip>
          {:else}<Chip><Spinner size={10} /> checking</Chip>{/if}
          {#if compat.kind === 'ok'}<Chip tone="ok" icon="check">compatible</Chip>
          {:else if compat.kind === 'other'}<Chip tone="bad" icon="alert">needs client {compat.version}</Chip>
          {:else if compat.kind === 'unsupported'}<Chip tone="bad" icon="alert">incompatible: client {compat.version}</Chip>{/if}
          {#if sel.logged_in}<Chip tone="info" icon="account">{sel.account}</Chip>{:else}<Chip>not logged in</Chip>{/if}
        </div>
        <p class="muted url"><code>{sel.url}</code> <span>{sel.name !== (ok?.name ?? sel.name) ? `(your name: ${sel.name})` : ''}</span></p>
      </div>
      <button type="button" class="big-play" class:ready={!blocker} disabled={blocker !== null} onclick={() => void app.startPlay()}>
        {#if busy}<Spinner size={22} /> Starting{:else if running}<Icon name="check" size={22} /> Running{:else}<Icon name="play" size={22} filled /> Play{/if}
      </button>
    </div>
    {#if ok}
      <div class="stat-row four">
        <div class="stat"><small>Players online</small><b>{ok.online}</b></div>
        <div class="stat"><small>Ping</small><b>{ping !== undefined ? `${ping} ms` : '-'}</b></div>
        <div class="stat"><small>Client version</small><b>{ok.version}</b></div>
        <div class="stat"><small>Uptime</small><b>{formatUptime(ok.uptime_secs)}</b></div>
      </div>
    {:else if typeof info === 'string'}
      <p class="err" style="margin: 12px 0 0">{info}</p>
    {/if}
    {#if blocker && !busy && !running}
      <p class="why"><Icon name="alert" size={14} /> {blocker}
        {#if fixLink}<a href={fixLink.href}>{fixLink.label}</a>{/if}
        {#if compat.kind === 'other'}<button type="button" class="link" onclick={() => app.switchVersion(compat.version)}>Switch to {compat.version}</button>{/if}
      </p>
    {/if}
    <div class="row foot">
      <button type="button" class="btn sm" onclick={check} disabled={checking}>
        {#if checking}<Spinner size={12} />{:else}<Icon name="refresh" size={13} />{/if} Check now
      </button>
      <a class="btn sm ghost" href="#/servers">Change server</a>
      <span class="grow"></span>
      <span class="hint">Playing {versionLabel(app.version)}</span>
    </div>
  </section>
{/if}

<section class="news" aria-labelledby="news-h">
  <div class="card-head" style="margin-top: 22px">
    <span class="head-icon"><Icon name="news" size={16} /></span>
    <h2 id="news-h">News</h2>
    <span class="grow"></span>
    {#if app.news?.offline}<span class="hint">offline{app.news.fetched_at ? `, saved ${formatAgo(app.news.fetched_at)}` : ''}</span>
    {:else if app.news?.fetched_at}<span class="hint">updated {formatAgo(app.news.fetched_at)}</span>{/if}
    <button type="button" class="btn sm ghost" onclick={() => app.loadNews(true)} disabled={app.newsLoading} aria-label="Refresh the news">
      {#if app.newsLoading}<Spinner size={12} />{:else}<Icon name="refresh" size={13} />{/if}
    </button>
  </div>
  {#if !app.news && app.newsLoading}
    <div class="loading-row"><Spinner size={16} /> Loading...</div>
  {:else if newsItems.length === 0}
    <div class="card"><p class="muted" style="margin: 0">{app.news?.offline ? 'No news yet, and GitHub could not be reached. This is not a problem: the news appear when you are online.' : 'No news yet.'}</p></div>
  {:else}
    <div class="news-list">
      {#each newsItems as n (n.id)}
        {@const long = n.body.length > 220 || n.body.includes('\n')}
        <article class="card news-item">
          <div class="n-head">
            <Chip tone={sourceTone[n.source]}>{sourceTitle[n.source]}{n.version ? ` ${n.version}` : ''}</Chip>
            <span class="hint">{formatNewsDate(n.date)}</span>
          </div>
          <h3>{n.title}</h3>
          {#if n.body}
            {#if long && !open[n.id]}
              <p class="muted preview">{plainText(n.body).slice(0, 200)}{plainText(n.body).length > 200 ? '...' : ''}</p>
            {:else}
              <Markdown source={n.body} />
            {/if}
          {/if}
          <div class="row">
            {#if long}<button type="button" class="link" onclick={() => (open[n.id] = !open[n.id])}>{open[n.id] ? 'Show less' : 'Read more'}</button>{/if}
            {#if n.link}<button type="button" class="link" onclick={() => link(n.link!)}>Open on GitHub <Icon name="external" size={12} /></button>{/if}
          </div>
        </article>
      {/each}
    </div>
  {/if}

  {#if sel && !app.official && motd}
    <div class="card-head" style="margin-top: 22px">
      <span class="head-icon"><Icon name="users" size={16} /></span>
      <h2>Message from {ok?.name ?? sel.name}</h2>
    </div>
    <div class="card motd"><Markdown source={motd} /></div>
  {/if}
</section>

<style>
  .hero { padding: 20px; }
  .hero.official { border-color: color-mix(in srgb, var(--warn) 40%, var(--line)); }
  .hero-main { display: flex; align-items: center; gap: 16px; flex-wrap: wrap; }
  .hero-badge { width: 48px; height: 48px; border-radius: 14px; display: grid; place-items: center; color: var(--accent); background: color-mix(in srgb, var(--accent) 14%, transparent); flex: none; }
  .official .hero-badge { color: var(--warn); background: color-mix(in srgb, var(--warn) 14%, transparent); }
  .hero h2 { margin: 0; font-size: 20px; text-transform: none; letter-spacing: 0; color: var(--text); }
  .hero p { margin: 4px 0 0; }
  .name { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .url { overflow-wrap: anywhere; }
  .big-play {
    position: relative; overflow: hidden; display: inline-flex; align-items: center; justify-content: center; gap: 10px;
    min-width: 168px; height: 56px; padding: 0 30px; border: 0; border-radius: 14px; cursor: pointer;
    font: 700 18px/1 var(--font); letter-spacing: 0.02em; color: var(--accent-ink);
    background: linear-gradient(135deg, var(--accent), var(--accent-2)); box-shadow: var(--glow);
    transition: transform 0.15s var(--ease), box-shadow 0.2s, filter 0.2s;
  }
  .big-play.official { background: linear-gradient(135deg, #ffb347, #ff6f4d); }
  .big-play:hover:not(:disabled) { transform: translateY(-1px); filter: brightness(1.06); }
  .big-play:active:not(:disabled) { transform: translateY(1px) scale(0.98); }
  .big-play:disabled { cursor: not-allowed; opacity: 0.45; box-shadow: none; filter: saturate(0.5); }
  .stat-row { margin: 16px 0 0; }
  .why { display: flex; align-items: center; flex-wrap: wrap; gap: 6px 10px; margin: 14px 0 0; padding: 10px 12px; border-radius: var(--radius-sm); border: 1px solid color-mix(in srgb, var(--warn) 35%, var(--line)); background: color-mix(in srgb, var(--warn) 8%, var(--surface)); color: var(--text); }
  .foot { margin-top: 14px; }
  .news-list { display: grid; gap: 12px; grid-template-columns: repeat(auto-fill, minmax(340px, 1fr)); align-items: start; }
  .news-item { display: flex; flex-direction: column; gap: 8px; }
  .n-head { display: flex; align-items: center; gap: 10px; }
  .news-item h3 { font-size: 15px; }
  .preview { margin: 0; font-size: 13px; }
  .motd { border-left: 3px solid var(--accent); }
  .loading-row { display: flex; align-items: center; gap: 10px; color: var(--muted); padding: 14px 0; }
</style>
