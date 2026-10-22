<script lang="ts">
  import { app } from '../app.svelte';
  import { updater } from '../update.svelte';
  import { formatBytes } from '../format';
  import Progress from './Progress.svelte';
  import Spinner from './Spinner.svelte';
  import Chip from './Chip.svelte';

  const s = $derived(updater.state);
  const pct = $derived(s.kind === 'installing' && s.total ? Math.min(100, Math.round((s.done / s.total) * 100)) : null);
</script>

<div class="stack" aria-live="polite">
  <p class="muted" style="margin: 0">Installed: <Chip tone="accent">{app.info?.launcher_version ?? '?'}</Chip></p>
  {#if s.kind === 'checking'}
    <p><Spinner size={14} /> Checking for updates...</p>
  {:else if s.kind === 'current'}
    <p><Chip tone="ok" icon="check">The launcher is up to date</Chip></p>
  {:else if s.kind === 'unavailable'}
    <p class="muted">No update info available (no release published yet, or the release page cannot be reached).</p>
    <p class="hint">{s.detail}</p>
  {:else if s.kind === 'available'}
    <p><b>Update available: {s.version}</b>{s.date ? ` (${s.date.slice(0, 10)})` : ''}</p>
    {#if s.notes}<pre class="notes">{s.notes}</pre>{/if}
    <button type="button" class="btn primary" onclick={() => updater.install()}>Download and install</button>
    <p class="hint">The download is verified against the launcher's embedded public key; the launcher restarts afterwards.</p>
  {:else if s.kind === 'installing'}
    <p>{s.phase === 'downloading' ? 'Downloading the update...' : 'Installing the update...'}{#if s.total} {formatBytes(s.done)} of {formatBytes(s.total)}{/if}</p>
    {#if pct !== null}<Progress value={pct} />{/if}
  {:else if s.kind === 'restarting'}
    <p>Update installed, restarting...</p>
  {:else if s.kind === 'error'}
    <p class="err" role="alert">{s.message}</p>
  {/if}
  {#if s.kind !== 'available' && s.kind !== 'installing' && s.kind !== 'restarting'}
    <button type="button" class="btn" disabled={updater.busy} onclick={() => updater.check()}>Check for updates</button>
  {/if}
</div>

<style>
  .notes { margin: 0; max-height: 160px; overflow: auto; white-space: pre-wrap; overflow-wrap: anywhere; font: 12px/1.5 var(--mono); background: var(--bg-2); border: 1px solid var(--line); border-radius: 8px; padding: 8px 12px; }
  .err { color: var(--danger); margin: 0; }
</style>
