<script lang="ts">
  import { frame } from '../frame';
  import Icon from './Icon.svelte';

  // Only the frameless window has this bar: a drag strip with the window buttons.
  // The brand, the version and the update link live in the sidebar.
  let { maximized }: { maximized: boolean } = $props();
</script>

<header class="titlebar" data-tauri-drag-region>
  <div class="drag" data-tauri-drag-region></div>
  <div class="winbtns">
    <button type="button" class="winbtn" aria-label="Minimize" title="Minimize" onclick={frame.minimize}><Icon name="minimize" size={15} /></button>
    <button type="button" class="winbtn" aria-label={maximized ? 'Restore' : 'Maximize'} title={maximized ? 'Restore' : 'Maximize'} onclick={frame.toggleMaximize}>
      <Icon name={maximized ? 'restore' : 'maximize'} size={14} />
    </button>
    <button type="button" class="winbtn close" aria-label="Close" title="Close" onclick={frame.close}><Icon name="close" size={16} /></button>
  </div>
</header>
