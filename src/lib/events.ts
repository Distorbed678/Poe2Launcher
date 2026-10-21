import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { InstallEvent } from './install';
import type { PlayState, VerifyReport } from './types';
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
