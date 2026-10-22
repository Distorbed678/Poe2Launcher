<script lang="ts">
  import { updater } from '../update.svelte';
  import { frame } from '../frame';
  import iconUrl from '../../assets/icon.png';
  import Icon from './Icon.svelte';
  import VersionPicker from './VersionPicker.svelte';

  let { custom, maximized }: { custom: boolean; maximized: boolean } = $props();
</script>

<header class="titlebar" data-tauri-drag-region>
  <div class="brand">
    <img src={iconUrl} alt="" />
    <span>Poe2 <b>Launcher</b></span>
  </div>
  <VersionPicker />
  <div class="drag" data-tauri-drag-region></div>
  {#if updater.state.kind === 'available'}
    <a class="updatelink" href="#/settings"><Icon name="download" size={13} /> Update {updater.state.version}</a>
  {/if}
  {#if custom && frame.available()}
    <div class="winbtns">
      <button type="button" class="winbtn" aria-label="Minimize" title="Minimize" onclick={frame.minimize}><Icon name="minimize" size={16} /></button>
      <button type="button" class="winbtn" aria-label={maximized ? 'Restore' : 'Maximize'} title={maximized ? 'Restore' : 'Maximize'} onclick={frame.toggleMaximize}>
        <Icon name={maximized ? 'restore' : 'maximize'} size={15} />
      </button>
      <button type="button" class="winbtn close" aria-label="Close" title="Close" onclick={frame.close}><Icon name="close" size={17} /></button>
    </div>
  {/if}
</header>
