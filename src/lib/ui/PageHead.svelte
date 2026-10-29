<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { IconName } from './Icon.svelte';
  // `icon` is accepted for the callers but not drawn: the headings stand on their own.
  let { title, sub, children }: { title: string; sub?: string; icon?: IconName; children?: Snippet } = $props();
</script>

<div class="page-head">
  <div class="page-title">
    <h1>{title}</h1>
    {#if sub}<p class="muted">{sub}</p>{/if}
  </div>
  {#if children}<div class="toolbar">{@render children()}</div>{/if}
</div>
