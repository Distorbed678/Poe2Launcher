<script lang="ts">
  let { size = 16, label = 'Working' }: { size?: number; label?: string } = $props();
</script>

<span class="spinner" style:width="{size}px" style:height="{size}px" role="status" aria-label={label}></span>
