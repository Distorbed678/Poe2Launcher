/** The custom title bar's window calls. Outside Tauri (the browser preview, tests) they do nothing. */
import { getCurrentWindow } from '@tauri-apps/api/window';

type Dir = 'North' | 'South' | 'East' | 'West' | 'NorthEast' | 'NorthWest' | 'SouthEast' | 'SouthWest';

function win() {
  try {
    return getCurrentWindow();
  } catch {
    return null;
  }
}

export const frame = {
  available: () => win() !== null,
  minimize: () => void win()?.minimize().catch(() => {}),
  toggleMaximize: () => void win()?.toggleMaximize().catch(() => {}),
  close: () => void win()?.close().catch(() => {}),
  isMaximized: async (): Promise<boolean> => (await win()?.isMaximized().catch(() => false)) ?? false,
  isFullscreen: async (): Promise<boolean> => (await win()?.isFullscreen().catch(() => false)) ?? false,
  resize: (dir: Dir) => void win()?.startResizeDragging(dir).catch(() => {}),
  setDecorations: (on: boolean) => void win()?.setDecorations(on).catch(() => {}),
  /** Calls `fn` when the window size changes (maximize, restore, resize). */
  onResized: async (fn: () => void): Promise<() => void> => {
    const w = win();
    if (!w) return () => {};
    try {
      return await w.onResized(fn);
    } catch {
      return () => {};
    }
  },
};

export type { Dir };
