<script lang="ts">
  import { parseMarkdown, type Inline } from '../markdown';
  import { ipc } from '../ipc';
  import { run } from '../notify.svelte';

  /** Markdown subset, rendered as plain text nodes: no HTML from the source is ever interpreted. */
  let { source, compact = false }: { source: string; compact?: boolean } = $props();
  const blocks = $derived(parseMarkdown(source));

  function open(e: Event, href: string) {
    e.preventDefault();
    void run(() => ipc.openLink(href));
  }
</script>

{#snippet inl(nodes: Inline[])}
  {#each nodes as n, i (i)}
    {#if n.t === 'text'}{n.s}
    {:else if n.t === 'b'}<strong>{@render inl(n.c)}</strong>
    {:else if n.t === 'i'}<em>{@render inl(n.c)}</em>
    {:else if n.t === 'code'}<code>{n.s}</code>
    {:else}<a href={n.href} onclick={(e) => open(e, n.href)} title={n.href}>{@render inl(n.c)}</a>{/if}
  {/each}
{/snippet}

<div class="md" class:compact>
  {#each blocks as b, i (i)}
    {#if b.t === 'p'}<p>{@render inl(b.c)}</p>
    {:else if b.t === 'h'}<p class="mdh h{b.level}">{@render inl(b.c)}</p>
    {:else if b.t === 'ul'}<ul>{#each b.items as it, j (j)}<li>{@render inl(it)}</li>{/each}</ul>
    {:else if b.t === 'ol'}<ol>{#each b.items as it, j (j)}<li>{@render inl(it)}</li>{/each}</ol>
    {:else if b.t === 'quote'}<blockquote>{@render inl(b.c)}</blockquote>
    {:else}<pre>{b.s}</pre>{/if}
  {/each}
</div>

<style>
  .md { font-size: 13.5px; line-height: 1.55; overflow-wrap: anywhere; }
  .md :global(p) { margin: 0 0 8px; }
  .md > :last-child { margin-bottom: 0; }
  .mdh { font-weight: 650; color: var(--text); }
  .h1 { font-size: 16px; } .h2 { font-size: 14.5px; } .h3 { font-size: 13.5px; }
  ul, ol { margin: 0 0 8px; padding-left: 20px; }
  li + li { margin-top: 2px; }
  blockquote { margin: 0 0 8px; padding: 4px 12px; border-left: 3px solid var(--line); color: var(--muted); }
  pre { margin: 0 0 8px; padding: 10px 12px; background: var(--bg-2); border: 1px solid var(--line-soft); border-radius: 8px; font: 12px/1.5 var(--mono); overflow: auto; white-space: pre-wrap; }
  .compact { font-size: 12.5px; }
</style>
