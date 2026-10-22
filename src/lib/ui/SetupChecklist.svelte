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
              <a class="btn" class:primary={s.id === now} href="#/host"><Icon name="host" size={14} /> Host a server</a>
              <span class="hint">or</span>
              <a class="btn" href="#/servers?tab=browse"><Icon name="compass" size={14} /> Find a server</a>
              <a class="btn" href="#/servers"><Icon name="plus" size={14} /> Add by address</a>
            {/if}
          </div>
        </li>
      {/each}
    </ol>
    {#if app.hookError}<div class="notice danger" style="margin-top: 10px"><span class="notice-title"><Icon name="alert" size={16} /> Client hook</span><p>{app.hookError}</p></div>{/if}
  </section>
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
</style>
