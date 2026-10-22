<script lang="ts">
  import Icon, { type IconName } from './Icon.svelte';

  export interface Tab { id: string; label: string; icon?: IconName; badge?: string }
  let { tabs, value, onchange, label = 'Sections' }: { tabs: Tab[]; value: string; onchange: (id: string) => void; label?: string } = $props();

  function key(e: KeyboardEvent, i: number) {
    const d = e.key === 'ArrowRight' ? 1 : e.key === 'ArrowLeft' ? -1 : 0;
    if (!d) return;
    e.preventDefault();
    const next = tabs[(i + d + tabs.length) % tabs.length];
    if (next) onchange(next.id);
  }
</script>

<div class="seg tabs" role="tablist" aria-label={label}>
  {#each tabs as t, i (t.id)}
    <button type="button" role="tab" class:on={value === t.id} aria-selected={value === t.id} tabindex={value === t.id ? 0 : -1} onclick={() => onchange(t.id)} onkeydown={(e) => key(e, i)}>
      {#if t.icon}<Icon name={t.icon} size={15} />{/if}
      {t.label}
      {#if t.badge}<span class="tab-badge">{t.badge}</span>{/if}
    </button>
  {/each}
</div>

<style>
  .tabs button { display: inline-flex; align-items: center; gap: 7px; }
  .tab-badge { font-size: 11px; padding: 0 6px; border-radius: 99px; background: var(--surface-3); color: var(--muted); font-variant-numeric: tabular-nums; }
  .on .tab-badge { background: color-mix(in srgb, var(--accent) 20%, transparent); color: var(--accent); }
</style>
