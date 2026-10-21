import type { PromptKind, VerifyReport } from './types';

export type Phase = 'idle' | 'fetching_tool' | 'downloading' | 'verifying' | 'done' | 'failed' | 'cancelled';

export interface InstallState {
  phase: Phase;
  message: string;
  /** Output of the download, newest last, bounded. */
  log: { text: string; stream: string }[];
  tool: { done: number; total: number } | null;
  /** Percent per depot id. */
  depots: Record<number, number>;
  /** DepotDownloader waits for a Steam Guard code. */
  prompt: Exclude<PromptKind, 'password'> | null;
  verify: VerifyReport | null;
}

export const MAX_LOG_LINES = 1500;

export function initialInstall(): InstallState {
  return { phase: 'idle', message: '', log: [], tool: null, depots: {}, prompt: null, verify: null };
}

export type InstallEvent =
  | { type: 'state'; state: Phase; message?: string }
  | { type: 'log'; text: string; stream: string }
  | { type: 'progress'; phase: 'tool'; done: number; total: number }
  | { type: 'progress'; phase: 'depot'; depot: number | null; percent: number }
  | { type: 'prompt'; kind: PromptKind }
  | { type: 'verify'; report: VerifyReport }
  | { type: 'input_sent' };

const ACTIVE: Phase[] = ['fetching_tool', 'downloading', 'verifying'];

export function isActive(p: Phase): boolean {
  return ACTIVE.includes(p);
}

/** Pure reducer: the install page's state from the backend's events. */
export function reduce(s: InstallState, ev: InstallEvent): InstallState {
  switch (ev.type) {
    case 'state': {
      // A new run starts clean.
      const fresh = ev.state === 'fetching_tool' ? initialInstall() : s;
      return { ...fresh, phase: ev.state, message: ev.message ?? '', prompt: isActive(ev.state) ? fresh.prompt : null };
    }
    case 'log': {
      const log = s.log.length >= MAX_LOG_LINES ? s.log.slice(s.log.length - MAX_LOG_LINES + 1) : s.log.slice();
      log.push({ text: ev.text, stream: ev.stream });
      return { ...s, log };
    }
    case 'progress':
      if (ev.phase === 'tool') return { ...s, tool: { done: ev.done, total: ev.total } };
      if (ev.depot === null) return s;
      return { ...s, depots: { ...s.depots, [ev.depot]: Math.max(0, Math.min(100, ev.percent)) } };
    case 'prompt':
      // The password is answered by the backend from the form; only Guard codes reach the UI.
      return ev.kind === 'password' ? s : { ...s, prompt: ev.kind };
    case 'verify':
      return { ...s, verify: ev.report };
    case 'input_sent':
      return { ...s, prompt: null };
  }
}

export function promptText(kind: Exclude<PromptKind, 'password'>): string {
  return kind === 'guard_email'
    ? 'Steam Guard: enter the code Steam emailed you.'
    : 'Steam Guard: enter the code from your authenticator app (or confirm the sign-in in the Steam mobile app).';
}

/** A Steam Guard code is 5 characters, letters and digits. */
export function validGuardCode(code: string): boolean {
  return /^[A-Za-z0-9]{5}$/.test(code.trim());
}
