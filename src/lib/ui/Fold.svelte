<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';

  // A titled section that folds open and shut with an arrow (server settings, Library details).
  // `summary` is the one-line state shown while it is shut.
  let {
    title,
    summary = '',
    open = $bindable(false),
    danger = false,
    children,
  }: { title: string; summary?: string; open?: boolean; danger?: boolean; children: Snippet } = $props();
</script>

<details class="fold" class:danger bind:open>
  <summary>
    <span class="arrow" aria-hidden="true"><Icon name="chevron" size={14} /></span>
    <span class="fold-title">{title}</span>
    {#if summary && !open}<span class="fold-summary">{summary}</span>{/if}
  </summary>
  <div class="fold-body">{@render children()}</div>
</details>

<style>
  .fold { border: 1px solid var(--line); border-radius: var(--radius-lg); background: var(--bg-2); }
  .fold.danger { border-color: color-mix(in srgb, var(--danger) 40%, var(--line)); }
  summary {
    display: flex; align-items: center; gap: var(--sp-3); padding: 14px var(--sp-4); cursor: pointer; list-style: none;
    font: 600 12.5px/1.2 var(--font-head); letter-spacing: var(--tracking-head); text-transform: uppercase; color: var(--text);
  }
  summary::-webkit-details-marker { display: none; }
  summary:hover .fold-title { color: var(--gold-2); }
  .danger summary, .danger summary:hover .fold-title { color: var(--danger); }
  .arrow { display: grid; place-items: center; color: var(--gold); transform: rotate(-90deg); transition: transform var(--dur) var(--ease); }
  .danger .arrow { color: var(--danger); }
  details[open] > summary .arrow { transform: none; }
  .fold-summary { margin-left: auto; font: 400 var(--fs-sm) / 1.3 var(--font); letter-spacing: 0; text-transform: none; color: var(--faint); text-align: right; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
  .fold-body { padding: var(--sp-2) var(--sp-5) var(--sp-5); border-top: 1px solid var(--line-soft); }
</style>
