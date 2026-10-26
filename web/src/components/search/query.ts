// The client side of the search syntax (server: src/search.rs).

export type Qualifier = { key: string; hint: string };

export const QUALIFIERS: Qualifier[] = [
  { key: 'tag', hint: 'has this tag; -tag: excludes it' },
  { key: 'folder', hint: 'in this folder or below it' },
  { key: 'root', hint: 'in this storage folder' },
  { key: 'kind', hint: 'video or image' },
];

/** Every prefix the server understands, aliases included. */
const KNOWN = new Set(['tag', 'folder', 'in', 'root', 'kind', 'type']);

/** The token being typed at the end of the input, quote aware. */
export function currentToken(input: string): { start: number; text: string } {
  let start = 0;
  let inQuote = false;
  for (let i = 0; i < input.length; i++) {
    const c = input[i];
    if (c === '"') inQuote = !inQuote;
    else if (!inQuote && /\s/.test(c)) start = i + 1;
  }
  return { start, text: input.slice(start) };
}

/** Swaps the token being typed for `value`; a complete value starts a new token. */
export function replaceToken(input: string, value: string, complete = true): string {
  return input.slice(0, currentToken(input).start) + value + (complete ? ' ' : '');
}

export const quoteValue = (value: string) => (/\s/.test(value) ? `"${value}"` : value);

export const hasQualifier = (input: string, key: string) => new RegExp(`(^|\\s)-?${key}:`, 'i').test(input);

/** The free words and phrases of a query: what to mark in result titles. */
export function highlightTerms(input: string): string[] {
  const terms: string[] = [];
  for (const match of input.matchAll(/(-?)("([^"]*)"?|[^\s"]+(?:"[^"]*"?)?)/g)) {
    if (match[1]) continue;
    const phrase = match[3];
    if (phrase !== undefined) {
      if (phrase.trim()) terms.push(phrase.trim());
      continue;
    }
    const key = match[2].split(':', 1)[0];
    if (match[2].includes(':') && KNOWN.has(key.toLowerCase())) continue;
    terms.push(match[2]);
  }
  return terms;
}

// ── recent searches ────────────────────────────────────────────────────

const RECENT_KEY = 'hoard.recentSearches';
const RECENT_MAX = 8;

export function recentSearches(): string[] {
  try {
    const list = JSON.parse(localStorage.getItem(RECENT_KEY) ?? '[]');
    return Array.isArray(list) ? list.filter((s) => typeof s === 'string') : [];
  } catch {
    return [];
  }
}

function saveRecent(list: string[]) {
  try {
    localStorage.setItem(RECENT_KEY, JSON.stringify(list.slice(0, RECENT_MAX)));
  } catch {
    // Forgetting history is harmless.
  }
}

/** Most recent first, without duplicates. */
export function rememberSearch(query: string) {
  const q = query.trim();
  if (q) saveRecent([q, ...recentSearches().filter((s) => s !== q)]);
}

export function forgetSearch(query: string) {
  saveRecent(recentSearches().filter((s) => s !== query));
}
