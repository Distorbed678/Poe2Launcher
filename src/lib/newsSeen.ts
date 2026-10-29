/** Which news the user has seen: the date of the newest item when the News page was last opened (local only). */
const KEY = 'poe2-launcher.news-seen';

/** Items dated after `seen` (ISO dates compare as strings). Nothing seen yet: none count as new, so a first start shows no badge. */
export function unseenCount(dates: string[], seen: string | null): number {
  if (seen === null) return 0;
  return dates.filter((d) => d > seen).length;
}

export function newestDate(dates: string[]): string | null {
  return dates.reduce<string | null>((a, d) => (a === null || d > a ? d : a), null);
}

export function loadSeen(): string | null {
  try {
    return localStorage.getItem(KEY);
  } catch {
    return null;
  }
}

export function saveSeen(date: string): void {
  try {
    localStorage.setItem(KEY, date);
  } catch {
    /* private mode: the badge just stays */
  }
}
