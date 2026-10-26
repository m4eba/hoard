import { Fragment } from 'react';
import { formatDuration, type Folder, type ListItem, type Place } from '../../api';
import { formatDate } from '../../format';
import { FolderIcon } from '../MediaGrid';

/** One entry in the search dropdown. */
export type Item =
  | { kind: 'search'; query: string }
  | { kind: 'recent'; query: string }
  | { kind: 'suggestion'; label: string; hint?: string; insert: string; complete: boolean }
  | { kind: 'scope'; label: string }
  | { kind: 'folder'; folder: Folder; label: string }
  | { kind: 'media'; media: ListItem; place: string }
  | { kind: 'all'; query: string };

export const itemKey = (item: Item, i: number) =>
  item.kind === 'media'
    ? `m-${item.media.id}`
    : item.kind === 'folder'
      ? `f-${item.folder.rootId}-${item.folder.path}`
      : `${item.kind}-${i}`;

/** The group heading shown above an item, if it starts a new group. */
export function heading(item: Item, prev: Item | undefined, first: boolean): string | null {
  if (prev?.kind === item.kind) return null;
  switch (item.kind) {
    case 'recent':
      return 'Recent searches';
    case 'suggestion':
      return first && !item.complete ? 'Narrow your search' : 'Suggestions';
    case 'folder':
      return 'Folders';
    case 'media':
      return 'Media';
    case 'scope':
      return 'Scope';
    default:
      return null;
  }
}

/** Marks the query's words in a title, the way a search engine bolds them. */
export function Highlight({ text, terms }: { text: string; terms: string[] }) {
  if (terms.length === 0) return <>{text}</>;
  const pattern = new RegExp(`(${terms.map((t) => t.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('|')})`, 'gi');
  return (
    <>
      {text.split(pattern).map((part, i) =>
        i % 2 ? (
          <mark key={i} className="rounded-sm bg-accent/30 text-inherit">
            {part}
          </mark>
        ) : (
          <Fragment key={i}>{part}</Fragment>
        ),
      )}
    </>
  );
}

export function ItemRow({
  item,
  terms,
  stale,
  onForget,
}: {
  item: Item;
  terms: string[];
  /** Results of an older query, shown dimmed until the new ones arrive. */
  stale: boolean;
  onForget: (query: string) => void;
}) {
  switch (item.kind) {
    case 'search':
      return (
        <>
          <SearchIcon />
          <span className="min-w-0 flex-1 truncate">
            Search for <span className="font-medium text-white">“{item.query}”</span>
          </span>
          <kbd className="text-[10px] text-muted">Enter</kbd>
        </>
      );
    case 'recent':
      return (
        <>
          <ClockIcon />
          <span className="min-w-0 flex-1 truncate">{item.query}</span>
          <button
            type="button"
            aria-label={`Remove ${item.query} from recent searches`}
            title="Remove"
            onClick={(e) => {
              e.stopPropagation();
              onForget(item.query);
            }}
            className="rounded px-1 text-muted hover:bg-edge hover:text-white"
          >
            ×
          </button>
        </>
      );
    case 'suggestion':
      return (
        <>
          <FilterIcon />
          <span className="font-mono text-xs">{item.label}</span>
          {item.hint && <span className="truncate text-xs text-muted">{item.hint}</span>}
        </>
      );
    case 'scope':
      return (
        <>
          <FolderIcon className="size-4 shrink-0 text-muted" />
          <span>Search in {item.label}</span>
        </>
      );
    case 'folder':
      return (
        <>
          <FolderIcon className="size-4 shrink-0 text-muted" />
          <span className="min-w-0 flex-1 truncate">
            <Highlight text={item.label} terms={terms} />
          </span>
          <span className="shrink-0 text-xs text-muted">{item.folder.total}</span>
        </>
      );
    case 'media':
      return <MediaRow media={item.media} place={item.place} terms={terms} stale={stale} />;
    case 'all':
      return (
        <span className="w-full text-center text-xs text-muted">See all results for “{item.query}” →</span>
      );
  }
}

function MediaRow({ media, place, terms, stale }: { media: ListItem; place: string; terms: string[]; stale: boolean }) {
  const meta = [media.kind === 'video' ? formatDuration(media.duration) : null, formatDate(media.createdAt)].filter(Boolean);
  // Where it lives goes on the second line: paths are long, and on the right
  // they pushed out the duration and date, which are what tell clips apart.
  const second = [place, media.tags.join(' · ')].filter(Boolean).join(' — ');
  return (
    <span className={`flex min-w-0 flex-1 items-center gap-2.5 ${stale ? 'opacity-50' : ''}`}>
      <span className="flex h-9 w-16 shrink-0 items-center justify-center overflow-hidden rounded bg-black/40">
        {media.thumbUrl && <img src={media.thumbUrl} alt="" className="h-full w-full object-cover" loading="lazy" />}
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate">
          <Highlight text={media.title || media.filename} terms={terms} />
        </span>
        <span className="block truncate text-[11px] text-muted" title={second}>
          <Highlight text={second} terms={terms} />
        </span>
      </span>
      <span className="shrink-0 text-xs text-muted tabular-nums">{meta.join(' · ')}</span>
    </span>
  );
}

export const placeOf = (m: ListItem): Place => ({ rootId: m.rootId, folder: m.folder });

const icon = { width: 16, height: 16, viewBox: '0 0 24 24', fill: 'currentColor', 'aria-hidden': true } as const;

export const SearchIcon = () => (
  <svg {...icon} className="shrink-0 text-muted">
    <path d="M15.5 14h-.79l-.28-.27A6.47 6.47 0 0 0 16 9.5 6.5 6.5 0 1 0 9.5 16a6.47 6.47 0 0 0 4.23-1.57l.27.28v.79l5 4.99L20.49 19l-4.99-5zm-6 0C7.01 14 5 11.99 5 9.5S7.01 5 9.5 5 14 7.01 14 9.5 11.99 14 9.5 14z" />
  </svg>
);

export const Spinner = () => (
  <svg {...icon} fill="none" className="shrink-0 animate-spin text-muted">
    <circle cx="12" cy="12" r="9" stroke="currentColor" strokeWidth="2.5" strokeOpacity="0.25" />
    <path d="M21 12a9 9 0 0 0-9-9" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" />
  </svg>
);

const ClockIcon = () => (
  <svg {...icon} className="shrink-0 text-muted">
    <path d="M13 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zm0 16a7 7 0 1 1 0-14 7 7 0 0 1 0 14zm.5-11H12v6l5.2 3.1.8-1.3-4.5-2.7V8z" />
  </svg>
);

const FilterIcon = () => (
  <svg {...icon} className="shrink-0 text-muted">
    <path d="M10 18h4v-2h-4v2zM3 6v2h18V6H3zm3 7h12v-2H6v2z" />
  </svg>
);
