<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon, { type IconName } from './Icon.svelte';
  let { title, sub, icon, children }: { title: string; sub?: string; icon?: IconName; children?: Snippet } = $props();
</script>

<div class="page-head">
  <div class="page-title">
    {#if icon}<span class="page-icon"><Icon name={icon} size={20} /></span>{/if}
    <div>
      <h1>{title}</h1>
      {#if sub}<p class="muted">{sub}</p>{/if}
    </div>
  </div>
  {#if children}<div class="toolbar">{@render children()}</div>{/if}
</div>
