import { useCallback, useEffect, useState } from 'react';

/** Orders a listing. Mirrors search::order_by on the server. */
export type SortOrder =
  | 'relevance'
  | 'newest'
  | 'oldest'
  | 'title'
  | 'title_desc'
  | 'largest'
  | 'smallest'
  | 'longest'
  | 'shortest';

const OPTIONS: { value: SortOrder; label: string }[] = [
  { value: 'relevance', label: 'Best match' },
  { value: 'newest', label: 'Newest first' },
  { value: 'oldest', label: 'Oldest first' },
  { value: 'title', label: 'Name A–Z' },
  { value: 'title_desc', label: 'Name Z–A' },
  { value: 'largest', label: 'Largest first' },
  { value: 'smallest', label: 'Smallest first' },
  { value: 'longest', label: 'Longest first' },
  { value: 'shortest', label: 'Shortest first' },
];

// Search remembers its own order: "best match" is what a query wants, and
// carrying it into a folder listing, where nothing is being matched, would be
// meaningless.
const keyFor = (search: boolean) => (search ? 'hoard.searchSort' : 'hoard.sort');

function read(search: boolean): SortOrder {
  try {
    const stored = localStorage.getItem(keyFor(search)) as SortOrder | null;
    if (stored && OPTIONS.some((o) => o.value === stored) && (search || stored !== 'relevance')) return stored;
  } catch {
    // Private-mode browsers can throw; the default is fine.
  }
  return search ? 'relevance' : 'newest';
}

/** Remembered per browser, like the view mode. */
export function useSortOrder(search = false): [SortOrder, (next: SortOrder) => void] {
  const [sort, setSort] = useState<SortOrder>(() => read(search));

  useEffect(() => {
    const onStorage = (e: StorageEvent) => {
      if (e.key === keyFor(search)) setSort(read(search));
    };
    window.addEventListener('storage', onStorage);
    return () => window.removeEventListener('storage', onStorage);
  }, [search]);

  const update = useCallback(
    (next: SortOrder) => {
      setSort(next);
      try {
        localStorage.setItem(keyFor(search), next);
      } catch {
        // Not remembering the choice is no reason to refuse it.
      }
    },
    [search],
  );

  return [sort, update];
}

export function SortSelect({
  value,
  onChange,
  searchMode = false,
}: {
  value: SortOrder;
  onChange: (next: SortOrder) => void;
  /** Adds "best match", which only means something for a query. */
  searchMode?: boolean;
}) {
  const options = searchMode ? OPTIONS : OPTIONS.filter((o) => o.value !== 'relevance');

  return (
    <label className="flex items-center gap-1.5 text-sm">
      <span className="sr-only">Sort by</span>
      <select
        value={value}
        onChange={(e) => onChange(e.target.value as SortOrder)}
        className="rounded-md border border-edge bg-panel px-2 py-1.5 text-sm outline-none focus:border-accent"
      >
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
    </label>
  );
}
