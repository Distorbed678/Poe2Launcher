<script lang="ts">
  import { toasts } from '../notify.svelte';
</script>

<div class="toasts" role="status" aria-live="polite">
  {#each toasts.items as t (t.id)}
    <div class="toast {t.kind}">
      <span>{t.text}</span>
      <button type="button" class="icon" aria-label="Dismiss" onclick={() => toasts.dismiss(t.id)}>x</button>
    </div>
  {/each}
</div>
