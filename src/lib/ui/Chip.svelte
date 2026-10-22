<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon, { type IconName } from './Icon.svelte';
  let {
    tone = 'neutral',
    icon,
    dot = false,
    children,
  }: { tone?: 'neutral' | 'ok' | 'warn' | 'bad' | 'info' | 'accent'; icon?: IconName; dot?: boolean; children?: Snippet } = $props();
</script>

<span class="chip {tone}">
  {#if dot}<i class="dot"></i>{/if}
  {#if icon}<Icon name={icon} size={12} />{/if}
  {@render children?.()}
</span>
