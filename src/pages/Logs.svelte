<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { ipc } from '../lib/ipc';
  import { run, toasts } from '../lib/notify.svelte';
  import { formatBytes, formatDateTime } from '../lib/format';
  import { fileLabel, groupFiles, highlight, visibleLines } from '../lib/logview';
  import Icon from '../lib/ui/Icon.svelte';
  import Chip from '../lib/ui/Chip.svelte';
  import Empty from '../lib/ui/Empty.svelte';
  import Spinner from '../lib/ui/Spinner.svelte';
  import type { LogContent, LogKind, LogList } from '../lib/types';

  let list = $state<LogList | null>(null);
  let selected = $state<string | null>(null);
  let content = $state<LogContent | null>(null);
  let loading = $state(false);
  let search = $state('');
  let onlyMatching = $state(false);
  let problemsOnly = $state(false);
  let report = $state('');
  let showReport = $state(false);
  let viewEl: HTMLElement | undefined = $state();

  const groups = $derived(groupFiles(list?.files ?? []));
  const file = $derived(list?.files.find((f) => f.id === selected) ?? null);
  const lines = $derived(visibleLines(content?.lines ?? [], search, onlyMatching, problemsOnly));
  const hits = $derived(search.trim() ? visibleLines(content?.lines ?? [], search, true).length : 0);

  async function reloadList(keep = true) {
    list = (await run(() => ipc.logList())) ?? list;
    if (!list) return;
    if (!keep || !list.files.some((f) => f.id === selected)) selected = list.files[0]?.id ?? null;
  }

  async function open(id: string) {
    selected = id;
    showReport = false;
    report = '';
    loading = true;
    content = (await run(() => ipc.logRead(id, 3000))) ?? null;
    loading = false;
    await tick();
    if (viewEl) viewEl.scrollTop = viewEl.scrollHeight;
  }

  onMount(async () => {
    await reloadList(false);
    if (selected) await open(selected);
  });

  async function refresh() {
    await reloadList();
    if (selected) await open(selected);
  }

  async function folder(kind: LogKind) {
    await run(() => ipc.logOpenFolder(kind));
  }

  async function copyText(text: string): Promise<boolean> {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      // The webview may refuse the async clipboard: fall back to a selection.
      const ta = document.createElement('textarea');
      ta.value = text;
      ta.style.position = 'fixed';
      ta.style.opacity = '0';
      document.body.appendChild(ta);
      ta.select();
      const ok = document.execCommand('copy');
      ta.remove();
      return ok;
    }
  }

  async function copyReport() {
    const text = await run(() => ipc.logReport(selected && file?.text ? selected : null, 200));
    if (text === undefined) return;
    report = text;
    showReport = true;
    if (await copyText(text)) toasts.show('ok', 'Copied for a bug report. Your home folder, account names and tokens were removed; look it over below before you paste it.');
    else toasts.show('error', 'Could not copy automatically: select the text below and copy it.');
  }
</script>

<div class="sechead">
  <p class="muted">The game logs the launcher saves, the hosted server's log and the crash dumps it sets aside so the game never uploads them.</p>
  <div class="toolbar">
    <button type="button" class="btn" onclick={refresh}><Icon name="refresh" size={14} /> Reload</button>
    <button type="button" class="btn primary" onclick={copyReport} title="System information and the last lines of the log, with your home folder, account names and tokens removed">
      <Icon name="copy" size={14} /> Copy for a bug report
    </button>
  </div>
</div>

{#if !list}
  <div class="loading-row"><Spinner size={16} /> Loading...</div>
{:else if list.files.length === 0}
  <Empty icon="file" title="No logs yet">
    The game's output is saved here when you press Play; the server's log appears when you host one.
    {#snippet action()}
      <div class="row" style="justify-content: center">
        <button type="button" class="btn" onclick={() => folder('game')}><Icon name="folder" size={14} /> Open the log folder</button>
        <button type="button" class="btn" onclick={copyReport}><Icon name="copy" size={14} /> Copy system information</button>
      </div>
    {/snippet}
  </Empty>
{:else}
  <div class="logs">
    <nav class="files" aria-label="Log files">
      {#each groups as g (g.kind)}
        <div class="grp">
          <div class="grp-head">
            <span>{g.title}</span>
            <button type="button" class="btn icon sm ghost" title="Open the folder" aria-label="Open the {g.title} folder" onclick={() => folder(g.kind)}><Icon name="folder" size={14} /></button>
          </div>
          {#each g.files as f (f.id)}
            <button type="button" class="file" class:on={selected === f.id} onclick={() => open(f.id)}>
              <Icon name="file" size={15} />
              <span class="fn">{fileLabel(f, formatDateTime)}</span>
              <span class="sz">{formatBytes(f.size)}</span>
            </button>
          {/each}
        </div>
      {/each}
    </nav>

    <section class="viewer card">
      <div class="vbar">
        <label class="searchbox">
          <Icon name="search" size={14} />
          <input type="search" placeholder="Search this log" bind:value={search} aria-label="Search the log" />
        </label>
        {#if search.trim()}<span class="hint">{hits} {hits === 1 ? 'line' : 'lines'}</span>{/if}
        <label class="check"><input type="checkbox" bind:checked={onlyMatching} disabled={!search.trim()} /> <span>Only matching lines</span></label>
        <label class="check"><input type="checkbox" bind:checked={problemsOnly} /> <span>Warnings and errors only</span></label>
      </div>
      {#if loading}
        <div class="loading-row"><Spinner size={16} /> Reading...</div>
      {:else if content?.binary || (file && !file.text)}
        <p class="muted pad">This is a binary file ({file ? formatBytes(file.size) : ''}); it cannot be shown. Open the folder to attach it to a bug report.</p>
      {:else if content}
        <div class="log big" bind:this={viewEl} role="log" aria-label={file?.name}>
          {#if content.truncated}<div class="muted cut">Earlier lines are not shown (the last {content.lines.length.toLocaleString()} lines of {formatBytes(content.size)}).</div>{/if}
          {#each lines as l (l.n)}
            <div class="ln {l.level}"><span class="no">{l.n}</span><span class="tx">{#each highlight(l.text, search) as part, i (i)}{#if part.hit}<mark>{part.s}</mark>{:else}{part.s}{/if}{/each}</span></div>
          {/each}
          {#if lines.length === 0}<span class="muted">{content.lines.length === 0 ? 'This file is empty.' : 'No line matches.'}</span>{/if}
        </div>
        <p class="hint foot">
          {#if file}<Chip>{file.kind}</Chip> <code>{file.name}</code>{/if}
          {content.lines.length.toLocaleString()} lines loaded.
        </p>
      {/if}
    </section>
  </div>

  {#if showReport}
    <section class="card" style="margin-top: 16px">
      <div class="card-head"><span class="head-icon"><Icon name="copy" size={16} /></span><h2>Bug report text (already copied)</h2></div>
      <textarea class="report" readonly rows="10" value={report} aria-label="Bug report text"></textarea>
      <p class="hint">Check it before you share it. The launcher removed your home folder, the account names it knows and anything that looks like a token or password, but a log can contain other things.</p>
    </section>
  {/if}
{/if}

<style>
  .sechead { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 10px 16px; margin-bottom: 16px; }
  .sechead p { margin: 0; flex: 1 1 300px; }
  .loading-row { display: flex; align-items: center; gap: 10px; color: var(--muted); padding: 24px; justify-content: center; }
  .logs { display: grid; grid-template-columns: 300px minmax(0, 1fr); gap: 16px; align-items: start; }
  .files { display: grid; gap: 14px; }
  .grp-head { display: flex; align-items: center; justify-content: space-between; font-size: 10.5px; font-weight: 650; letter-spacing: 0.09em; text-transform: uppercase; color: var(--faint); padding: 0 4px 4px 8px; }
  .file { display: flex; align-items: center; gap: 8px; width: 100%; padding: 7px 8px; border: 1px solid transparent; border-radius: 8px; background: transparent; color: var(--muted); text-align: left; cursor: pointer; font: inherit; font-size: 12.5px; }
  .file:hover { background: var(--surface); color: var(--text); }
  .file.on { background: color-mix(in srgb, var(--accent) 11%, var(--surface)); border-color: color-mix(in srgb, var(--accent) 35%, var(--line)); color: var(--text); }
  .fn { flex: 1 1 auto; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sz { font-size: 11px; color: var(--faint); font-variant-numeric: tabular-nums; }
  .viewer { padding: 14px; min-width: 0; }
  .vbar { display: flex; flex-wrap: wrap; align-items: center; gap: 8px 16px; margin-bottom: 10px; }
  .searchbox { display: flex; align-items: center; gap: 8px; padding: 0 10px; border: 1px solid var(--line); border-radius: var(--radius-sm); background: var(--bg-2); color: var(--muted); flex: 1 1 200px; }
  .searchbox input { border: 0; background: transparent; box-shadow: none; padding: 7px 0; flex: 1; min-width: 0; }
  .searchbox:focus-within { border-color: var(--accent); }
  .log.big { height: min(52vh, 520px); font: 12px/1.55 var(--mono); }
  .ln { display: flex; gap: 12px; white-space: pre-wrap; overflow-wrap: anywhere; }
  .ln .no { flex: none; width: 4ch; text-align: right; color: var(--faint); user-select: none; }
  .ln.error { color: var(--danger); }
  .ln.warn { color: var(--warn); }
  mark { background: color-mix(in srgb, var(--accent) 40%, transparent); color: inherit; border-radius: 3px; padding: 0 1px; }
  .cut { padding-bottom: 6px; }
  .pad { padding: 14px 4px; }
  .foot { margin: 8px 0 0; }
  .report { width: 100%; font: 12px/1.5 var(--mono); resize: vertical; }
  @media (max-width: 900px) { .logs { grid-template-columns: 1fr; } }
</style>
