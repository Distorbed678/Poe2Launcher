<script lang="ts">
  import { app } from '../lib/app.svelte';
  import { ipc } from '../lib/ipc';
  import { run } from '../lib/notify.svelte';
  import { formatBytes } from '../lib/format';
  import { isActive, promptText, validGuardCode } from '../lib/install';
  import PageHead from '../lib/ui/PageHead.svelte';

  let username = $state(app.settings?.steam_username ?? '');
  let password = $state('');
  let remember = $state(app.settings?.remember_steam_login ?? false);
  let code = $state('');
  let logEl = $state<HTMLElement | null>(null);

  const st = $derived(app.status);
  const ins = $derived(app.install);
  const active = $derived(isActive(ins.phase) || (st?.running ?? false));

  $effect(() => {
    void ins.log.length;
    if (logEl) logEl.scrollTop = logEl.scrollHeight;
  });

  async function start(validate: boolean) {
    if (app.settings) {
      const s = await run(() => ipc.setSettings({ ...app.settings!, steam_username: username.trim(), remember_steam_login: remember }));
      if (s) app.settings = s;
    }
    const ok = await run(() =>
      ipc.installStart({ steam_username: username, steam_password: password, remember_login: remember, validate }),
    );
    // The password has been handed over; do not keep it in the form.
    if (ok !== undefined) password = '';
  }

  async function sendCode(e: SubmitEvent) {
    e.preventDefault();
    const c = code.trim();
    if ((await run(() => ipc.installInput(c))) !== undefined) {
      app.applyInstall({ type: 'input_sent' });
      code = '';
    }
  }

  async function verify() {
    await run(() => ipc.installVerify());
  }

  const toolPct = $derived(ins.tool && ins.tool.total > 0 ? (ins.tool.done / ins.tool.total) * 100 : 0);
</script>

<PageHead title="Install" sub={st ? `Client ${st.version} (Steam app ${st.app_id}), into ${st.install_dir}` : ''}>
  {#if st?.installed}<span class="pill ok">installed</span>{:else}<span class="pill">not installed</span>{/if}
  <button type="button" onclick={verify} disabled={!st?.installed || active}>Verify files</button>
</PageHead>

<div class="two">
  <section class="card">
    <h2>Download from Steam</h2>
    <p class="hint">
      The launcher downloads the two depots with DepotDownloader {st?.tool_version ?? ''} using <b>your own Steam login</b>,
      into its own folder. It never touches the Steam install of the game. Your password is passed to the downloader once and is
      not stored.
    </p>
    <form
      onsubmit={(e) => {
        e.preventDefault();
        void start(false);
      }}
    >
      <label class="field"><span>Steam account name</span><input bind:value={username} autocomplete="username" disabled={active} /></label>
      <label class="field">
        <span>Steam password</span>
        <input type="password" bind:value={password} autocomplete="current-password" disabled={active} />
        {#if remember}<span class="hint">Not needed again once the Steam login is remembered; leave it empty then.</span>{/if}
      </label>
      <label class="check">
        <input type="checkbox" bind:checked={remember} disabled={active} />
        Remember the Steam login (DepotDownloader keeps a login token of its own, not your password)
      </label>
      <div class="actions">
        <button type="button" onclick={() => start(true)} disabled={active || !username.trim()} title="Check the files that are already there and fetch what differs">
          Repair
        </button>
        <button class="primary" type="submit" disabled={active || !username.trim()}>
          {st?.installed ? 'Download again' : 'Install'}
        </button>
        {#if active}<button class="danger" type="button" onclick={() => run(() => ipc.installCancel())}>Cancel</button>{/if}
      </div>
    </form>
    {#if st}
      <h2 style="margin-top: 14px">Depots (install order)</h2>
      <ol class="hint">
        {#each st.depots as d (d.id)}<li>{d.id}: manifest <code>{d.manifest}</code> ({d.role})</li>{/each}
      </ol>
    {/if}
  </section>

  <section class="card">
    <h2>Progress</h2>
    <p>
      {#if ins.phase === 'idle'}<span class="muted">Nothing running.</span>
      {:else if ins.phase === 'fetching_tool'}Fetching DepotDownloader...
      {:else if ins.phase === 'downloading'}Downloading from Steam...
      {:else if ins.phase === 'verifying'}Verifying the install...
      {:else if ins.phase === 'done'}<span class="pill ok">Done</span>
      {:else if ins.phase === 'cancelled'}<span class="pill warn">Cancelled</span>
      {:else}<span class="pill bad">Failed</span> <span class="err">{ins.message}</span>{/if}
    </p>
    {#if ins.tool && ins.phase === 'fetching_tool'}
      <div class="progrow">
        <span>DepotDownloader</span>
        <div class="prog"><div style="width: {toolPct}%"></div></div>
        <span class="num">{formatBytes(ins.tool.done)}</span>
      </div>
    {/if}
    {#each st?.depots ?? [] as d (d.id)}
      {#if d.id in ins.depots}
        <div class="progrow">
          <span>Depot {d.id} ({d.role})</span>
          <div class="prog"><div style="width: {ins.depots[d.id]}%"></div></div>
          <span class="num">{ins.depots[d.id].toFixed(1)}%</span>
        </div>
      {/if}
    {/each}
    {#if ins.prompt}
      <form class="notice" onsubmit={sendCode}>
        <p>{promptText(ins.prompt)}</p>
        <div class="row">
          <input bind:value={code} placeholder="Steam Guard code" autocomplete="one-time-code" maxlength="8" />
          <button class="primary" type="submit" disabled={!validGuardCode(code)}>Send</button>
        </div>
      </form>
    {/if}
  </section>
</div>

{#if ins.verify}
  <section class="card">
    <h2>Verification {ins.verify.ok ? '' : '(failed)'}</h2>
    <p class="muted">{ins.verify.files.toLocaleString()} files, {formatBytes(ins.verify.bytes)}</p>
    <div class="table-wrap">
      <table>
        <tbody>
          {#each ins.verify.checks as c (c.name)}
            <tr>
              <td><span class="pill {c.status === 'pass' ? 'ok' : c.status === 'fail' ? 'bad' : 'warn'}">{c.status}</span></td>
              <td>{c.name}</td>
              <td class="muted">{c.detail}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  </section>
{/if}

<section class="card">
  <h2>Output</h2>
  <div class="log" bind:this={logEl} aria-live="off">
    {#each ins.log as l, i (i)}<div class={l.stream}>{l.text}</div>{/each}
    {#if ins.log.length === 0}<span class="muted">The downloader's output appears here.</span>{/if}
  </div>
</section>
