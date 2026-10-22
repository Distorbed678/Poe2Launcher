<script lang="ts">
  import { frame, type Dir } from '../frame';
  const handles: { cls: string; dir: Dir }[] = [
    { cls: 'n', dir: 'North' }, { cls: 's', dir: 'South' }, { cls: 'e', dir: 'East' }, { cls: 'w', dir: 'West' },
    { cls: 'ne', dir: 'NorthEast' }, { cls: 'nw', dir: 'NorthWest' }, { cls: 'se', dir: 'SouthEast' }, { cls: 'sw', dir: 'SouthWest' },
  ];
</script>

{#each handles as h (h.cls)}
  <div class="rz {h.cls}" role="presentation" onpointerdown={(e) => { if (e.button === 0) { e.preventDefault(); frame.resize(h.dir); } }}></div>
{/each}
