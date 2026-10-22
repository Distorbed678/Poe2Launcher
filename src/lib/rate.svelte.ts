import { SpeedMeter } from './transfer';

/** Speed and time left of one transfer, as reactive state the UI can show. */
export class RateTracker {
  bps = $state<number | null>(null);
  etaSecs = $state<number | null>(null);
  private meter = new SpeedMeter();

  /** `done` and `total` in bytes (total 0 = unknown). */
  feed(done: number, total: number, t: number = Date.now()): void {
    this.meter.sample(done, t);
    this.bps = this.meter.rate();
    this.etaSecs = total > 0 ? this.meter.etaSecs(Math.max(0, total - done)) : null;
  }

  reset(): void {
    this.meter.reset();
    this.bps = null;
    this.etaSecs = null;
  }
}
