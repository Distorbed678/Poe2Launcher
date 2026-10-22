<script lang="ts">
  import { app } from '../app.svelte';
  import { versionLabel, versionSub } from '../gameversion';
  import Dropdown, { type DropItem } from './Dropdown.svelte';
  import Icon from './Icon.svelte';

  const items = $derived<DropItem[]>(
    app.versions.map((v, i, all) => ({
      value: v.version,
      label: versionLabel(v),
      sub: versionSub(v),
      icon: v.official ? 'eye' : 'portal',
      heading: v.official ? 'Official servers' : all[i - 1]?.official || i === 0 ? 'Private server versions' : undefined,
      badges: [
        ...(v.official ? [{ text: 'read-only', tone: 'warn' as const }] : []),
        ...(v.newest ? [{ text: 'newest', tone: 'accent' as const }] : []),
        ...(v.installed ? [{ text: 'installed', tone: 'ok' as const }] : []),
      ],
    })),
  );
</script>

{#if app.versions.length > 0}
  <Dropdown
    {items}
    value={app.version}
    label="Game version"
    variant="title"
    disabled={app.switching}
    minWidth={340}
    onselect={(v) => void app.switchVersion(v)}
  >
    {#snippet trigger(cur)}
      <span class="version-trigger">
        <span class="vt-icon" class:official={app.official}><Icon name={app.official ? 'eye' : 'portal'} size={16} /></span>
        <span class="vt-text">
          <small>Game version</small>
          <strong>{cur?.label ?? versionLabel(app.version)}</strong>
        </span>
      </span>
    {/snippet}
    {#snippet footer()}
      Everything follows this choice: install folder, files, play, server list. More versions appear here as the project supports them.
    {/snippet}
  </Dropdown>
{/if}
