import { describe, expect, it } from 'vitest';
import { countMatches, fileLabel, groupFiles, highlight, lineLevel, visibleLines } from './logview';
import type { LogFile } from './types';

describe('log view', () => {
  it('guesses the level of a line', () => {
    expect(lineLevel('2026 ERROR connect failed')).toBe('error');
    expect(lineLevel('thread panicked at x')).toBe('error');
    expect(lineLevel('WARN slow frame')).toBe('warn');
    expect(lineLevel('warning: low disk')).toBe('warn');
    expect(lineLevel('player errorless run')).toBe('info');
    expect(lineLevel('ok')).toBe('info');
  });

  const lines = ['start', 'ERROR disk full', 'Player Alice joined', 'WARN lag', 'error again'];
  it('filters by search words and by level', () => {
    expect(visibleLines(lines, '', false)).toHaveLength(5);
    expect(visibleLines(lines, 'alice', true).map((l) => l.n)).toEqual([3]);
    expect(visibleLines(lines, 'alice', false)).toHaveLength(5);
    expect(visibleLines(lines, 'player joined', true)).toHaveLength(1);
    expect(visibleLines(lines, 'nothing', true)).toEqual([]);
    expect(visibleLines(lines, '', false, true).map((l) => l.n)).toEqual([2, 4, 5]); // warnings and errors
    expect(countMatches(lines, 'error')).toBe(2);
  });

  it('cuts a line into highlighted parts', () => {
    expect(highlight('abc', '')).toEqual([{ s: 'abc', hit: false }]);
    expect(highlight('Disk full on disk', 'disk')).toEqual([
      { s: 'Disk', hit: true },
      { s: ' full on ', hit: false },
      { s: 'disk', hit: true },
    ]);
    // Overlapping words merge into one hit.
    expect(highlight('abcdef', 'abc cde')).toEqual([{ s: 'abcde', hit: true }, { s: 'f', hit: false }]);
    expect(highlight('', 'x')).toEqual([]);
  });

  it('groups files and labels game sessions', () => {
    const f = (kind: LogFile['kind'], name: string): LogFile => ({ id: `${kind}:${name}`, kind, name, size: 1, modified: 1761000000, text: true });
    const g = groupFiles([f('crash', 'a.dmp'), f('game', 'game-1.log'), f('game', 'game-2.log')]);
    expect(g.map((x) => [x.kind, x.files.length])).toEqual([['game', 2], ['crash', 1]]);
    expect(fileLabel(f('game', 'game-1761000000.log'), (u) => `T${u}`)).toBe('Game, T1761000000');
    expect(fileLabel(f('server', 'server.log'), () => 'x')).toBe('server.log');
  });
});
