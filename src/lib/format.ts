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

/** "22 Oct 2026" for a `YYYY-MM-DD` or RFC 3339 date (local time is not involved: the date part is shown as written). */
export function formatNewsDate(date: string): string {
  const m = /^(\d{4})-(\d{2})-(\d{2})/.exec(date);
  if (!m) return date;
  const months = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
  return `${Number(m[3])} ${months[Number(m[2]) - 1] ?? m[2]} ${m[1]}`;
}

/** "3 min ago", "2 h ago", "5 d ago" for a Unix-seconds time. */
export function formatAgo(unix: number, nowSecs = Math.floor(Date.now() / 1000)): string {
  const d = nowSecs - unix;
  if (d < 45) return 'just now';
  if (d < 3600) return `${Math.max(1, Math.round(d / 60))} min ago`;
  if (d < 172800) return `${Math.round(d / 3600)} h ago`;
  return `${Math.floor(d / 86400)} d ago`;
}

/** "22 Oct 2026, 13:45" in local time for a Unix-seconds time. */
export function formatDateTime(unix: number): string {
  const d = new Date(unix * 1000);
  const pad = (n: number) => String(n).padStart(2, '0');
  const months = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
  return `${d.getDate()} ${months[d.getMonth()]} ${d.getFullYear()}, ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}
