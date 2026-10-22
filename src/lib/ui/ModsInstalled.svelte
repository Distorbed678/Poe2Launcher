<script lang="ts">
  import { open } from '@tauri-apps/plugin-dialog';
  import { fly, fade } from 'svelte/transition';
  import { flip } from 'svelte/animate';
  import { cubicOut } from 'svelte/easing';
  import { app } from '../app.svelte';
  import { errorText, ipc } from '../ipc';
  import { run, toasts } from '../notify.svelte';
  import { ask } from '../confirm.svelte';
  import { formatBytes } from '../format';
  import { ms } from '../motion';
  import { SCOPE_ALL, blocksEnable, groupMods, isValid, originLabel, overridable, sideLabel, stepTarget, syncStateLabel, compatLabel } from '../mods';
  import Icon from './Icon.svelte';
  import Chip from './Chip.svelte';
  import Empty from './Empty.svelte';
  import type { ModEntry, ModsView, SyncMod } from '../types';

  let { data, scope, reload, openBrowse }: { data: ModsView | null; scope: string; reload: () => Promise<void>; openBrowse: () => void } = $props();

  const groups = $derived(data ? groupMods(data.entries) : []);
  const isAll = $derived(scope === SCOPE_ALL);
  const missingRequired = $derived((data?.required ?? []).filter((m) => m.state === 'missing' || m.state === 'changed'));
  /** The version the row works on: the one the player picked, else the enabled one, else the newest. */
  let picked = $state<Record<string, string>>({});
  function current(g: (typeof groups)[number]): ModEntry {
    const want = picked[g.id];
    return g.versions.find((v) => v.version === want) ?? g.enabled ?? g.inEffect ?? g.versions[0];
  }

  async function install(path: string | null) {
    if (!path) return;
    let r;
    try {
      r = await ipc.modInstall(path, scope, false);
    } catch (e) {
      const msg = errorText(e);
      if (msg.includes('already cached') && (await ask({ title: 'Replace the cached copy?', message: msg, confirmLabel: 'Replace', danger: true }))) {
        r = await run(() => ipc.modInstall(path, scope, true));
      } else {
        toasts.show('error', msg);
      }
    }
    if (r) {
      toasts.show('ok', r.already_cached ? `${r.id} ${r.version} was already cached` : `Added ${r.id} ${r.version}${r.server_half_left_out ? ' (client half only)' : ''}`);
      for (const w of r.warnings) toasts.show('ok', w);
    }
    await reload();
  }

  async function pickFolder() {
    const p = await open({ directory: true, title: 'Mod folder (with mod.toml)' });
    await install(typeof p === 'string' ? p : null);
  }

  async function pickZip() {
    const p = await open({ filters: [{ name: 'Mod package', extensions: ['zip'] }], title: 'Mod package (<id>-<version>.zip)' });
    await install(typeof p === 'string' ? p : null);
  }

  async function setEnabled(e: ModEntry, on: boolean, forScope = scope) {
    let force = false;
    if (on && blocksEnable(e.compat)) {
      if (!overridable(e.compat)) {
        toasts.show('error', `${e.id} ${e.version} cannot be enabled: ${e.compat_text}`);
        await reload();
        return;
      }
      if (!(await ask({ title: `Enable ${e.id} anyway?`, message: `It is ${compatLabel(e.compat).text}. The game or the mod may not work.`, confirmLabel: 'Enable anyway', danger: true }))) {
        await reload();
        return;
      }
      force = true;
    }
    if (on && e.has_code) {
      // Code is a DLL in the game process; say it once per enabling, not per Play.
      const name = forScope === SCOPE_ALL ? 'every server' : (app.profiles.find((p) => p.id === forScope)?.name ?? 'this server');
      if (!(await ask({ title: `Enable ${e.id} for ${name}?`, message: `${e.id} contains a DLL that runs inside the game process with your user's rights. Enable it only if you trust where it came from.`, confirmLabel: 'Enable' }))) {
        await reload();
        return;
      }
    }
    const notes = await run(() => ipc.modSetEnabled(forScope, e.id, e.version, on, force));
    for (const n of notes ?? []) toasts.show('error', n);
    await reload();
  }

  async function step(e: ModEntry, delta: -1 | 1) {
    if (!data) return;
    const to = stepTarget(data.entries, e.id, delta);
    if (to === null) return;
    await run(() => ipc.modMove(scope, e.id, to));
    await reload();
  }

  async function remove(e: ModEntry) {
    if (!(await ask({ title: `Remove ${e.id} ${e.version}?`, message: 'The cached copy is deleted and the mod is switched off everywhere. A server that requires it makes the launcher download it again.', confirmLabel: 'Remove', danger: true }))) return;
    await run(() => ipc.modRemove(e.id, e.version), 'Removed');
    await reload();
  }

  async function fetchRequired(m?: SyncMod) {
    if (isAll) return;
    const plan = await run(() => ipc.modSyncApply(scope, m ? [m.id] : null));
    if (plan) {
      app.modPlans[scope] = plan;
      for (const err of plan.errors) toasts.show('error', err);
      if (plan.applied.length) toasts.show('ok', `Downloaded and verified: ${plan.applied.join(', ')}`);
    }
    await reload();
  }
</script>

<div class="toolbar tb">
  <button type="button" class="btn" onclick={pickFolder}><Icon name="folder" size={14} /> Add folder</button>
  <button type="button" class="btn" onclick={pickZip}><Icon name="box" size={14} /> Add package (.zip)</button>
  <button type="button" class="btn" onclick={async () => { await run(() => ipc.modAutoOrder(scope), 'Load order resolved'); await reload(); }}><Icon name="refresh" size={14} /> Auto order</button>
  <button type="button" class="btn primary" onclick={openBrowse}><Icon name="search" size={14} /> Find mods</button>
</div>

{#if data}
  {#if !isAll && data.required.length > 0}
    <section class="card req" aria-label="Required by the server">
      <div class="card-head"><span class="head-icon"><Icon name="shield" size={16} /></span><h2>Required by {data.scope_name}</h2>
        <span class="grow"></span>
        {#if missingRequired.length}
          <button type="button" class="btn sm primary" onclick={() => fetchRequired()}><Icon name="download" size={13} /> Download {formatBytes(missingRequired.reduce((n, m) => n + m.size, 0))}</button>
        {/if}
      </div>
      <ul class="reqlist">
        {#each data.required as m (m.id)}
          {@const st = syncStateLabel(m.state)}
          {@const side = sideLabel(m.side)}
          <li>
            <strong>{m.name}</strong> <span class="muted">{m.version}</span>
            <Chip tone={side.tone}>{side.text}</Chip>
            {#if m.code}<Chip tone="warn" icon="alert">code</Chip>{/if}
            <Chip tone={st.tone}>{st.text}</Chip>
            {#if m.state !== 'ready'}<span class="hint">{m.detail}</span>{/if}
          </li>
        {/each}
      </ul>
      <p class="hint">What the server last said. It is checked when you select the server and again at Play; these mods are always on for this server.</p>
    </section>
  {/if}

  {#each data.problems as p (p)}
    <div class="notice danger"><span class="notice-title"><Icon name="alert" size={16} /> {p}</span></div>
  {/each}
  {#each data.dropped as d (d)}
    <div class="notice info"><span class="notice-title"><Icon name="info" size={16} /> {d}</span></div>
  {/each}

  {#if groups.length === 0}
    <Empty icon="mods" title="No mods in the cache">
      Find mods in the browser, add a package (<code>&lt;id&gt;-&lt;version&gt;.zip</code>) or a mod folder, or join a server that requires some: the launcher downloads those itself.
      {#snippet action()}
        <div class="row" style="justify-content: center">
          <button type="button" class="btn primary" onclick={openBrowse}><Icon name="search" size={14} /> Find mods</button>
          <button type="button" class="btn" onclick={pickZip}><Icon name="box" size={14} /> Add package</button>
        </div>
      {/snippet}
    </Empty>
  {:else}
    <ul class="modlist">
      {#each groups as g (g.id)}
        {@const e = current(g)}
        {@const side = sideLabel(e.side)}
        {@const lab = compatLabel(e.compat)}
        <li in:fly={{ y: 12, duration: ms(240), easing: cubicOut }} out:fade={{ duration: ms(140) }} animate:flip={{ duration: ms(220) }} class:off={!e.in_effect && !e.enabled}>
          <input
            type="checkbox"
            checked={e.enabled || e.required}
            disabled={!isValid(e) || e.required}
            aria-label="Enable {e.id} for {data.scope_name}"
            onchange={(ev) => setEnabled(e, ev.currentTarget.checked)}
          />
          <div>
            <strong>{e.manifest?.name ?? e.id}</strong>
            {#if g.versions.length > 1}
              <select class="ver" aria-label="Version of {e.id}" value={e.version} onchange={(ev) => (picked[g.id] = ev.currentTarget.value)}>
                {#each g.versions as v (v.version)}<option value={v.version}>{v.version}{v.enabled ? ' (on)' : ''}</option>{/each}
              </select>
            {:else}
              <span class="muted">{e.version}</span>
            {/if}
            <Chip tone={side.tone}><span title={side.title}>{side.text}</span></Chip>
            {#if e.has_code}<Chip tone="warn" icon="alert">code</Chip>{/if}
            {#if e.in_effect}<Chip tone="ok">#{(e.order ?? 0) + 1}</Chip>{/if}
            {#if e.manifest}<Chip tone={lab.tone === 'ok' ? 'ok' : lab.tone === 'bad' ? 'bad' : 'warn'}>{lab.text}</Chip>{/if}
            {#if e.in_effect && e.origin}<Chip tone={e.origin === 'required' ? 'accent' : 'neutral'}>{#if e.origin === 'required'}<Icon name="shield" size={11} /> {/if}{originLabel(e.origin)}</Chip>{/if}
            <div class="meta">
              <code>{e.id}</code>
              {#if e.manifest?.description} - {e.manifest.description}{/if}
              {#if e.manifest && Object.keys(e.manifest.dependencies).length}
                - needs {Object.entries(e.manifest.dependencies).map(([k, v]) => `${k} ${v}`).join(', ')}
              {/if}
              <span class="faint"> - {formatBytes(e.size)}, from {e.source}{e.server_half_left_out ? ', client half only' : ''}</span>
            </div>
            {#if e.errors.length}<ul class="errs">{#each e.errors as er}<li>{er}</li>{/each}</ul>{/if}
          </div>
          <div class="row">
            {#if !isAll && isValid(e) && !e.required}
              <label class="allsw" title="Also load it on every server"><input type="checkbox" checked={e.enabled_all} onchange={(ev) => setEnabled(e, ev.currentTarget.checked, SCOPE_ALL)} /> all servers</label>
            {/if}
            {#if e.enabled}
              <button type="button" class="btn icon sm ghost" title="Earlier" aria-label="Move earlier" disabled={stepTarget(data.entries, e.id, -1) === null} onclick={() => step(e, -1)}><Icon name="up" size={15} /></button>
              <button type="button" class="btn icon sm ghost" title="Later" aria-label="Move later" disabled={stepTarget(data.entries, e.id, 1) === null} onclick={() => step(e, 1)}><Icon name="down" size={15} /></button>
            {/if}
            <button type="button" class="btn sm danger" aria-label="Remove {e.id} {e.version} from the cache" onclick={() => remove(e)}><Icon name="trash" size={13} /> Remove</button>
          </div>
        </li>
      {/each}
    </ul>
  {/if}
  <p class="hint cachehint">Cache: <code>{data.cache_dir}</code> ({formatBytes(data.cache_bytes)}). Playing copies the set into the game's <code>poe-hook-mods</code> folder; the official (read-only) entry never loads mods.</p>
{/if}

<style>
  .tb { margin-bottom: 14px; }
  .req { margin-bottom: 14px; }
  .reqlist { list-style: none; margin: 0 0 8px; padding: 0; display: grid; gap: 8px; }
  .reqlist li { display: flex; align-items: center; flex-wrap: wrap; gap: 6px 8px; }
  .notice { margin-bottom: 10px; }
  .ver { min-height: 26px; padding: 0 6px; width: auto; font-size: 12.5px; }
  .off { opacity: 0.85; }
  .allsw { display: inline-flex; align-items: center; gap: 6px; font-size: 12px; color: var(--muted); cursor: pointer; }
  .cachehint { margin-top: 16px; }
  .faint { color: var(--faint); }
</style>
