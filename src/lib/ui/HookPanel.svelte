<script lang="ts">
  import { app } from '../app.svelte';
  import { hookAction, hookReady, hookSummary } from '../hook';
  import { formatBytes } from '../format';
  import { formatEta, formatSpeed } from '../transfer';
  import { versionLabel } from '../gameversion';
  import Icon from './Icon.svelte';
  import Chip from './Chip.svelte';
  import Spinner from './Spinner.svelte';
  import Progress from './Progress.svelte';

  let { compact = false }: { compact?: boolean } = $props();

  const h = $derived(app.hook);
  const check = $derived(app.hookCheck);
  const sum = $derived(hookSummary(h, check));
  const action = $derived(hookAction(h, check));
  const busy = $derived(app.hookTask !== '');
  const installing = $derived(app.hookTask === 'install');
  const pct = $derived(app.hookProgress && app.hookProgress.total > 0 ? (app.hookProgress.done / app.hookProgress.total) * 100 : null);
  const downloaded = $derived(h?.source.kind === 'downloaded');

  // "Check again" / "Check for updates" only look; the rest downloads.
  const looksOnly = $derived(action.label === 'Check again' || action.label === 'Check for updates');

  function go() {
    if (looksOnly) void app.checkHook();
    else void app.installHook();
  }
</script>

{#if compact}
  <section class="card hook-slim">
    <div class="slim-row">
      <span class="head-icon"><Icon name="box" size={16} /></span>
      <div class="grow slim-text">
        <b>Client hook</b>
        <span class="muted">
          {#if downloaded && h?.installed}{h.installed.version} for {h.installed.game_versions.join(', ')}{#if check && check.release.version !== h.installed.version} · newest {check.release.version}{/if}
          {:else}{sum.detail || 'The DLLs that connect the game to a private server.'}{/if}
        </span>
      </div>
      <Chip tone={sum.tone} icon={sum.tone === 'ok' ? 'check' : sum.tone === 'bad' ? 'alert' : undefined}>{sum.label}</Chip>
      <button type="button" class="btn sm" class:primary={action.primary} disabled={busy || !h || (h.busy ?? false)} onclick={go}>
        {#if busy}<Spinner size={12} />{:else}<Icon name={looksOnly ? 'refresh' : 'download'} size={13} />{/if}
        {action.label}
      </button>
    </div>
    {#if installing}
      <div class="progrow hook-prog">
        <span>{app.hookStep || 'working'}</span>
        <Progress value={pct} />
        <span class="num">{app.hookProgress ? formatBytes(app.hookProgress.done) : ''}</span>
      </div>
    {/if}
    {#if app.hookError}<p class="err slim-err">{app.hookError}</p>{/if}
  </section>
{:else}
<section class="card hook-card" class:compact>
  <div class="card-head">
    <span class="head-icon"><Icon name="box" size={16} /></span>
    <h2>Client hook</h2>
    <span class="grow"></span>
    <Chip tone={sum.tone} icon={sum.tone === 'ok' ? 'check' : sum.tone === 'bad' ? 'alert' : undefined}>{sum.label}</Chip>
  </div>
  <p class="hint">
    The hook is a pair of DLLs that connect the game to a private server (and, read-only, watch the official one). The launcher
    downloads it from <b>github.com/{h?.repo ?? 'Distorbed678/Poe2ClientHook'}</b> for <b>{versionLabel(h?.game_version ?? app.version)}</b>, checks its SHA-256
    and puts it in its own folder; Play uses it automatically.
  </p>
  {#if h?.source.kind === 'manual'}
    <div class="notice info">
      <span class="notice-title"><Icon name="folder" size={16} /> Your own build is in use</span>
      <p>{sum.detail} Clear the setting to use the downloaded hook.</p>
    </div>
  {:else if sum.detail && !downloaded}
    <p class="hint">{sum.detail}</p>
  {/if}
  {#if downloaded && h?.installed}
    <dl class="kv" style="margin: 10px 0">
      <dt>Installed</dt><dd><b>{h.installed.version}</b> <span class="muted">(mod ABI {h.installed.mod_abi})</span></dd>
      <dt>Game versions</dt><dd>{h.installed.game_versions.join(', ')}</dd>
      {#if check}<dt>Newest</dt><dd>{check.release.version}{#if check.release.prerelease} <Chip tone="warn">pre-release</Chip>{/if}{#if check.release.size} <span class="muted">({formatBytes(check.release.size)})</span>{/if}</dd>{/if}
      {#if !compact}<dt>Folder</dt><dd><code>{h.installed.dir}</code></dd>{/if}
    </dl>
  {/if}

  {#if installing}
    <div class="progrow hook-prog">
      <span>{app.hookStep || 'working'}</span>
      <Progress value={pct} />
      <span class="num">{app.hookProgress ? formatBytes(app.hookProgress.done) : ''}</span>
    </div>
    {#if app.hookProgress}
      <p class="hint">{formatSpeed(app.hookRate.bps)}{app.hookProgress.total > 0 ? ` · ${formatEta(app.hookRate.etaSecs)} left · ${formatBytes(app.hookProgress.total)} in all` : ''}</p>
    {/if}
  {/if}
  {#if app.hookError}
    <div class="notice danger"><span class="notice-title"><Icon name="alert" size={16} /> {looksOnly || !installing ? 'The client hook' : 'Download failed'}</span><p>{app.hookError}</p></div>
  {/if}
  <div class="actions" style="margin-top: 12px">
    <button type="button" class="btn" class:primary={action.primary} disabled={busy || !h || (h.busy ?? false)} onclick={go}>
      {#if busy}<Spinner size={14} />{:else}<Icon name={looksOnly ? 'refresh' : 'download'} size={14} />{/if}
      {action.label}
    </button>
    {#if !hookReady(h) && !compact}<span class="hint">or build it yourself and set the Hook DLL directory in Settings</span>{/if}
  </div>
</section>
{/if}

<style>
  .slim-row { display: flex; align-items: center; gap: var(--sp-3); flex-wrap: wrap; }
  .slim-row .head-icon { color: var(--gold); display: grid; place-items: center; }
  .slim-text { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .slim-text b { font-weight: 600; }
  .slim-text .muted { font-size: var(--fs-sm); }
  .slim-err { margin: var(--sp-3) 0 0; }
  .hook-prog { grid-template-columns: minmax(120px, 1fr) minmax(0, 2fr) 64px; margin-top: 8px; }
</style>
