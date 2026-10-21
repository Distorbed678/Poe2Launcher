import { errorText } from './ipc';

export interface Toast {
  id: number;
  kind: 'ok' | 'error';
  text: string;
}

class Toasts {
  items = $state<Toast[]>([]);
  private next = 1;

  show(kind: Toast['kind'], text: string): void {
    const id = this.next++;
    this.items = [...this.items, { id, kind, text }];
    setTimeout(() => this.dismiss(id), kind === 'error' ? 9000 : 4000);
  }

  dismiss(id: number): void {
    this.items = this.items.filter((t) => t.id !== id);
  }
}
export const toasts = new Toasts();

/** Runs an action and reports the outcome in a toast. Returns the result, or undefined on failure. */
export async function run<T>(fn: () => Promise<T>, ok?: string | ((r: T) => string)): Promise<T | undefined> {
  try {
    const r = await fn();
    const text = typeof ok === 'function' ? ok(r) : ok;
    if (text) toasts.show('ok', text);
    return r;
  } catch (e) {
    toasts.show('error', errorText(e));
    return undefined;
  }
}
