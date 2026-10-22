<script lang="ts">
  /** A progress bar; `value` is 0..100, `null` is indeterminate. `active` adds the moving shine. */
  let { value, active = true, label = '' }: { value: number | null; active?: boolean; label?: string } = $props();
  const pct = $derived(value === null ? 0 : Math.max(0, Math.min(100, value)));
</script>

<div
  class="progress"
  class:indeterminate={value === null}
  class:active
  role="progressbar"
  aria-label={label || undefined}
  aria-valuemin="0"
  aria-valuemax="100"
  aria-valuenow={value === null ? undefined : Math.round(pct)}
>
  <div class="fill" style:width={value === null ? undefined : `${pct}%`}></div>
</div>
