import { useEffect, useMemo, useRef, useState } from 'react';
import { useLocation, useNavigate, useSearchParams } from 'react-router';
import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { listMedia, suggestTags, type Place } from '../../api';
import { libraryUrl, placeLabel, useFolders, useRoots } from '../../places';
import { FolderIcon } from '../MediaGrid';
import {
  QUALIFIERS,
  currentToken,
  forgetSearch,
  hasQualifier,
  highlightTerms,
  quoteValue,
  recentSearches,
  rememberSearch,
  replaceToken,
} from './query';
import { ItemRow, SearchIcon, Spinner, heading, itemKey, placeOf, type Item } from './SearchItems';

const QUICK_RESULTS = 8;
const FOLDER_RESULTS = 5;

function useDebounced<T>(value: T, ms: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const id = setTimeout(() => setDebounced(value), ms);
    return () => clearTimeout(id);
  }, [value, ms]);
  return debounced;
}

/** Aliases the server accepts, folded onto the qualifier they stand for. */
const ALIASES: Record<string, string> = { in: 'folder', type: 'kind' };

/**
 * The search box: results while you type, completion for qualifiers and their
 * values, recent searches, and a folder scope.
 *
 * Enter on the first row runs the full search, as a search engine does;
 * the arrows reach the instant results. On a folder's page the search starts
 * scoped to that folder, shown as a chip — Backspace at the start of the box,
 * or the chip's ×, widens it to the whole library.
 */
export function SearchBar() {
  const navigate = useNavigate();
  const { pathname } = useLocation();
  const [params] = useSearchParams();
  const roots = useRoots();
  const folders = useFolders();
  const [value, setValue] = useState('');
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const [recent, setRecent] = useState(recentSearches);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  // The box shows the query of the results page, and is empty elsewhere.
  const urlQuery = pathname === '/search' ? (params.get('q') ?? '') : '';
  useEffect(() => setValue(urlQuery), [urlQuery]);

  // ── scope ────────────────────────────────────────────────────────────
  const rootParam = params.get('root');
  const pageScope: Place | null =
    (pathname === '/library' || pathname === '/search') && rootParam !== null && !Number.isNaN(Number(rootParam))
      ? { rootId: Number(rootParam), folder: params.get('folder') ?? '' }
      : null;
  const scopeKey = pageScope ? `${pathname}:${pageScope.rootId}:${pageScope.folder}` : null;
  const [droppedScope, setDroppedScope] = useState<string | null>(null);
  // A folder or root typed into the query says where to look; the chip would contradict it.
  const explicitPlace = ['folder', 'in', 'root'].some((k) => hasQualifier(value, k));
  const scope = pageScope && droppedScope !== scopeKey && !explicitPlace ? pageScope : null;
  const scopeLabel = scope ? placeLabel(scope, roots) : '';
  const scopeFilter = scope ? { rootId: scope.rootId, folder: scope.folder, recursive: true } : {};

  // "/" and Ctrl/Cmd+K focus the search from anywhere.
  useEffect(() => {
    const down = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement;
      const typing = target.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName);
      if ((e.key === 'k' && (e.metaKey || e.ctrlKey)) || (e.key === '/' && !typing)) {
        e.preventDefault();
        inputRef.current?.focus();
        inputRef.current?.select();
      }
    };
    document.addEventListener('keydown', down);
    return () => document.removeEventListener('keydown', down);
  }, []);

  // ── qualifier being typed (tag:bea) ─────────────────────────────────
  const token = currentToken(value);
  const colon = token.text.indexOf(':');
  const rawKey = colon > 0 ? token.text.slice(0, colon).replace(/^-/, '').toLowerCase() : null;
  const key = rawKey ? (ALIASES[rawKey] ?? rawKey) : null;
  const qualifier = QUALIFIERS.find((q) => q.key === key);
  const partial = colon > 0 ? token.text.slice(colon + 1).replace(/^"/, '').replace(/"$/, '') : '';
  const negation = token.text.startsWith('-') ? '-' : '';

  const tagPrefix = useDebounced(qualifier?.key === 'tag' ? partial : '', 120);
  const { data: tagMatches } = useQuery({
    queryKey: ['tag-suggest', tagPrefix],
    queryFn: ({ signal }) => suggestTags(tagPrefix, signal),
    enabled: qualifier?.key === 'tag',
    staleTime: 30_000,
  });

  const valueSuggestions = useMemo((): Item[] | null => {
    if (!qualifier || !rawKey) return null;
    let values: { value: string; hint?: string }[];
    switch (qualifier.key) {
      case 'tag':
        // Only once the answer is for what is typed now, so a quick Tab
        // cannot complete a stale value.
        values = tagPrefix === partial ? (tagMatches ?? []).map((t) => ({ value: t })) : [];
        break;
      case 'folder': {
        const seen = new Map<string, number>();
        for (const f of folders ?? []) if (f.path) seen.set(f.path, (seen.get(f.path) ?? 0) + f.total);
        values = [...seen].map(([path, total]) => ({ value: path, hint: `${total} items` }));
        break;
      }
      case 'root':
        values = (roots ?? []).map((r) => ({ value: r.name, hint: `${r.count} items` }));
        break;
      default:
        values = [{ value: 'video' }, { value: 'image' }];
    }
    const lower = partial.toLowerCase();
    return values
      .filter((v) => (qualifier.key === 'tag' || v.value.toLowerCase().includes(lower)) && v.value.toLowerCase() !== lower)
      .slice(0, 8)
      .map((v) => {
        const insert = `${negation}${rawKey}:${quoteValue(v.value)}`;
        return { kind: 'suggestion', label: insert, hint: v.hint, insert, complete: true };
      });
  }, [qualifier, rawKey, partial, negation, tagPrefix, tagMatches, folders, roots]);

  // ── instant results ─────────────────────────────────────────────────
  const pending = !!valueSuggestions && valueSuggestions.length > 0;
  // A qualifier still being completed is left out until it is finished.
  const liveQuery = (pending ? value.slice(0, token.start) : value).trim();
  const query = useDebounced(liveQuery, 200);

  const { data, isFetching, isPlaceholderData } = useQuery({
    queryKey: ['quick-search', query, scope?.rootId, scope?.folder],
    // The signal drops superseded searches rather than letting them race.
    queryFn: ({ signal }) => listMedia({ q: query, ...scopeFilter }, 0, 'relevance', QUICK_RESULTS, signal),
    enabled: query.length > 1,
    placeholderData: keepPreviousData,
    staleTime: 15_000,
  });
  const media = query.length > 1 ? (data?.media ?? []) : [];
  const stale = liveQuery !== query || isPlaceholderData;
  const terms = useMemo(() => highlightTerms(query), [query]);

  const folderHits = useMemo(() => {
    const words = terms.map((t) => t.toLowerCase());
    if (words.length === 0) return [];
    return (folders ?? [])
      .filter((f) => f.path && words.every((w) => f.path.toLowerCase().includes(w)))
      .filter((f) => !scope || (f.rootId === scope.rootId && (!scope.folder || f.path.startsWith(`${scope.folder}/`))))
      .slice(0, FOLDER_RESULTS);
  }, [terms, folders, scope]);

  const keySuggestions = useMemo((): Item[] => {
    const t = token.text.toLowerCase();
    if (t.includes(':') || t.startsWith('"')) return [];
    const prefix = t.replace(/^-/, '');
    if (!prefix && value.trim() !== '') return [];
    return QUALIFIERS.filter((q) => q.key.startsWith(prefix) && (q.key === 'tag' || !hasQualifier(value, q.key))).map(
      (q) => ({
        kind: 'suggestion',
        label: `${q.key}:`,
        hint: q.hint,
        insert: `${t.startsWith('-') ? '-' : ''}${q.key}:`,
        complete: false,
      }),
    );
  }, [token.text, value]);

  const items = useMemo((): Item[] => {
    const out: Item[] = [];
    if (value.trim() === '') {
      if (pageScope && !scope && !explicitPlace) out.push({ kind: 'scope', label: placeLabel(pageScope, roots) });
      out.push(...recent.map((q): Item => ({ kind: 'recent', query: q })));
      out.push(...keySuggestions);
      return out;
    }
    if (pending) out.push(...valueSuggestions!);
    else out.push({ kind: 'search', query: liveQuery });
    for (const f of folderHits) out.push({ kind: 'folder', folder: f, label: placeLabel({ rootId: f.rootId, folder: f.path }, roots) });
    for (const m of media) out.push({ kind: 'media', media: m, place: placeLabel(placeOf(m), roots) });
    if (data?.nextOffset && media.length > 0) out.push({ kind: 'all', query: liveQuery });
    out.push(...keySuggestions);
    return out;
  }, [value, pageScope, scope, explicitPlace, roots, recent, keySuggestions, pending, valueSuggestions, liveQuery, folderHits, media, data]);

  const current = Math.max(0, Math.min(active, items.length - 1));
  useEffect(() => {
    listRef.current?.querySelector(`[data-index="${current}"]`)?.scrollIntoView({ block: 'nearest' });
  }, [current]);

  function go(url: string, newTab: boolean) {
    if (newTab) {
      window.open(url, '_blank');
      return;
    }
    setOpen(false);
    inputRef.current?.blur();
    navigate(url);
  }

  function submit(q: string, newTab = false) {
    const text = q.trim();
    if (!text) return;
    rememberSearch(text);
    setRecent(recentSearches());
    const search = new URLSearchParams({ q: text });
    if (scope) {
      search.set('root', String(scope.rootId));
      if (scope.folder) search.set('folder', scope.folder);
    }
    go(`/search?${search}`, newTab);
  }

  function choose(item: Item | undefined, newTab = false) {
    if (!item) return;
    switch (item.kind) {
      case 'search':
      case 'all':
        submit(item.query, newTab);
        break;
      case 'recent':
        setValue(item.query);
        submit(item.query, newTab);
        break;
      case 'suggestion':
        setValue(replaceToken(value, item.insert, item.complete));
        setActive(0);
        inputRef.current?.focus();
        break;
      case 'scope':
        setDroppedScope(null);
        break;
      case 'folder':
        go(libraryUrl({ rootId: item.folder.rootId, folder: item.folder.path }), newTab);
        break;
      case 'media':
        go(`/m/${item.media.id}`, newTab);
        break;
    }
  }

  function onKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    // Keys confirming an IME composition belong to the IME.
    if (e.nativeEvent.isComposing || e.keyCode === 229) return;
    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault();
        setOpen(true);
        setActive(Math.min(current + 1, items.length - 1));
        break;
      case 'ArrowUp':
        e.preventDefault();
        setActive(Math.max(current - 1, 0));
        break;
      case 'Enter': {
        e.preventDefault();
        const item = items[current];
        // A result from the previous query is not what was asked for.
        if (!open || !item || (item.kind === 'media' && stale)) submit(liveQuery || value, e.metaKey || e.ctrlKey);
        else choose(item, e.metaKey || e.ctrlKey);
        break;
      }
      case 'Tab': {
        // Tab completes the highlighted (or first) suggestion instead of leaving the box.
        const suggestion =
          items[current]?.kind === 'suggestion' ? items[current] : items.find((i) => i.kind === 'suggestion');
        if (suggestion && value.trim()) {
          e.preventDefault();
          choose(suggestion);
        }
        break;
      }
      case 'Backspace':
        if (scope && e.currentTarget.selectionStart === 0 && e.currentTarget.selectionEnd === 0) {
          e.preventDefault();
          setDroppedScope(scopeKey);
        }
        break;
      case 'Escape':
        if (open) setOpen(false);
        else inputRef.current?.blur();
        break;
    }
  }

  const showEmpty = query.length > 1 && !isFetching && data && media.length === 0 && folderHits.length === 0 && !pending;

  return (
    <div className="relative w-full min-w-0">
      <div
        className={`flex h-9 items-center gap-2 rounded-md border bg-ink px-3 text-sm transition-colors ${
          open ? 'border-accent' : 'border-edge'
        }`}
        onMouseDown={(e) => {
          if (e.target !== inputRef.current) {
            e.preventDefault();
            inputRef.current?.focus();
          }
        }}
      >
        {isFetching ? <Spinner /> : <SearchIcon />}
        {scope && (
          <span
            title={`Searching in ${scopeLabel}`}
            className="flex max-w-[40%] shrink-0 items-center gap-1 rounded bg-edge py-0.5 pr-0.5 pl-1.5 text-xs font-medium"
          >
            <FolderIcon className="size-3.5 shrink-0 text-muted" />
            <span className="truncate">{scopeLabel}</span>
            <button
              type="button"
              aria-label={`Search everywhere instead of ${scopeLabel}`}
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => setDroppedScope(scopeKey)}
              className="rounded-sm px-1 text-muted hover:bg-ink hover:text-white"
            >
              ×
            </button>
          </span>
        )}
        <input
          ref={inputRef}
          value={value}
          onChange={(e) => {
            setValue(e.target.value);
            setActive(0);
            setOpen(true);
          }}
          onFocus={() => {
            setRecent(recentSearches());
            setOpen(true);
          }}
          onBlur={() => setOpen(false)}
          onKeyDown={onKeyDown}
          placeholder={scope ? `Search in ${scopeLabel}…` : 'Search titles, tags, file names, folders…'}
          spellCheck={false}
          autoComplete="off"
          role="combobox"
          aria-expanded={open}
          aria-controls="search-list"
          className="h-full min-w-0 flex-1 bg-transparent outline-none placeholder:text-muted"
        />
        {value ? (
          <button
            type="button"
            aria-label="Clear search"
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => {
              setValue('');
              setActive(0);
              setOpen(true);
            }}
            className="text-muted hover:text-white"
          >
            ×
          </button>
        ) : (
          <kbd className="pointer-events-none hidden rounded border border-edge px-1.5 font-mono text-[10px] text-muted sm:block">
            /
          </kbd>
        )}
      </div>

      {open && (items.length > 0 || showEmpty) && (
        <div
          id="search-list"
          ref={listRef}
          role="listbox"
          // Keep focus in the input while clicking entries.
          onMouseDown={(e) => e.preventDefault()}
          className="absolute top-full left-0 z-50 mt-1 max-h-[70vh] w-full min-w-[min(36rem,calc(100vw-2rem))] overflow-y-auto rounded-md border border-edge bg-panel p-1 shadow-2xl max-sm:fixed max-sm:inset-x-4 max-sm:top-14 max-sm:w-auto max-sm:min-w-0"
        >
          {items.map((item, i) => {
            const title = heading(item, items[i - 1], i === 0);
            return (
              <div key={itemKey(item, i)}>
                {title && <div className="px-2 pt-2 pb-1 text-[11px] font-medium tracking-wide text-muted uppercase">{title}</div>}
                <div
                  data-index={i}
                  role="option"
                  aria-selected={i === current}
                  // Only real movement: a list appearing under a resting cursor
                  // fires mousemove too, and would steal Enter from the query.
                  onMouseMove={(e) => (e.movementX || e.movementY) && setActive(i)}
                  onClick={(e) => choose(item, e.metaKey || e.ctrlKey)}
                  className={`flex cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-sm ${
                    i === current ? 'bg-edge text-white' : 'text-white/85'
                  }`}
                >
                  <ItemRow
                    item={item}
                    terms={terms}
                    stale={stale}
                    onForget={(q) => {
                      forgetSearch(q);
                      setRecent(recentSearches());
                    }}
                  />
                </div>
              </div>
            );
          })}
          {showEmpty && <div className="px-2 py-4 text-center text-sm text-muted">Nothing matches “{query}”</div>}
          <div className="mt-1 border-t border-edge px-2 pt-1.5 pb-1 text-[11px] text-muted">
            <span className="font-mono">"exact phrase"</span> · <span className="font-mono">-exclude</span> ·{' '}
            <span className="font-mono">tag:</span> · <span className="font-mono">folder:</span> ·{' '}
            <span className="font-mono">kind:video</span> · <kbd>Tab</kbd> completes · <kbd>Ctrl</kbd>+<kbd>Enter</kbd> new tab
            {scope && (
              <>
                {' '}
                · <kbd>Backspace</kbd> at start searches everywhere
              </>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
