const UNITS = ['B', 'KB', 'MB', 'GB', 'TB'];

/** Decimal units, like the OS file manager and Steam. */
export function formatBytes(n: number): string {
  if (!Number.isFinite(n) || n < 0) return '-';
  let v = n;
  let i = 0;
  while (v >= 1000 && i < UNITS.length - 1) {
    v /= 1000;
    i++;
  }
  return i === 0 ? `${n} B` : `${v.toFixed(1)} ${UNITS[i]}`;
}

export function formatUptime(secs: number): string {
  if (secs < 60) return `${secs}s`;
  const m = Math.floor(secs / 60);
  if (m < 60) return `${m} min`;
  const h = Math.floor(m / 60);
  if (h < 48) return `${h} h ${m % 60} min`;
  return `${Math.floor(h / 24)} d ${h % 24} h`;
}

/** "in 6 days", "expired", for a Unix-seconds expiry. */
export function formatExpiry(expiresAt: number | null, nowSecs = Math.floor(Date.now() / 1000)): string {
  if (expiresAt === null) return '';
  const left = expiresAt - nowSecs;
  if (left <= 0) return 'expired';
  if (left < 3600) return `in ${Math.max(1, Math.floor(left / 60))} min`;
  if (left < 172800) return `in ${Math.floor(left / 3600)} h`;
  return `in ${Math.floor(left / 86400)} days`;
}
