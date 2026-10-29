<script lang="ts">
  import { app } from '../app.svelte';
  import { currentStep, hookReady, setupNeeded, setupSteps } from '../hook';
  import Icon from './Icon.svelte';
  import Chip from './Chip.svelte';
  import Spinner from './Spinner.svelte';

  // The official mode has no server of its own to host or join.
  const steps = $derived(
    setupSteps({ gameInstalled: app.status?.installed ?? false, hookReady: hookReady(app.hook), profiles: app.profiles.length }).filter(
      (s) => !app.official || s.id !== 'server',
    ),
  );
  const now = $derived(currentStep(steps));
  const show = $derived(setupNeeded(steps));
  const installingHook = $derived(app.hookTask === 'install');

  let chooser: HTMLDialogElement | undefined = $state();
  function choose(href: string) {
    chooser?.close();
    location.hash = href;
  }
</script>

{#if show}
  <section class="card setup" aria-labelledby="setup-title">
    <div class="card-head">
      <span class="head-icon"><Icon name="check" size={16} /></span>
      <h2 id="setup-title">Get ready to play</h2>
      <span class="grow"></span>
      <span class="muted">{steps.filter((s) => s.done).length} of {steps.length} done</span>
    </div>
    <ol class="steps">
      {#each steps as s, i (s.id)}
        <li class:done={s.done} class:now={s.id === now}>
          <span class="mark" aria-hidden="true">{#if s.done}<Icon name="check" size={14} />{:else}{i + 1}{/if}</span>
          <div class="grow">
            <b>{s.title}</b>
            <div class="hint">{s.detail}</div>
          </div>
          <div class="step-actions">
            {#if s.done}
              <Chip tone="ok">done</Chip>
            {:else if s.id === 'game'}
              <a class="btn" class:primary={s.id === now} href="#/library"><Icon name="install" size={14} /> Open Library</a>
            {:else if s.id === 'hook'}
              <button type="button" class="btn" class:primary={s.id === now} disabled={app.hookTask !== ''} onclick={() => void app.installHook()}>
                {#if installingHook}<Spinner size={14} /> Downloading{:else}<Icon name="download" size={14} /> Download{/if}
              </button>
            {:else}
              <button type="button" class="btn" class:primary={s.id === now} onclick={() => chooser?.showModal()}><Icon name="servers" size={14} /> Set up a server</button>
            {/if}
          </div>
        </li>
      {/each}
    </ol>
    {#if app.hookError}<div class="notice danger" style="margin-top: 10px"><span class="notice-title"><Icon name="alert" size={16} /> Client hook</span><p>{app.hookError}</p></div>{/if}
  </section>

  <dialog bind:this={chooser} class="modal" aria-labelledby="chooser-title">
    <div class="dialog-body">
      <div class="dialog-head"><h2 id="chooser-title">Set up a server</h2></div>
      <p class="muted">Pick one. You can add more later on the Servers page.</p>
      <div class="choices">
        <button type="button" class="choice" onclick={() => choose('#/servers?tab=host&create=1')}>
          <span class="choice-icon"><Icon name="host" size={18} /></span>
          <span class="grow"><b>Host on this computer</b><small>Run your own server; friends join with its address.</small></span>
          <span class="btn sm primary">Create</span>
        </button>
        <button type="button" class="choice" onclick={() => choose('#/servers?tab=browse')}>
          <span class="choice-icon"><Icon name="compass" size={18} /></span>
          <span class="grow"><b>Browse public servers</b><small>Servers that list themselves publicly.</small></span>
          <span class="btn sm">Browse</span>
        </button>
        <button type="button" class="choice" onclick={() => choose('#/servers?add=1')}>
          <span class="choice-icon"><Icon name="plus" size={18} /></span>
          <span class="grow"><b>Add by address</b><small>A friend's server: its web address, e.g. 192.168.1.20:8080.</small></span>
          <span class="btn sm">Add</span>
        </button>
      </div>
      <div class="actions"><button type="button" class="btn ghost" onclick={() => chooser?.close()}>Cancel</button></div>
    </div>
  </dialog>
{/if}

<style>
  .setup { margin-bottom: 18px; }
  .steps { list-style: none; margin: 0; padding: 0; display: grid; gap: 8px; }
  .steps li { display: flex; align-items: center; gap: 12px; padding: 10px 12px; border-radius: var(--radius-sm); border: 1px solid var(--line); background: var(--bg-2); }
  .steps li.now { border-color: color-mix(in srgb, var(--accent) 55%, var(--line)); background: color-mix(in srgb, var(--accent) 7%, var(--surface)); }
  .steps li.done { opacity: 0.78; }
  .mark { flex: none; width: 26px; height: 26px; border-radius: 50%; display: grid; place-items: center; font-size: 13px; font-weight: 650; background: var(--surface-3); color: var(--muted); }
  li.now .mark { background: linear-gradient(135deg, var(--accent), var(--accent-2)); color: var(--accent-ink); }
  li.done .mark { background: color-mix(in srgb, var(--ok) 20%, transparent); color: var(--ok); }
  .step-actions { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; justify-content: flex-end; }
  a.btn { text-decoration: none; }
  .choices { display: grid; gap: var(--sp-3); margin: var(--sp-3) 0 var(--sp-2); }
  .choice { display: flex; align-items: center; gap: var(--sp-4); padding: var(--sp-4); border: 1px solid var(--line); border-radius: var(--radius-lg); background: var(--bg-2); color: var(--text); text-align: left; cursor: pointer; font: inherit; transition: border-color var(--dur) var(--ease), background var(--dur) var(--ease); }
  .choice:hover, .choice:focus-visible { border-color: color-mix(in srgb, var(--gold) 55%, var(--line)); background: color-mix(in srgb, var(--crimson) 14%, var(--bg-2)); }
  .choice b { display: block; font-weight: 600; }
  .choice small { color: var(--muted); font-size: var(--fs-sm); }
  .choice-icon { flex: none; width: 38px; height: 38px; border-radius: var(--radius-lg); background: var(--surface-3); display: grid; place-items: center; color: var(--gold-2); }
</style>
