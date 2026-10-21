<script lang="ts">
  import type { Snippet } from 'svelte';
  let { title, sub, children }: { title: string; sub?: string; children?: Snippet } = $props();
</script>

<div class="page-head">
  <div>
    <h1>{title}</h1>
    {#if sub}<p class="muted">{sub}</p>{/if}
  </div>
  {#if children}<div class="toolbar">{@render children()}</div>{/if}
</div>
