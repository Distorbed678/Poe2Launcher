import type { DepotView } from './types';

/** Speed from a sliding window of (time, bytes done) samples. */
export class SpeedMeter {
  private samples: { t: number; v: number }[] = [];

  constructor(private readonly windowMs = 15_000) {}

  reset(): void {
    this.samples = [];
  }

  /** Records `done` bytes at time `t` (ms). A counter that goes backwards starts a new measurement. */
  sample(done: number, t: number = Date.now()): void {
    const last = this.samples.at(-1);
    if (last && (done < last.v || t < last.t)) this.samples = [];
    this.samples.push({ t, v: done });
    const from = t - this.windowMs;
    while (this.samples.length > 2 && this.samples[1]!.t < from) this.samples.shift();
  }

  /** Bytes per second over the window, or null when there is not enough to say. */
  rate(): number | null {
    if (this.samples.length < 2) return null;
    const a = this.samples[0]!;
    const b = this.samples.at(-1)!;
    const dt = (b.t - a.t) / 1000;
    if (dt < 1) return null;
    const r = (b.v - a.v) / dt;
    return r > 0 ? r : null;
  }

  /** Seconds until `remaining` bytes are done at the current rate. */
  etaSecs(remaining: number): number | null {
    const r = this.rate();
    if (r === null || remaining <= 0) return remaining <= 0 ? 0 : null;
    return remaining / r;
  }
}

/** "12.3 MB/s", or "-" when unknown. */
export function formatSpeed(bps: number | null): string {
  if (bps === null || !Number.isFinite(bps) || bps <= 0) return '-';
  const units = ['B/s', 'KB/s', 'MB/s', 'GB/s'];
  let v = bps;
  let i = 0;
  while (v >= 1000 && i < units.length - 1) {
    v /= 1000;
    i++;
  }
  return `${i === 0 ? Math.round(v) : v.toFixed(v >= 100 ? 0 : 1)} ${units[i]}`;
}

/** "45 s", "3 min", "2 h 5 min", "1 d 4 h"; "-" when unknown. */
export function formatEta(secs: number | null): string {
  if (secs === null || !Number.isFinite(secs) || secs < 0) return '-';
  if (secs < 60) return `${Math.max(1, Math.round(secs))} s`;
  const m = Math.round(secs / 60);
  if (m < 60) return `${m} min`;
  const h = Math.floor(m / 60);
  if (h < 48) return `${h} h ${m % 60} min`;
  return `${Math.floor(h / 24)} d ${h % 24} h`;
}

/** Bytes of the whole install that are done, from the percent of each depot (a depot not started counts 0). */
export function installBytesDone(depots: DepotView[], percents: Record<number, number>): number {
  let sum = 0;
  for (const d of depots) sum += (d.est_bytes * Math.max(0, Math.min(100, percents[d.id] ?? 0))) / 100;
  return sum;
}

export function installBytesTotal(depots: DepotView[]): number {
  return depots.reduce((a, d) => a + d.est_bytes, 0);
}

/** What the progress card shows. */
export interface Rate {
  bps: number | null;
  etaSecs: number | null;
  done: number;
  total: number;
}
