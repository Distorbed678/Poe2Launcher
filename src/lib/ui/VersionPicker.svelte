<script lang="ts">
  import { app } from '../app.svelte';
  import { versionLabel } from '../gameversion';
  import Dropdown, { type DropItem } from './Dropdown.svelte';
  import Icon from './Icon.svelte';

  const items = $derived<DropItem[]>(
    app.versions.map((v, i, all) => ({
      value: v.version,
      label: versionLabel(v),
      // Short: the menu is as narrow as the sidebar. The long text is the row's tooltip.
      sub: v.official ? 'read-only' : [v.installed ? 'installed' : 'not installed', v.newest ? 'newest' : ''].filter(Boolean).join(' · '),
      icon: v.official ? 'eye' : 'portal',
      heading: v.official ? 'Official servers' : all[i - 1]?.official || i === 0 ? 'Private servers' : undefined,
    })),
  );
</script>

{#if app.versions.length > 0}
  <Dropdown
    {items}
    value={app.version}
    label="Game version"
    variant="attached"
    disabled={app.switching}
    onselect={(v) => void app.switchVersion(v)}
  >
    {#snippet trigger(cur)}
      <span class="version-trigger" title="Game version: {cur?.label ?? versionLabel(app.version)}">
        <span class="vt-icon" class:official={app.official}><Icon name={app.official ? 'eye' : 'portal'} size={15} /></span>
        <span class="vt-text">
          <small>Game version</small>
          <strong>{cur?.label ?? versionLabel(app.version)}</strong>
        </span>
      </span>
    {/snippet}
    {#snippet footer()}
      Install folder, files, servers and Play follow this choice.
    {/snippet}
  </Dropdown>
{/if}
