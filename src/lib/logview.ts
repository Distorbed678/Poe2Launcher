import type { LogFile } from './types';

export type Level = 'error' | 'warn' | 'info';

/** Rough severity of a log line, for colouring only. */
export function lineLevel(line: string): Level {
  const l = line.toLowerCase();
  if (/\b(error|fatal|panic|panicked|failed|exception|crash)\b/.test(l) || l.includes('[err]')) return 'error';
  if (/\b(warn|warning)\b/.test(l)) return 'warn';
  return 'info';
}

export interface ViewLine {
  /** 1-based number in the loaded text. */
  n: number;
  text: string;
  level: Level;
}

/** The lines to show: all, or only those that contain every search word (case-insensitive); `problemsOnly` keeps warnings and errors. */
export function visibleLines(lines: string[], search: string, onlyMatching: boolean, problemsOnly = false): ViewLine[] {
  const words = search.toLowerCase().split(/\s+/).filter(Boolean);
  const out: ViewLine[] = [];
  lines.forEach((text, i) => {
    const level = lineLevel(text);
    if (problemsOnly && level === 'info') return;
    if (onlyMatching && words.length) {
      const l = text.toLowerCase();
      if (!words.every((w) => l.includes(w))) return;
    }
    out.push({ n: i + 1, text, level });
  });
  return out;
}

/** A line cut into parts that match the search and parts that do not (for highlighting). */
export function highlight(text: string, search: string): { s: string; hit: boolean }[] {
  const words = [...new Set(search.toLowerCase().split(/\s+/).filter(Boolean))];
  if (words.length === 0) return [{ s: text, hit: false }];
  const lower = text.toLowerCase();
  const mark = new Array<boolean>(text.length).fill(false);
  for (const w of words) {
    let from = 0;
    for (;;) {
      const i = lower.indexOf(w, from);
      if (i < 0) break;
      for (let k = i; k < i + w.length; k++) mark[k] = true;
      from = i + w.length;
    }
  }
  const out: { s: string; hit: boolean }[] = [];
  let start = 0;
  for (let i = 1; i <= text.length; i++) {
    if (i === text.length || mark[i] !== mark[start]) {
      out.push({ s: text.slice(start, i), hit: mark[start]! });
      start = i;
    }
  }
  return out;
}

export function countMatches(lines: string[], search: string): number {
  return visibleLines(lines, search, true).length;
}

export const KIND_TITLES: Record<LogFile['kind'], string> = {
  game: 'Game logs',
  server: 'Hosted server',
  crash: 'Crash dumps',
};

/** Files grouped by kind in a fixed order, newest first inside a group (the backend sorts them). */
export function groupFiles(files: LogFile[]): { kind: LogFile['kind']; title: string; files: LogFile[] }[] {
  return (['game', 'server', 'crash'] as const)
    .map((kind) => ({ kind, title: KIND_TITLES[kind], files: files.filter((f) => f.kind === kind) }))
    .filter((g) => g.files.length > 0);
}

/** `game-1761000000.log` shows as its date; other names as they are. */
export function fileLabel(f: Pick<LogFile, 'name' | 'kind' | 'modified'>, fmt: (unix: number) => string): string {
  return f.kind === 'game' && /^game-\d+\.log$/.test(f.name) ? `Game, ${fmt(f.modified)}` : f.name;
}
