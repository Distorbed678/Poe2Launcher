<script lang="ts">
  import { app } from '../lib/app.svelte';
  import { versionLabel } from '../lib/gameversion';
  import iconUrl from '../assets/icon.png';
  import PageHead from '../lib/ui/PageHead.svelte';
  import Icon from '../lib/ui/Icon.svelte';
  import Chip from '../lib/ui/Chip.svelte';

  /** `embedded`: shown inside Settings (Updates & About), without its own page title. */
  let { embedded = false }: { embedded?: boolean } = $props();
</script>

{#if !embedded}<PageHead icon="info" title="About" />{/if}

<section class="card about-hero">
  <img src={iconUrl} alt="" />
  <div>
    <h1>Poe2 Launcher</h1>
    <p class="muted">Unofficial launcher for the Path of Exile 2 private server.</p>
    <div class="row">
      <Chip tone="accent">launcher {app.info?.launcher_version}</Chip>
      <Chip tone={app.official ? 'warn' : 'info'} icon={app.official ? 'eye' : 'portal'}>{versionLabel(app.version)}</Chip>
    </div>
  </div>
</section>

<div class="two about-two">
  <section class="card">
    <div class="card-head"><span class="head-icon"><Icon name="portal" size={16} /></span><h2>Supported game versions</h2></div>
    <ul class="depot-list">
      {#each app.versions as v (v.version)}
        <li>
          <b>{versionLabel(v)}</b>
          <span class="grow"></span>
          {#if v.official}<Chip tone="warn">read-only</Chip>{/if}
          {#if v.newest}<Chip tone="accent">newest</Chip>{/if}
          {#if v.installed}<Chip tone="ok" icon="check">installed</Chip>{:else}<Chip>not installed</Chip>{/if}
          {#if v.selected}<Chip tone="info">selected</Chip>{/if}
        </li>
      {/each}
    </ul>
  </section>
  <section class="card">
    <div class="card-head"><span class="head-icon"><Icon name="folder" size={16} /></span><h2>Where things are</h2></div>
    <dl class="kv">
      <dt>Config</dt><dd><code>{app.info?.config_dir}</code></dd>
      <dt>Data</dt><dd><code>{app.info?.data_dir}</code></dd>
    </dl>
  </section>
</div>

<section class="card about-disclaimer">
  <div class="card-head"><span class="head-icon"><Icon name="shield" size={16} /></span><h2>Disclaimer</h2></div>
  <p class="hint">
    This is an unofficial, fan-made project for personal and educational use. It is <b>not affiliated with, endorsed by or connected to Grinding Gear Games</b>;
    "Path of Exile" is their trademark. No game files are distributed: the Library downloads the game from Steam with <b>your</b> Steam account, so you need your own legal copy.
    The client hook that the launcher deploys can be detected as a modification by anti-cheat software; use is at your own risk.
  </p>
</section>

<style>
  .about-two, .about-disclaimer { margin-top: var(--sp-4); }
</style>
