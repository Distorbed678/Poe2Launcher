<script lang="ts">
  import { ipc } from '../ipc';
  import { run } from '../notify.svelte';
  import { plainHttpWarning } from '../url';
  import Icon from './Icon.svelte';
  import type { ProfileView } from '../types';

  let { profile, onchange }: { profile: ProfileView; onchange: () => void | Promise<void> } = $props();

  const text = $derived(plainHttpWarning(profile.plain_http));
  const open = $derived(text !== null && !profile.plain_http_confirmed);

  async function allow() {
    if (await run(() => ipc.confirmPlainHttp(profile.id, true), 'Plain http allowed for this server')) await onchange();
  }
</script>

{#if open}
  <div class="notice" role="alert">
    <span class="notice-title"><Icon name="alert" size={16} /> Unencrypted connection</span>
    <p>{text}</p>
    <p class="hint">Nothing is sent to this server (no login, no token) until you agree, and the agreement is for this address only.</p>
    <div><button type="button" class="btn sm" onclick={allow}>I understand, allow plain http for this server</button></div>
  </div>
{/if}
