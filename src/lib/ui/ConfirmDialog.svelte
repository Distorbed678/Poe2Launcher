<script lang="ts">
  import { confirmer } from '../confirm.svelte';
  import Icon from './Icon.svelte';

  let dialog: HTMLDialogElement | undefined = $state();
  let ticked = $state(false);
  let typed = $state('');
  const c = $derived(confirmer.current);
  const ok = $derived(!!c && (!c.acknowledge || ticked) && (!c.typeToConfirm || typed === c.typeToConfirm));

  $effect(() => {
    if (!dialog) return;
    if (c && !dialog.open) {
      ticked = false;
      typed = '';
      dialog.showModal();
    } else if (!c && dialog.open) {
      dialog.close();
    }
  });

  function cancel(e: Event) {
    // Escape: answer "no" ourselves so the promise settles.
    e.preventDefault();
    confirmer.answer(false);
  }
</script>

<dialog bind:this={dialog} class="modal" oncancel={cancel} aria-labelledby="confirm-title" aria-describedby="confirm-msg">
  {#if c}
    <form
      class="dialog-body"
      onsubmit={(e) => {
        e.preventDefault();
        if (ok) confirmer.answer(true);
      }}
    >
      <div class="dialog-head">
        <span class="dialog-icon" class:danger={c.danger}><Icon name={c.danger || c.acknowledge ? 'alert' : 'info'} size={20} /></span>
        <h2 id="confirm-title">{c.title}</h2>
      </div>
      <div id="confirm-msg" class="dialog-msg">
        {#each c.message.split('\n\n') as para (para)}<p>{para}</p>{/each}
      </div>
      {#if c.typeToConfirm}
        <label class="field">
          <span>Type <b class="confirm-word">{c.typeToConfirm}</b> to confirm</span>
          <!-- svelte-ignore a11y_autofocus -->
          <input bind:value={typed} autocomplete="off" spellcheck="false" autofocus />
        </label>
      {/if}
      {#if c.acknowledge}
        <label class="check ack"><input type="checkbox" bind:checked={ticked} /> <span>{c.acknowledge}</span></label>
      {/if}
      <div class="actions">
        <button type="button" class="btn" onclick={() => confirmer.answer(false)}>{c.cancelLabel ?? 'Cancel'}</button>
        <button type="submit" class="btn {c.danger ? 'danger solid' : 'primary'}" disabled={!ok}>
          {c.confirmLabel ?? 'OK'}
        </button>
      </div>
    </form>
  {/if}
</dialog>

<style>
  .confirm-word { color: var(--text); font-weight: 600; }
</style>
