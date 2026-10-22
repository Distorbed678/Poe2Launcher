<script lang="ts">
  import { app } from '../app.svelte';
  import { formatBytes } from '../format';
  import { dialogHasCode, dialogMods, sideLabel, totalBytes } from '../mods';
  import Chip from './Chip.svelte';
  import Icon from './Icon.svelte';
  import Progress from './Progress.svelte';
  import Spinner from './Spinner.svelte';

  let dialog: HTMLDialogElement | undefined = $state();
  const d = $derived(app.modDialog);
  const rows = $derived(d ? dialogMods(d.plan) : []);
  const code = $derived(dialogHasCode(rows));
  const prog = $derived(app.modProgress);

  $effect(() => {
    if (!dialog) return;
    if (d && !dialog.open) dialog.showModal();
    else if (!d && dialog.open) dialog.close();
  });

  function cancel(e: Event) {
    // Escape closes it like Cancel (not while a download runs).
    e.preventDefault();
    if (!d?.busy) app.closeModDialog(false);
  }
</script>

<dialog bind:this={dialog} class="modal wide" oncancel={cancel} aria-labelledby="modsync-title">
  {#if d}
    <div class="dialog-body">
      <div class="dialog-head">
        <span class="dialog-icon"><Icon name="mods" size={20} /></span>
        <h2 id="modsync-title">{d.plan.profile_name} needs {rows.length === 1 ? 'a mod' : `${rows.length} mods`}</h2>
      </div>
      <div class="dialog-msg">
        <p>
          This server requires {rows.length === 1 ? 'this mod' : 'these mods'} to play on it. They are downloaded from the server (or the addresses it lists),
          checked against the SHA-256 and package rules the server advertised, and switched on for this server only. Your other mods stay off here.
        </p>
      </div>
      <ul class="syncrows" aria-label="Required mods">
        {#each rows as m (m.id)}
          {@const side = sideLabel(m.side)}
          <li>
            <div class="grow">
              <strong>{m.name}</strong> <span class="muted">{m.version}</span>
              <Chip tone={side.tone}>{side.text}</Chip>
              {#if m.code}<Chip tone="warn" icon="alert">contains code</Chip>{:else}<Chip>data only</Chip>{/if}
              {#if m.state === 'changed'}<Chip tone="warn">replaces a cached copy</Chip>{/if}
              <div class="meta">
                <code>{m.id}</code> - {formatBytes(m.size)}{#if m.authors.length}{' - ' + m.authors.join(', ')}{/if}
                {#if m.description}<div>{m.description}</div>{/if}
              </div>
            </div>
          </li>
        {/each}
      </ul>
      {#if code}
        <p class="warnline">
          <Icon name="alert" size={15} /> A mod with code is a DLL that runs inside the game on your computer with your user's rights. The checks prove it is the file the server
          announced, not that it is safe. Download only from a server you trust.
        </p>
      {/if}
      {#if d.plan.stale}
        <p class="warnline"><Icon name="alert" size={15} /> The server did not answer just now; this is what it listed last time.</p>
      {/if}
      {#if d.busy}
        <div class="prog" aria-live="polite">
          <Spinner size={14} />
          <span>Downloading{prog ? ` ${prog.id} ${prog.version}` : ''}{prog && prog.total > 0 ? ` - ${formatBytes(prog.done)} of ${formatBytes(prog.total)}` : ''}</span>
          {#if prog && prog.total > 0}<Progress value={(prog.done / prog.total) * 100} label="Mod download" />{/if}
        </div>
      {/if}
      {#if d.error}
        <p class="err" role="alert">{d.error}</p>
        <p class="hint">Nothing was installed from a download that failed a check.</p>
      {/if}
      <div class="actions">
        <button type="button" class="btn" disabled={d.busy} onclick={() => app.closeModDialog(false)}>Cancel</button>
        <button type="button" class="btn primary" disabled={d.busy || rows.length === 0} onclick={() => void app.downloadMods()}>
          <Icon name="download" size={14} /> Download {formatBytes(totalBytes(rows))}
        </button>
      </div>
    </div>
  {/if}
</dialog>

<style>
  dialog.wide { width: min(620px, calc(100vw - 32px)); }
  .syncrows { list-style: none; margin: 14px 0 0; padding: 0; display: grid; gap: 8px; }
  .syncrows li { display: flex; gap: 12px; padding: 10px 12px; border-radius: var(--radius-sm); border: 1px solid var(--line); background: var(--bg-2); }
  .meta { font-size: 12px; color: var(--muted); margin-top: 3px; }
  .warnline { display: flex; gap: 8px; align-items: flex-start; margin: 12px 0 0; padding: 10px 12px; border-radius: var(--radius-sm); border: 1px solid color-mix(in srgb, var(--warn) 38%, var(--line)); background: color-mix(in srgb, var(--warn) 8%, var(--surface)); font-size: 12.5px; color: var(--text); }
  .prog { display: grid; gap: 6px; margin-top: 14px; font-size: 12.5px; color: var(--muted); }
  .err { color: var(--danger); margin: 12px 0 0; white-space: pre-wrap; overflow-wrap: anywhere; }
</style>
