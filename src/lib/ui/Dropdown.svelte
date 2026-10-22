<script lang="ts" module>
  export interface DropItem {
    value: string;
    label: string;
    sub?: string;
    badges?: { text: string; tone?: 'neutral' | 'ok' | 'warn' | 'bad' | 'info' | 'accent' }[];
    icon?: IconName;
    /** A small heading shown above this entry. */
    heading?: string;
    disabled?: boolean;
  }
  import type { IconName } from './Icon.svelte';
</script>

<script lang="ts">
  import type { Snippet } from 'svelte';
  import { tick } from 'svelte';
  import Icon from './Icon.svelte';
  import Chip from './Chip.svelte';

  let {
    items,
    value,
    onselect,
    label,
    placement = 'down',
    align = 'start',
    disabled = false,
    variant = 'field',
    trigger,
    footer,
    minWidth,
  }: {
    items: DropItem[];
    value: string;
    onselect: (value: string) => void;
    label: string;
    placement?: 'down' | 'up';
    align?: 'start' | 'end';
    disabled?: boolean;
    variant?: 'field' | 'title';
    trigger?: Snippet<[DropItem | undefined]>;
    footer?: Snippet;
    minWidth?: number;
  } = $props();

  const uid = `dd-${Math.random().toString(36).slice(2, 8)}`;
  let open = $state(false);
  let active = $state(0);
  let root: HTMLElement | undefined = $state();
  let btn: HTMLButtonElement | undefined = $state();
  let list: HTMLElement | undefined = $state();

  const current = $derived(items.find((i) => i.value === value));

  async function show() {
    if (disabled) return;
    open = true;
    active = Math.max(0, items.findIndex((i) => i.value === value));
    await tick();
    list?.focus();
  }

  function close(refocus = true) {
    open = false;
    if (refocus) btn?.focus();
  }

  function choose(i: DropItem) {
    if (i.disabled) return;
    close();
    if (i.value !== value) onselect(i.value);
  }

  function step(delta: number) {
    const n = items.length;
    for (let k = 1; k <= n; k++) {
      const j = (active + delta * k + n * k) % n;
      if (!items[j]?.disabled) {
        active = j;
        break;
      }
    }
    void tick().then(() => list?.querySelector('[data-active="true"]')?.scrollIntoView({ block: 'nearest' }));
  }

  function key(e: KeyboardEvent) {
    switch (e.key) {
      case 'ArrowDown': e.preventDefault(); step(1); break;
      case 'ArrowUp': e.preventDefault(); step(-1); break;
      case 'Home': e.preventDefault(); active = 0; break;
      case 'End': e.preventDefault(); active = items.length - 1; break;
      case 'Enter': case ' ': e.preventDefault(); { const i = items[active]; if (i) choose(i); } break;
      case 'Escape': e.preventDefault(); close(); break;
      case 'Tab': close(false); break;
    }
  }

  function outside(e: PointerEvent) {
    if (open && root && !root.contains(e.target as Node)) close(false);
  }
</script>

<svelte:window onpointerdown={outside} />

<div class="dd {variant}" class:open bind:this={root}>
  <button
    type="button"
    class="dd-trigger"
    bind:this={btn}
    {disabled}
    aria-haspopup="listbox"
    aria-expanded={open}
    aria-controls={open ? uid : undefined}
    aria-label={label}
    onclick={() => (open ? close() : show())}
    onkeydown={(e) => {
      if (!open && (e.key === 'ArrowDown' || e.key === 'ArrowUp')) {
        e.preventDefault();
        void show();
      }
    }}
  >
    {#if trigger}
      {@render trigger(current)}
    {:else}
      <span class="dd-label">{current?.label ?? 'Choose...'}</span>
    {/if}
    <Icon name="chevron" size={16} class="dd-chevron" />
  </button>
  {#if open}
    <div class="dd-menu {placement} {align}" style:min-width={minWidth ? `${minWidth}px` : undefined}>
      <ul
        id={uid}
        class="dd-list"
        role="listbox"
        tabindex="-1"
        aria-label={label}
        aria-activedescendant={`${uid}-${active}`}
        bind:this={list}
        onkeydown={key}
      >
        {#each items as it, i (it.value)}
          {#if it.heading}<li class="dd-heading" role="presentation">{it.heading}</li>{/if}
          <li
            id={`${uid}-${i}`}
            role="option"
            aria-selected={it.value === value}
            aria-disabled={it.disabled || undefined}
            data-active={i === active}
            class="dd-item"
            class:selected={it.value === value}
            onpointerenter={() => (active = i)}
            onclick={() => choose(it)}
            onkeydown={null}
          >
            {#if it.icon}<span class="dd-icon"><Icon name={it.icon} size={16} /></span>{/if}
            <span class="dd-text">
              <span class="dd-name">{it.label}</span>
              {#if it.sub}<span class="dd-sub">{it.sub}</span>{/if}
            </span>
            {#each it.badges ?? [] as b (b.text)}<Chip tone={b.tone ?? 'neutral'}>{b.text}</Chip>{/each}
            {#if it.value === value}<Icon name="check" size={15} class="dd-check" />{/if}
          </li>
        {/each}
      </ul>
      {#if footer}<div class="dd-footer">{@render footer()}</div>{/if}
    </div>
  {/if}
</div>
