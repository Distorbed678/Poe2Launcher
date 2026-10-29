/** An in-app confirmation dialog (replaces `window.confirm`). `await ask({...})` resolves to the user's choice. */
export interface ConfirmRequest {
  title: string;
  message: string;
  confirmLabel?: string;
  cancelLabel?: string;
  /** Styles the confirm button as destructive. */
  danger?: boolean;
  /** A checkbox that must be ticked before the confirm button works (a warning to accept). */
  acknowledge?: string;
  /** Text the user must type exactly (e.g. a server's name) before the confirm button works. */
  typeToConfirm?: string;
}

interface Pending extends ConfirmRequest {
  resolve: (ok: boolean) => void;
}

class Confirmer {
  current = $state<Pending | null>(null);

  ask(req: ConfirmRequest): Promise<boolean> {
    // A second question while one is open answers the first with "no".
    this.current?.resolve(false);
    return new Promise((resolve) => {
      this.current = { ...req, resolve };
    });
  }

  answer(ok: boolean): void {
    const c = this.current;
    this.current = null;
    c?.resolve(ok);
  }
}

export const confirmer = new Confirmer();
export const ask = (req: ConfirmRequest): Promise<boolean> => confirmer.ask(req);
