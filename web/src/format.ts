/**
 * Date and time formatting.
 *
 * Deliberately does NOT follow navigator.language. A browser set to English by
 * someone sitting in Germany reports en-US, and the runtime default then
 * renders 8/18/2026 — a format that is not merely unfamiliar but ambiguous,
 * since 8/9 could be either day. The instance locale is configuration, not
 * something to infer from a header.
 *
 * Resolution order:
 *   1. a per-browser override in localStorage (setLocale), set in Settings
 *   2. VITE_LOCALE at build time
 *   3. en-CA with a 24-hour clock: 2026-08-18 reads the same everywhere,
 *      and relative times stay in English
 */
const LOCALE_KEY = 'hoard.locale';

const FALLBACK_LOCALE = 'en-CA';

export function locale(): string {
  try {
    const stored = localStorage.getItem(LOCALE_KEY);
    if (stored) return stored;
  } catch {
    // Private-mode browsers can throw on localStorage; the default is fine.
  }
  return import.meta.env.VITE_LOCALE || FALLBACK_LOCALE;
}

/** Overrides the locale for this browser. Empty string restores the default. */
export function setLocale(next: string) {
  if (next) localStorage.setItem(LOCALE_KEY, next);
  else localStorage.removeItem(LOCALE_KEY);
}

// Intl formatters are expensive to construct and are used per row, so they are
// built once per locale rather than per call.
const dateCache = new Map<string, Intl.DateTimeFormat>();
const dateTimeCache = new Map<string, Intl.DateTimeFormat>();

function formatter(cache: Map<string, Intl.DateTimeFormat>, opts: Intl.DateTimeFormatOptions) {
  const key = locale();
  let f = cache.get(key);
  if (!f) {
    // The fallback's own clock is 12-hour; ISO dates read oddly next to "p.m.".
    f = new Intl.DateTimeFormat(key, key === FALLBACK_LOCALE ? { ...opts, hourCycle: 'h23' } : opts);
    cache.set(key, f);
  }
  return f;
}

/** "2026-08-18" by default. Times are rendered in the browser's own zone. */
export function formatDate(iso: string | undefined | null): string {
  const d = parse(iso);
  if (!d) return '';
  return formatter(dateCache, { year: 'numeric', month: '2-digit', day: '2-digit' }).format(d);
}

/** "2026-08-18 14:30" by default. */
export function formatDateTime(iso: string | undefined | null): string {
  const d = parse(iso);
  if (!d) return '';
  return formatter(dateTimeCache, {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  }).format(d);
}

/**
 * "in 6 days" / "3 hours ago", which is what actually matters for a retention
 * window — a date alone makes you count.
 */
export function formatRelative(iso: string | undefined | null): string {
  const d = parse(iso);
  if (!d) return '';
  const seconds = (d.getTime() - Date.now()) / 1000;
  const rtf = new Intl.RelativeTimeFormat(locale(), { numeric: 'auto' });
  const steps: [Intl.RelativeTimeFormatUnit, number][] = [
    ['year', 31536000],
    ['month', 2592000],
    ['day', 86400],
    ['hour', 3600],
    ['minute', 60],
  ];
  for (const [unit, size] of steps) {
    if (Math.abs(seconds) >= size) return rtf.format(Math.round(seconds / size), unit);
  }
  return rtf.format(Math.round(seconds), 'second');
}

/** Returns null rather than an Invalid Date, so callers render nothing. */
function parse(iso: string | undefined | null): Date | null {
  if (!iso) return null;
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? null : d;
}
