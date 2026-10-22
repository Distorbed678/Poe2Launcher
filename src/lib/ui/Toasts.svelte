<script lang="ts">
  import { fly, fade } from 'svelte/transition';
  import { flip } from 'svelte/animate';
  import { cubicOut } from 'svelte/easing';
  import { toasts } from '../notify.svelte';
  import { ms } from '../motion';
  import Icon from './Icon.svelte';
</script>

<div class="toasts" role="status" aria-live="polite">
  {#each toasts.items as t (t.id)}
    <div
      class="toast {t.kind}"
      in:fly={{ x: 40, duration: ms(260), easing: cubicOut }}
      out:fade={{ duration: ms(160) }}
      animate:flip={{ duration: ms(200) }}
    >
      <span class="toast-icon"><Icon name={t.kind === 'ok' ? 'check' : 'alert'} size={16} /></span>
      <span class="toast-text">{t.text}</span>
      <button type="button" class="btn icon ghost sm" aria-label="Dismiss" onclick={() => toasts.dismiss(t.id)}><Icon name="close" size={14} /></button>
      <span class="toast-timer" style:animation-duration="{t.kind === 'error' ? 9 : 4}s"></span>
    </div>
  {/each}
</div>
