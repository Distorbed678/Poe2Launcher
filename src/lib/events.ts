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

/** A hosted server's events (`id` picks the server): run state, log lines, download progress,
 * setup steps, a new package. `id` null: every server's run state (the Host list). */
export async function listenHost(id: string | null, h: {
  onState?: (state: RunState, id: string) => void;
  onLog?: Handler<string>;
  onProgress?: Handler<HostProgress>;
  onStep?: Handler<string>;
  onInstalled?: Handler<void>;
}): Promise<UnlistenFn> {
  const mine = (p: { id?: string }) => id === null || p.id === id;
  const subs = await Promise.all([
    listen<{ id: string; run: RunState }>('host:status', (e) => mine(e.payload) && h.onState?.(e.payload.run, e.payload.id)),
    listen<{ id: string; line: string }>('host:log', (e) => mine(e.payload) && h.onLog?.(e.payload.line)),
    listen<HostProgress & { id: string }>('host:progress', (e) => mine(e.payload) && h.onProgress?.(e.payload)),
    listen<{ id: string; step: string }>('host:setup', (e) => mine(e.payload) && h.onStep?.(e.payload.step)),
    listen<{ id: string }>('host:installed', (e) => mine(e.payload) && h.onInstalled?.()),
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

/** Mod downloads: progress of the package being fetched, and mods the launcher synced by itself (Auto, or at Play). */
export async function listenMods(h: {
  onProgress: Handler<{ id: string; version: string; done: number; total: number }>;
  onSynced: Handler<{ profile: string; mods: string[] }>;
}): Promise<UnlistenFn> {
  const subs = await Promise.all([
    listen<{ id: string; version: string; done: number; total: number }>('mods:progress', (e) => h.onProgress(e.payload)),
    listen<{ profile: string; mods: string[] }>('mods:synced', (e) => h.onSynced(e.payload)),
  ]);
  return () => subs.forEach((u) => u());
}
