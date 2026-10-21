import { describe, expect, it } from 'vitest';
import { initialInstall, isActive, MAX_LOG_LINES, promptText, reduce, validGuardCode, type InstallEvent } from './install';

const run = (events: InstallEvent[]) => events.reduce(reduce, initialInstall());

describe('install reducer', () => {
  it('follows a normal run', () => {
    const s = run([
      { type: 'state', state: 'fetching_tool' },
      { type: 'progress', phase: 'tool', done: 5, total: 10 },
      { type: 'state', state: 'downloading' },
      { type: 'log', text: 'Downloading depot 2694491', stream: 'stdout' },
      { type: 'progress', phase: 'depot', depot: 2694491, percent: 42.5 },
      { type: 'progress', phase: 'depot', depot: 2694491, percent: 100 },
      { type: 'progress', phase: 'depot', depot: 2694492, percent: 10 },
    ]);
    expect(s.phase).toBe('downloading');
    expect(s.tool).toEqual({ done: 5, total: 10 });
    expect(s.depots).toEqual({ 2694491: 100, 2694492: 10 });
    expect(s.log.map((l) => l.text)).toEqual(['Downloading depot 2694491']);
    expect(isActive(s.phase)).toBe(true);
  });

  it('a new run starts clean and a finished one drops the prompt', () => {
    let s = run([
      { type: 'state', state: 'downloading' },
      { type: 'log', text: 'old', stream: 'stdout' },
      { type: 'prompt', kind: 'guard_device' },
    ]);
    expect(s.prompt).toBe('guard_device');
    s = reduce(s, { type: 'state', state: 'failed', message: 'bad password' });
    expect(s.prompt).toBeNull();
    expect(s.message).toBe('bad password');
    s = reduce(s, { type: 'state', state: 'fetching_tool' });
    expect(s.log).toEqual([]);
    expect(s.message).toBe('');
  });

  it('shows only Guard prompts: the password is answered by the backend', () => {
    expect(run([{ type: 'prompt', kind: 'password' }]).prompt).toBeNull();
    const s = run([{ type: 'state', state: 'downloading' }, { type: 'prompt', kind: 'guard_email' }]);
    expect(s.prompt).toBe('guard_email');
    expect(reduce(s, { type: 'input_sent' }).prompt).toBeNull();
    expect(promptText('guard_email')).toMatch(/emailed/);
    expect(promptText('guard_device')).toMatch(/authenticator/);
  });

  it('bounds the log and clamps progress', () => {
    let s = initialInstall();
    for (let i = 0; i < MAX_LOG_LINES + 50; i++) s = reduce(s, { type: 'log', text: `l${i}`, stream: 'stdout' });
    expect(s.log).toHaveLength(MAX_LOG_LINES);
    expect(s.log.at(-1)?.text).toBe(`l${MAX_LOG_LINES + 49}`);
    s = reduce(s, { type: 'progress', phase: 'depot', depot: 1, percent: 250 });
    expect(s.depots[1]).toBe(100);
    // Progress before any "Downloading depot" line has no depot to attach to.
    expect(reduce(s, { type: 'progress', phase: 'depot', depot: null, percent: 5 })).toBe(s);
  });

  it('stores the verification report', () => {
    const report = { ok: true, files: 1, bytes: 2, exe_sha256: null, checks: [] };
    expect(run([{ type: 'verify', report }]).verify).toEqual(report);
  });

  it('validates Steam Guard codes', () => {
    expect(validGuardCode('AB3D9')).toBe(true);
    expect(validGuardCode(' ab3d9 ')).toBe(true);
    expect(validGuardCode('1234')).toBe(false);
    expect(validGuardCode('12 45')).toBe(false);
  });
});
