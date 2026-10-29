<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { app } from '../lib/app.svelte';
  import { formatNewsDate } from '../lib/format';
  import { serverCompat } from '../lib/gameversion';
  import { isPlainHttpBlocked } from '../lib/url';
  import { updateHeadline } from '../lib/host';
  import { router } from '../lib/router.svelte';
  import Icon from '../lib/ui/Icon.svelte';
  import Spinner from '../lib/ui/Spinner.svelte';
  import Dropdown, { type DropItem } from '../lib/ui/Dropdown.svelte';
  import SetupChecklist from '../lib/ui/SetupChecklist.svelte';

  // The Play page: the server, the Play button and the state around it. Everything else is one level down.
  const sel = $derived(app.selected);
  const info = $derived(sel ? app.infos[sel.id] : undefined);
  const ok = $derived(typeof info === 'object' ? info : null);
  const ping = $derived(sel ? app.pings[sel.id] : undefined);
  const compat = $derived(serverCompat(ok?.version, app.version, app.versions));
  const blocker = $derived(app.playBlocker);
  const busy = $derived(app.play.state === 'preparing');
  const running = $derived(app.play.state === 'running');
  const updateText = $derived(app.official ? null : updateHeadline(app.serverUpdate));
  const latest = $derived(app.news?.items[0]);
  let timer: ReturnType<typeof setInterval> | undefined;

  function check() {
    if (sel && !isPlainHttpBlocked(sel)) void app.checkServer(sel.id);
  }

  // The status follows the selected server and is looked at again every 30 s while this page is open.
  $effect(() => {
    void sel?.id;
    check();
  });
  onMount(() => {
    // A newer server release for the server you host: quiet, not more than every 10 minutes.
    void app.checkServerUpdate();
    timer = setInterval(check, 30_000);
  });
  onDestroy(() => timer && clearInterval(timer));

  const stateText = $derived.by(() => {
    const p = app.play;
    if (p.state === 'preparing') return `Preparing: ${p.step}...`;
    if (p.state === 'running') return 'Game running';
    if (p.state === 'exited' && !blocker) return p.code === 0 || p.code === null ? 'Game closed. Ready again.' : `Game exited with code ${p.code}`;
    return null;
  });

  /** Where to fix whatever blocks Play. */
  const fixLink = $derived.by(() => {
    if (!blocker) return null;
    if (/game first|client hook/i.test(blocker)) return { href: '#/library', label: 'Open Library' };
    if (/server first|plain http|Log in|client /i.test(blocker)) return { href: '#/servers', label: 'Open Servers' };
    return null;
  });

  const MANAGE = '__manage__';
  const serverItems = $derived<DropItem[]>([
    ...app.profiles.map((p) => ({
      value: p.id,
      label: p.name,
      sub: p.logged_in ? `signed in as ${p.account}` : 'not signed in',
      badges: p.logged_in ? [] : [{ text: 'log in', tone: 'warn' as const }],
    })),
    { value: MANAGE, label: 'Manage servers...', icon: 'settings' as const },
  ]);

  function pick(id: string) {
    if (id === MANAGE) router.go('servers');
    else void app.select(id);
  }
</script>

<div class="play-wrap">
  <SetupChecklist />

  {#if updateText && app.serverUpdate?.release}
    <a class="play-note" href="#/servers?host=1"><Icon name="download" size={14} /> {updateText} <span class="go">Open your server</span></a>
  {/if}

  <div class="play-center">
    {#if app.official}
      <div class="server-plate official">
        <span class="plate-icon"><Icon name="eye" size={18} /></span>
        <span class="plate-text">
          <b>Official servers</b>
          <small>Steam login and the original realms. The hook only watches.</small>
        </span>
      </div>
    {:else if !sel}
      <div class="server-plate empty">
        <span class="plate-text">
          <b>No server yet</b>
          <small>Pick one from the public list, add one by address, or create your own.</small>
        </span>
        <a class="btn sm" href="#/servers">Choose</a>
      </div>
    {:else}
      <Dropdown items={serverItems} value={sel.id} label="Server" variant="plate" onselect={pick}>
        {#snippet trigger()}
          <span class="dot-state" class:on={!!ok} class:bad={typeof info === 'string'}></span>
          <span class="plate-text">
            <b>{ok?.name ?? sel.name}</b>
            <small>
              {#if ok}{ok.online} online{ping !== undefined ? ` · ${ping} ms` : ''} · {sel.logged_in ? sel.account : 'not signed in'}
              {:else if typeof info === 'string'}unreachable
              {:else}<Spinner size={10} /> checking{/if}
            </small>
          </span>
          <span class="plate-change">Change</span>
        {/snippet}
      </Dropdown>
    {/if}

    <div class="ornament" aria-hidden="true"><span class="gem"></span></div>

    <button
      type="button"
      class="play-btn"
      class:ready={!blocker}
      class:official={app.official}
      disabled={blocker !== null}
      title={blocker ?? (app.official ? 'Deploy the read-only hook and start the game' : 'Deploy the hook and start the game')}
      onclick={() => void app.startPlay()}
    >
      {#if busy}<Spinner size={20} label="Starting" /> Starting{:else if running}Running{:else}Play{/if}
    </button>

    <p class="play-state" class:blocked={!!blocker && !busy && !running} aria-live="polite">
      {#if stateText}
        {stateText}
      {:else if blocker && !running}
        <Icon name="alert" size={13} /> {blocker.replace(/ \((Library|Servers)\)$/, '')}
        {#if fixLink}<a href={fixLink.href}>{fixLink.label}</a>{/if}
        {#if sel && !app.official && compat.kind === 'other'}<button type="button" class="link" onclick={() => app.switchVersion(compat.version)}>Switch to {compat.version}</button>{/if}
      {:else}
        <span class="ok-dot"></span> Ready to play
      {/if}
    </p>
  </div>

  {#if latest}
    <a class="news-teaser" href="#/news">
      <span class="tag">News</span>
      <span class="t">{latest.title}</span>
      <span class="d">{formatNewsDate(latest.date)}</span>
      <Icon name="chevron" size={14} class="go" />
    </a>
  {/if}
</div>

<style>
  .play-wrap { position: relative; min-height: 100%; display: flex; flex-direction: column; gap: var(--sp-4); }
  .play-wrap :global(.setup) { max-width: 720px; width: 100%; margin: 0 auto; }
  .play-note { align-self: center; display: inline-flex; align-items: center; gap: 8px; font-size: var(--fs-sm); color: var(--gold-2); text-decoration: none; border: 1px solid color-mix(in srgb, var(--gold) 45%, var(--line)); background: var(--surface); padding: 6px 12px; border-radius: var(--radius); transition: border-color var(--dur) var(--ease); }
  .play-note:hover { border-color: var(--gold); }
  .play-note .go { color: var(--muted); }

  .play-center { flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: var(--sp-4); padding: var(--sp-6) 0; }

  .server-plate, .play-center :global(.dd.plate .dd-trigger) {
    display: flex; align-items: center; gap: var(--sp-3); width: 340px; min-height: 58px; padding: 10px 14px;
    background: color-mix(in srgb, var(--surface) 88%, transparent); border: 1px solid var(--line); border-radius: var(--radius); text-align: left;
  }
  .play-center :global(.dd.plate) { display: block; }
  .play-center :global(.dd.plate .dd-trigger:hover:not(:disabled)) { border-color: color-mix(in srgb, var(--gold) 60%, var(--line)); }
  .play-center :global(.dd.plate.open .dd-trigger) { border-color: var(--gold); box-shadow: none; }
  .play-center :global(.dd.plate .dd-chevron) { color: var(--gold); }
  .play-center :global(.dd.plate .dd-menu) { width: 340px; }
  .server-plate.official { border-color: color-mix(in srgb, var(--warn) 45%, var(--line)); }
  .plate-icon { color: var(--warn); display: grid; }
  .plate-text { display: flex; flex-direction: column; min-width: 0; flex: 1; }
  .plate-text b { font-size: var(--fs-md); font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .plate-text small { display: flex; align-items: center; gap: 6px; font-size: 11.5px; color: var(--muted); }
  .plate-change { font-size: var(--fs-xs); color: var(--gold); }
  .dot-state { width: 8px; height: 8px; border-radius: 50%; background: var(--faint); flex: none; transition: background var(--dur) var(--ease); }
  .dot-state.on { background: var(--ok); box-shadow: 0 0 8px var(--ok); }
  .dot-state.bad { background: var(--danger); }

  .ornament { display: flex; align-items: center; gap: 10px; width: 340px; color: var(--gold); }
  .ornament::before, .ornament::after { content: ''; flex: 1; height: 1px; background: linear-gradient(90deg, transparent, var(--gold)); }
  .ornament::after { background: linear-gradient(90deg, var(--gold), transparent); }
  .gem { width: 6px; height: 6px; background: var(--gold); transform: rotate(45deg); }

  .play-btn {
    display: inline-flex; align-items: center; justify-content: center; gap: 12px; width: 340px; height: 68px;
    font: 700 24px/1 var(--font-head); letter-spacing: 0.24em; text-indent: 0.24em; color: var(--on-crimson);
    background: var(--play-bg); border: 1px solid var(--gold); border-radius: var(--radius); cursor: pointer;
    box-shadow: var(--glow), inset 0 1px 0 rgb(255 255 255 / 0.18);
    transition: filter var(--dur) var(--ease), box-shadow var(--dur-slow) var(--ease), transform var(--dur-fast) var(--ease), opacity var(--dur) var(--ease);
  }
  .play-btn:hover:not(:disabled) { filter: brightness(1.12); box-shadow: 0 12px 44px rgb(209 161 78 / 0.32), inset 0 1px 0 rgb(255 255 255 / 0.22); }
  .play-btn:active:not(:disabled) { transform: translateY(1px); }
  .play-btn:disabled { cursor: not-allowed; opacity: 0.5; box-shadow: none; filter: saturate(0.4); }
  .play-btn:disabled:has(:global(.spinner)) { opacity: 0.9; filter: none; }

  .play-state { margin: 0; min-height: 22px; display: flex; align-items: center; justify-content: center; flex-wrap: wrap; gap: 6px 10px; font-size: var(--fs-sm); color: var(--muted); text-align: center; max-width: 520px; }
  .play-state.blocked { color: var(--warn); }
  .play-state a, .play-state .link { color: var(--gold-2); }
  .ok-dot { width: 7px; height: 7px; border-radius: 50%; background: var(--ok); }

  .news-teaser {
    display: flex; align-items: center; gap: var(--sp-3); padding: 10px 14px; text-decoration: none; color: var(--muted); font-size: var(--fs-sm);
    border: 1px solid var(--line); background: color-mix(in srgb, var(--surface) 82%, transparent); border-radius: var(--radius);
    transition: border-color var(--dur) var(--ease), color var(--dur) var(--ease);
  }
  .news-teaser:hover { border-color: color-mix(in srgb, var(--gold) 55%, var(--line)); color: var(--text); }
  .news-teaser .tag { color: var(--gold-2); font-weight: 600; }
  .news-teaser .t { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text); }
  .news-teaser .d { color: var(--faint); }
  .news-teaser :global(.go) { transform: rotate(-90deg); color: var(--gold); }
</style>
