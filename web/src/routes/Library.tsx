import { useCallback, useEffect, useState } from 'react';
import { Link, useSearchParams } from 'react-router';
import { useInfiniteQuery, useQuery } from '@tanstack/react-query';
import { countSelection, listMedia, type Filter, type Place } from '../api';
import { MediaGrid } from '../components/MediaGrid';
import { BulkBar } from '../components/BulkBar';
import { useSelection } from '../components/useSelection';
import { UploadDropzone } from '../components/UploadDropzone';
import { ViewToggle } from '../components/ViewToggle';
import { useViewMode } from '../components/viewMode';
import { SortSelect, useSortOrder, type SortOrder } from '../components/SortSelect';
import { useMoveDrop } from '../components/dnd';
import { childFolders, libraryUrl, placeLabel, useFolders, useRoots } from '../places';

/**
 * How many files the current filter matches in total, which is what turns
 * "select these 40" into "select all 312". Only asked for once something is
 * selected — before that, nobody is about to act on the number.
 */
function useMatchCount(filter: Filter, enabled: boolean) {
  const { data } = useQuery({
    queryKey: ['count', filter],
    queryFn: () => countSelection({ query: filter }).then((r) => r.count),
    enabled,
  });
  return data;
}

function useMedia(filter: Filter, sort: SortOrder, enabled = true) {
  const query = useInfiniteQuery({
    // sort is part of the key: changing it invalidates the pages, since an
    // offset into one order is meaningless in another.
    queryKey: ['media', filter, sort],
    initialPageParam: 0,
    queryFn: ({ pageParam }) => listMedia(filter, pageParam as number, sort),
    getNextPageParam: (last) => (last.nextOffset > 0 ? last.nextOffset : undefined),
    enabled,
  });
  return { query, items: query.data?.pages.flatMap((p) => p.media) ?? [] };
}

const RECURSIVE_KEY = 'hoard.recursive';

/** Whether a folder view includes everything below it, remembered per browser. */
function useRecursive(): [boolean, (next: boolean) => void] {
  const [value, setValue] = useState(() => {
    try {
      return localStorage.getItem(RECURSIVE_KEY) === '1';
    } catch {
      return false;
    }
  });
  const update = useCallback((next: boolean) => {
    setValue(next);
    try {
      localStorage.setItem(RECURSIVE_KEY, next ? '1' : '0');
    } catch {
      // Not remembering the choice is no reason to refuse it.
    }
  }, []);
  return [value, update];
}

/** The library grid: everything, one kind, or one folder with its subfolders. */
export function Library() {
  const [params] = useSearchParams();
  const [view, setView] = useViewMode();
  const [sort, setSort] = useSortOrder();
  const [recursive, setRecursive] = useRecursive();
  const roots = useRoots();
  const folders = useFolders();

  const rootParam = params.get('root');
  const rootId = rootParam !== null ? Number(rootParam) : undefined;
  const folder = params.get('folder') ?? '';
  const kind = params.get('kind') ?? undefined;
  const inFolder = rootId !== undefined && !Number.isNaN(rootId);
  const place: Place | null = inFolder ? { rootId: rootId!, folder } : null;

  const filter: Filter = inFolder ? { rootId, folder, recursive, kind } : { kind };
  const { query, items } = useMedia(filter, sort);
  const selection = useSelection(items, filter);
  const total = useMatchCount(filter, selection.active);
  const subfolders = inFolder ? childFolders(folders, rootId!, folder) : [];

  // A different folder or kind is a different set of files; keeping the ticks
  // would leave a bulk edit acting on things no longer on screen.
  useEffect(() => selection.clear(), [rootId, folder, kind, recursive]); // eslint-disable-line react-hooks/exhaustive-deps

  const heading = place ? (
    <Breadcrumbs place={place} rootName={roots?.find((r) => r.id === place.rootId)?.name ?? '…'} />
  ) : (
    <h1 className="text-sm font-medium">{kind === 'video' ? 'Videos' : kind === 'image' ? 'Images' : 'All media'}</h1>
  );

  let empty = 'Nothing here yet — import some links or drop files onto a folder.';
  if (roots && roots.length === 0) empty = 'Add a storage folder in Settings to get started.';
  else if (inFolder) empty = 'This folder is empty — drop files here, or drag media onto it in the sidebar.';

  return (
    <UploadDropzone
      target={place}
      heading={heading}
      actions={
        <div className="flex items-center gap-2">
          {inFolder && (
            <label className="flex items-center gap-1.5 text-sm text-muted">
              <input
                type="checkbox"
                checked={recursive}
                onChange={(e) => setRecursive(e.target.checked)}
                className="accent-accent"
              />
              Include subfolders
            </label>
          )}
          <SortSelect value={sort} onChange={setSort} />
          <ViewToggle value={view} onChange={setView} />
        </div>
      }
    >
      {query.isLoading ? (
        <p className="p-8 text-center text-sm text-muted">Loading…</p>
      ) : (
        <>
          <MediaGrid items={items} folders={subfolders} view={view} selection={selection} emptyMessage={empty} />
          <LoadMore
            hasMore={Boolean(query.hasNextPage)}
            loading={query.isFetchingNextPage}
            onClick={() => void query.fetchNextPage()}
          />
          <BulkBar selection={selection} loaded={items.length} total={total} fallbackRoot={rootId} />
        </>
      )}
    </UploadDropzone>
  );
}

/** Ranked search results, across the library or within one folder. */
export function SearchResults() {
  const [params] = useSearchParams();
  const [view, setView] = useViewMode();
  const [sort, setSort] = useSortOrder(true);
  const roots = useRoots();
  const q = params.get('q') ?? '';
  const rootParam = params.get('root');
  const scope: Place | null =
    rootParam !== null && !Number.isNaN(Number(rootParam))
      ? { rootId: Number(rootParam), folder: params.get('folder') ?? '' }
      : null;

  const filter: Filter = scope ? { q, rootId: scope.rootId, folder: scope.folder, recursive: true } : { q };
  const { query, items } = useMedia(filter, sort, q.length > 0);
  const selection = useSelection(items, filter);
  const total = useMatchCount(filter, selection.active && q.length > 0);

  useEffect(() => selection.clear(), [q, scope?.rootId, scope?.folder]); // eslint-disable-line react-hooks/exhaustive-deps

  return (
    <div>
      <div className="mb-3 flex items-center gap-3">
        <p className="text-sm text-muted">
          {query.isLoading
            ? 'Searching…'
            : `${items.length}${query.hasNextPage ? '+' : ''} result${items.length === 1 ? '' : 's'} for `}
          {!query.isLoading && <span className="text-white/90">{q}</span>}
          {scope && (
            <>
              {' in '}
              <Link to={libraryUrl(scope)} className="text-white/90 hover:underline">
                {placeLabel(scope, roots)}
              </Link>
              {' · '}
              <Link to={`/search?q=${encodeURIComponent(q)}`} className="text-accent hover:underline">
                search everywhere
              </Link>
            </>
          )}
        </p>
        <div className="ml-auto flex items-center gap-2">
          <SortSelect value={sort} onChange={setSort} searchMode />
          <ViewToggle value={view} onChange={setView} />
        </div>
      </div>
      <MediaGrid items={items} view={view} selection={selection} emptyMessage="Nothing matched that search." />
      <LoadMore
        hasMore={Boolean(query.hasNextPage)}
        loading={query.isFetchingNextPage}
        onClick={() => void query.fetchNextPage()}
      />
      <BulkBar selection={selection} loaded={items.length} total={total} />
    </div>
  );
}

/** Root / sub / dir, each a link — and a drop target, so files can be moved up. */
function Breadcrumbs({ place, rootName }: { place: Place; rootName: string }) {
  const parts = place.folder.split('/').filter(Boolean);
  const crumbs = [{ label: rootName, folder: '' }, ...parts.map((p, i) => ({ label: p, folder: parts.slice(0, i + 1).join('/') }))];
  return (
    <nav className="flex min-w-0 flex-wrap items-center gap-1 text-sm" aria-label="Folder">
      {crumbs.map((c, i) => (
        <span key={c.folder} className="flex items-center gap-1">
          {i > 0 && <span className="text-muted">/</span>}
          <Crumb place={{ rootId: place.rootId, folder: c.folder }} last={i === crumbs.length - 1}>
            {c.label}
          </Crumb>
        </span>
      ))}
    </nav>
  );
}

function Crumb({ place, last, children }: { place: Place; last: boolean; children: React.ReactNode }) {
  const { over, handlers } = useMoveDrop(place);
  return (
    <Link
      to={libraryUrl(place)}
      {...handlers}
      className={`rounded px-1.5 py-0.5 ${over ? 'bg-accent/30' : ''} ${
        last ? 'font-medium text-white' : 'text-muted hover:text-white'
      }`}
    >
      {children}
    </Link>
  );
}

function LoadMore({ hasMore, loading, onClick }: { hasMore: boolean; loading: boolean; onClick: () => void }) {
  if (!hasMore) return null;
  return (
    <div className="py-6 text-center">
      <button
        onClick={onClick}
        disabled={loading}
        className="rounded-md border border-edge px-4 py-2 text-sm hover:bg-edge disabled:opacity-40"
      >
        {loading ? 'Loading…' : 'Load more'}
      </button>
    </div>
  );
}
