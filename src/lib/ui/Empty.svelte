<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon, { type IconName } from './Icon.svelte';
  let { icon, title, children, action }: { icon: IconName; title: string; children?: Snippet; action?: Snippet } = $props();
</script>

<div class="empty-state">
  <span class="empty-icon"><Icon name={icon} size={28} /></span>
  <h3>{title}</h3>
  {#if children}<p class="muted">{@render children()}</p>{/if}
  {#if action}<div class="empty-action">{@render action()}</div>{/if}
</div>
