import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { InstallEvent } from './install';
import type { PlayState, RunState, VerifyReport } from './types';
import type { Phase } from './install';
import type { PromptKind } from './types';

type Handler<T> = (payload: T) => void;

/** Subscribes to the backend's install events and hands them over as reducer events. */
export async function listenInstall(onEvent: Handler<InstallEvent>): Promise<UnlistenFn> {
  const subs = await Promise.all([
    listen<{ state: Phase; message?: string }>('install:state', (e) =>
      onEvent({ type: 'state', state: e.payload.state, message: e.payload.message }),
    ),
    listen<{ text: string; stream: string }>('install:log', (e) =>
      onEvent({ type: 'log', text: e.payload.text, stream: e.payload.stream }),
    ),
    listen<{ phase: 'tool'; done: number; total: number } | { phase: 'depot'; depot: number | null; percent: number }>(
      'install:progress',
      (e) => onEvent({ type: 'progress', ...e.payload } as InstallEvent),
    ),
    listen<{ kind: PromptKind }>('install:prompt', (e) => onEvent({ type: 'prompt', kind: e.payload.kind })),
    listen<VerifyReport>('install:verify', (e) => onEvent({ type: 'verify', report: e.payload })),
  ]);
  return () => subs.forEach((u) => u());
}

export async function listenPlay(
  onState: Handler<PlayState>,
  onWarning: Handler<string>,
): Promise<UnlistenFn> {
  const subs = await Promise.all([
    listen<PlayState>('play:status', (e) => onState(e.payload)),
    listen<{ text: string }>('play:warning', (e) => onWarning(e.payload.text)),
  ]);
  return () => subs.forEach((u) => u());
}

export interface HostProgress {
  phase: string;
  done: number;
  total: number;
}

/** The hosted server's events: run state, log lines, download progress, setup steps, a new package. */
export async function listenHost(h: {
  onState: Handler<RunState>;
  onLog: Handler<string>;
  onProgress: Handler<HostProgress>;
  onStep: Handler<string>;
  onInstalled: Handler<void>;
}): Promise<UnlistenFn> {
  const subs = await Promise.all([
    listen<RunState>('host:status', (e) => h.onState(e.payload)),
    listen<{ line: string }>('host:log', (e) => h.onLog(e.payload.line)),
    listen<HostProgress>('host:progress', (e) => h.onProgress(e.payload)),
    listen<{ step: string }>('host:setup', (e) => h.onStep(e.payload.step)),
    listen<unknown>('host:installed', () => h.onInstalled()),
  ]);
  return () => subs.forEach((u) => u());
}

/** The client hook download: progress of the archive and the current step. */
export async function listenHook(h: { onProgress: Handler<HostProgress>; onStep: Handler<string> }): Promise<UnlistenFn> {
  const subs = await Promise.all([
    listen<HostProgress>('hook:progress', (e) => h.onProgress(e.payload)),
    listen<{ step: string }>('hook:step', (e) => h.onStep(e.payload.step)),
  ]);
  return () => subs.forEach((u) => u());
}
