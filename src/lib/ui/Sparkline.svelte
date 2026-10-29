<script lang="ts">
  // A tiny line chart of the last readings (Manage > Overview usage tiles). No axes: the tile
  // shows the current value; this only shows the trend.
  let { values, color = 'var(--gold)', max }: { values: number[]; color?: string; max?: number } = $props();

  const points = $derived.by(() => {
    if (values.length < 2) return '';
    const top = max ?? Math.max(...values, 1e-9);
    const bottom = max === undefined ? Math.min(...values) : 0;
    const span = top - bottom || 1;
    return values
      .map((v, i) => `${((i / (values.length - 1)) * 100).toFixed(2)},${(24 - ((v - bottom) / span) * 22).toFixed(2)}`)
      .join(' ');
  });
</script>

<svg class="spark" viewBox="0 0 100 26" preserveAspectRatio="none" aria-hidden="true">
  {#if points}<polyline fill="none" stroke={color} stroke-width="1.5" vector-effect="non-scaling-stroke" points={points} />{/if}
</svg>

<style>
  .spark { display: block; width: 100%; height: 26px; margin-top: var(--sp-2); }
</style>
