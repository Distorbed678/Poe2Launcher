<script lang="ts">
  import type { Snippet } from 'svelte';
  let {
    checked = $bindable(false),
    disabled = false,
    onchange,
    children,
  }: { checked?: boolean; disabled?: boolean; onchange?: (v: boolean) => void; children?: Snippet } = $props();
</script>

<label class="switch" class:disabled>
  <input
    type="checkbox"
    role="switch"
    bind:checked
    {disabled}
    onchange={(e) => onchange?.(e.currentTarget.checked)}
  />
  <span class="track"><span class="thumb"></span></span>
  {#if children}<span class="switch-text">{@render children()}</span>{/if}
</label>
