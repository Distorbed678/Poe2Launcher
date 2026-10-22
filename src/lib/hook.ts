import type { HookCheck, HookStatus } from './types';

/** What the hook is, in one line for a chip and one for the details. */
export interface HookSummary {
  tone: 'ok' | 'warn' | 'bad' | 'neutral';
  label: string;
  detail: string;
}

export function hookReady(h: HookStatus | null): boolean {
  return h !== null && h.source.kind !== 'missing';
}

export function hookSummary(h: HookStatus | null, check: HookCheck | null): HookSummary {
  if (!h) return { tone: 'neutral', label: 'checking', detail: '' };
  switch (h.source.kind) {
    case 'manual':
      return {
        tone: 'warn',
        label: 'your own build',
        detail: `Using the DLLs in ${h.dll_dir} (Settings, Hook DLL directory). The downloaded hook is not used while this is set.`,
      };
    case 'legacy':
      return {
        tone: 'warn',
        label: 'files copied by hand',
        detail: `Using the DLLs in ${h.dll_dir}. Download the hook to have a verified, versioned copy.`,
      };
    case 'downloaded':
      return check?.update_available
        ? { tone: 'warn', label: `${h.source.version}, update available`, detail: `Version ${check.release.version} is out.` }
        : { tone: 'ok', label: `${h.source.version} installed`, detail: `Verified when downloaded and again before every Play.` };
    default:
      return {
        tone: 'bad',
        label: 'not installed',
        detail: `Download the client hook for game version ${h.game_version}, or set the Hook DLL directory in Settings.`,
      };
  }
}

/** The button next to the status: what it says and whether it is a download at all. */
export function hookAction(h: HookStatus | null, check: HookCheck | null): { label: string; primary: boolean } {
  if (h?.installed && check && !check.update_available) return { label: 'Check again', primary: false };
  if (h?.installed) return { label: check?.update_available ? 'Update' : 'Check for updates', primary: !!check?.update_available };
  return { label: 'Download client hook', primary: true };
}

export interface SetupStep {
  id: 'game' | 'hook' | 'server';
  title: string;
  done: boolean;
  /** Step 3 is either of two ways. */
  detail: string;
}

export interface SetupInputs {
  gameInstalled: boolean;
  hookReady: boolean;
  profiles: number;
}

/** The first-start checklist: install the game, get the hook, then host or join a server. */
export function setupSteps(i: SetupInputs): SetupStep[] {
  return [
    { id: 'game', title: 'Install the game', done: i.gameInstalled, detail: 'Download Path of Exile 2 from Steam into the launcher\'s own folder.' },
    { id: 'hook', title: 'Get the client hook', done: i.hookReady, detail: 'The DLL that lets the game talk to a private server.' },
    { id: 'server', title: 'Host a server or join one', done: i.profiles > 0, detail: 'Host your own from the launcher, or add a friend\'s server.' },
  ];
}

/** The checklist is for people who are not set up yet. */
export function setupNeeded(steps: SetupStep[]): boolean {
  return steps.some((s) => !s.done);
}

/** The first step that is not done. */
export function currentStep(steps: SetupStep[]): SetupStep['id'] | null {
  return steps.find((s) => !s.done)?.id ?? null;
}
