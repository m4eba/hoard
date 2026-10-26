import { Link } from 'react-router';
import { formatDate } from '../format';
import { formatBytes, formatDuration, type Folder, type ListItem } from '../api';
import { folderName, libraryUrl } from '../places';
import { startMediaDrag, useMoveDrop } from './dnd';
import { MediaMenu } from './MediaMenu';
import { PreviewFrame, StoryboardBadge, useHoverPreview } from './TilePreview';
import type { ViewMode } from './viewMode';
import type { SelectionState } from './useSelection';

/**
 * The library, in one of three layouts, with the current folder's subfolders
 * leading the grid the way a file manager shows them.
 *
 * Every tile is 16:9, whatever the media is. Letting each keep its own ratio
 * read as a mistake: grid rows stretch to their tallest member, so one square
 * image left every video beside it sitting in dead space with its tags
 * floating mid-tile.
 */
export function MediaGrid({
  items,
  folders = [],
  emptyMessage,
  view = 'grid',
  selection,
}: {
  items: ListItem[];
  /** Subfolders to show ahead of the media. */
  folders?: Folder[];
  emptyMessage: string;
  view?: ViewMode;
  /** Omitted where picking files makes no sense; the grid is then read-only. */
  selection?: SelectionState;
}) {
  if (items.length === 0 && folders.length === 0) {
    return <p className="px-1 py-16 text-center text-sm text-muted">{emptyMessage}</p>;
  }

  // A modifier-click picks instead of opening, so the mouse can do the whole
  // job without first arming a mode.
  const pick = (item: ListItem) => (e: React.MouseEvent) => {
    if (!selection) return;
    if (!e.shiftKey && !e.ctrlKey && !e.metaKey) return;
    e.preventDefault();
    selection.toggle(item.id, e.shiftKey);
  };

  // Dragging a selected tile carries the whole hand-picked selection with it;
  // dragging any other tile carries just that one.
  const drag = (item: ListItem) => (e: React.DragEvent) => {
    const picked = selection && !selection.allMatching && selection.ids.has(item.id);
    startMediaDrag(e, picked ? [...selection.ids] : [item.id]);
  };

  if (view === 'list') {
    return (
      <div className="divide-y divide-edge overflow-hidden rounded-lg border border-edge">
        {folders.map((f) => (
          <FolderRow key={f.path} folder={f} />
        ))}
        {items.map((item) => (
          <Row
            key={item.id}
            item={item}
            selection={selection}
            selected={selection?.isSelected(item.id) ?? false}
            onPick={pick(item)}
            onDragStart={drag(item)}
          />
        ))}
      </div>
    );
  }

  // Large is half again the width of the default, which is what "50% larger"
  // means for a grid: fewer, bigger tiles rather than the same tiles spaced out.
  const min = view === 'large' ? 270 : 180;
  return (
    <div
      className="grid gap-3"
      style={{ gridTemplateColumns: `repeat(auto-fill, minmax(${min}px, 1fr))` }}
    >
      {folders.map((f) => (
        <FolderTile key={f.path} folder={f} />
      ))}
      {items.map((item) => (
        <Tile
          key={item.id}
          item={item}
          large={view === 'large'}
          selection={selection}
          selected={selection?.isSelected(item.id) ?? false}
          onPick={pick(item)}
          onDragStart={drag(item)}
        />
      ))}
    </div>
  );
}

// Two chips plus a counter is what fits on one line at the narrowest tile
// width. A third pushed the row to wrap, which ate a third of the thumbnail.
const VISIBLE_TAGS = 2;

// Below this, cropping to 16:9 would cut away too much to be worth it — a
// portrait photo becomes a horizontal slice of itself. Those are letterboxed
// instead: still a uniform tile, but the whole picture.
const CROPPABLE_RATIO = 1.2;

function Tile({
  item,
  large,
  selection,
  selected,
  onPick,
  onDragStart,
}: {
  item: ListItem;
  large: boolean;
  selection?: SelectionState;
  selected: boolean;
  onPick: (e: React.MouseEvent) => void;
  onDragStart: (e: React.DragEvent) => void;
}) {
  const ratio = item.width && item.height ? item.width / item.height : 16 / 9;
  const croppable = ratio >= CROPPABLE_RATIO;
  const tags = item.tags ?? [];
  // A larger tile has room for more before the row fills up.
  const limit = large ? VISIBLE_TAGS + 2 : VISIBLE_TAGS;
  const shown = tags.slice(0, limit);
  const hidden = tags.length - shown.length;
  const preview = useHoverPreview(item.storyboard);
  const previewing = preview.ready && preview.frame !== null;

  return (
    // A plain container, not a link: the tag chips are links themselves, and
    // an anchor inside an anchor is invalid — the browser closes the outer one
    // early, which breaks both.
    <div
      className={`group relative overflow-hidden rounded-lg border bg-panel ${
        selected ? 'border-accent ring-1 ring-accent' : 'border-edge'
      }`}
    >
      {selection && <Check selected={selected} onToggle={selection.toggle} id={item.id} label={item.title} />}
      <Link
        to={`/m/${item.id}`}
        className="block"
        title={item.title || item.filename}
        onClick={onPick}
        draggable
        onDragStart={onDragStart}
      >
        <div className="relative flex aspect-video items-center justify-center bg-black/40" {...preview.handlers}>
          {item.thumbUrl ? (
            <img
              src={item.thumbUrl}
              alt={item.title || item.filename}
              loading="lazy"
              draggable={false}
              className={`h-full w-full transition group-hover:opacity-90 ${
                croppable ? 'object-cover' : 'object-contain'
              }`}
            />
          ) : (
            // No thumbnail yet: the worker may still be catching up, and a
            // placeholder is better than a broken image.
            <span className="text-xs text-muted">{item.kind === 'video' ? 'no preview' : item.filename}</span>
          )}
          {previewing && <PreviewFrame storyboard={preview.ready!} frame={preview.frame!} />}
        </div>

        {item.title && (
          // Both the title and the tick appear on hover, and both live at the
          // top of the tile — so the title starts after the tick rather than
          // underneath it.
          <div
            className={`pointer-events-none absolute inset-x-0 top-0 truncate bg-gradient-to-b from-black/80 to-transparent py-1.5 pb-6 text-xs text-white/90 opacity-0 transition group-hover:opacity-100 ${
              selection ? 'pr-2 pl-9' : 'px-2'
            }`}
          >
            {item.title}
          </div>
        )}
      </Link>

      {/* Outside the link, so the buttons are not buttons inside an anchor;
          the duration lets clicks fall through to the link. The menu button
          stays through the hover preview: hiding it there would make it
          unreachable, since pointing at the tile is what starts the preview. */}
      <div className="pointer-events-none absolute top-1.5 right-1.5 flex items-center gap-1">
        {item.kind === 'video' && !previewing && (
          <>
            <span className="pointer-events-auto">
              <StoryboardBadge item={item} />
            </span>
            <span className="rounded bg-black/70 px-1.5 py-0.5 text-[11px] tabular-nums text-white/90">
              {formatDuration(item.duration)}
            </span>
          </>
        )}
        <span className="pointer-events-auto">
          <MediaMenu item={item} className="opacity-0 group-hover:opacity-100 focus-visible:opacity-100" />
        </span>
      </div>

      {shown.length > 0 && !previewing && (
        // Outside the link above, so each chip is its own navigation target.
        // nowrap with clipping, so the row can never grow to a second line
        // however long a tag is; each chip is capped and truncated, and the
        // counter is protected from shrinking so it always stays readable.
        <div className="pointer-events-none absolute inset-x-0 bottom-0 flex flex-nowrap items-center gap-1 overflow-hidden bg-gradient-to-t from-black/85 to-transparent px-1.5 pt-6 pb-1.5">
          {shown.map((tag) => (
            <TagChip key={tag} tag={tag} className="max-w-[45%] shrink" />
          ))}
          {hidden > 0 && <Overflow item={item} hidden={hidden} limit={limit} />}
        </div>
      )}
    </div>
  );
}

/** One row of the list view: small thumbnail, name, tags, then the details. */
function Row({
  item,
  selection,
  selected,
  onPick,
  onDragStart,
}: {
  item: ListItem;
  selection?: SelectionState;
  selected: boolean;
  onPick: (e: React.MouseEvent) => void;
  onDragStart: (e: React.DragEvent) => void;
}) {
  const tags = item.tags ?? [];
  const shown = tags.slice(0, 4);
  const hidden = tags.length - shown.length;

  return (
    <div
      className={`group flex items-center gap-3 px-2 py-1.5 ${
        selected ? 'bg-accent/15' : 'bg-panel hover:bg-edge/50'
      }`}
    >
      {selection && (
        <input
          type="checkbox"
          checked={selected}
          onChange={() => {}}
          onClick={(e) => selection.toggle(item.id, e.shiftKey)}
          aria-label={`Select ${item.title || item.filename}`}
          className="ml-1 size-4 shrink-0 accent-accent"
        />
      )}
      <Link
        to={`/m/${item.id}`}
        className="flex min-w-0 flex-1 items-center gap-3"
        title={item.filename}
        onClick={onPick}
        draggable
        onDragStart={onDragStart}
      >
        <div className="relative h-11 w-16 shrink-0 overflow-hidden rounded bg-black/40">
          {item.thumbUrl ? (
            <img
              src={item.thumbUrl}
              alt=""
              loading="lazy"
              draggable={false}
              className="h-full w-full object-cover"
            />
          ) : (
            <span className="flex h-full items-center justify-center text-[10px] text-muted">
              —
            </span>
          )}
          {item.kind === 'video' && (
            <span className="absolute right-0.5 bottom-0.5 rounded bg-black/75 px-1 text-[10px] tabular-nums text-white/90">
              {formatDuration(item.duration)}
            </span>
          )}
        </div>
        <span className="min-w-0 flex-1 truncate text-sm">{item.title || item.filename}</span>
      </Link>

      {/* Sibling of the link, not inside it, so the chips remain their own
          navigation targets. */}
      <div className="hidden min-w-0 shrink-0 items-center gap-1 sm:flex">
        {shown.map((tag) => (
          <TagChip key={tag} tag={tag} className="max-w-32" />
        ))}
        {hidden > 0 && <Overflow item={item} hidden={hidden} limit={4} />}
      </div>

      <div className="hidden w-56 shrink-0 justify-end gap-3 text-right text-xs text-muted md:flex">
        {item.width > 0 && (
          <span className="tabular-nums">
            {item.width}×{item.height}
          </span>
        )}
        <span className="w-16 text-right tabular-nums">{formatBytes(item.size)}</span>
        <span className="tabular-nums">{formatDate(item.createdAt)}</span>
      </div>
      <MediaMenu item={item} className="shrink-0 opacity-0 group-hover:opacity-100 focus-visible:opacity-100" />
    </div>
  );
}

/**
 * The tick in the tile's corner. Hidden until hover or until something is
 * selected — a grid of empty checkboxes reads as a form, and most visits to
 * the library are for looking rather than filing.
 */
function Check({
  selected,
  id,
  label,
  onToggle,
}: {
  selected: boolean;
  id: number;
  label: string;
  onToggle: (id: number, extend?: boolean) => void;
}) {
  return (
    <label
      className={`absolute top-1.5 left-1.5 z-10 flex size-6 cursor-pointer items-center justify-center rounded bg-black/60 backdrop-blur transition ${
        selected ? 'opacity-100' : 'opacity-0 group-hover:opacity-100 focus-within:opacity-100'
      }`}
      onClick={(e) => e.stopPropagation()}
    >
      <input
        type="checkbox"
        checked={selected}
        // Click only, and a no-op onChange to keep it controlled: a checkbox
        // fires change *after* click, so handling both made a shift-click
        // extend the range and then immediately untick the tile it landed on.
        onChange={() => {}}
        onClick={(e) => onToggle(id, e.shiftKey)}
        aria-label={`Select ${label}`}
        className="size-4 accent-accent"
      />
    </label>
  );
}

function TagChip({ tag, className = '' }: { tag: string; className?: string }) {
  return (
    <Link
      to={`/search?q=${encodeURIComponent(`tag:${tag}`)}`}
      className={`pointer-events-auto truncate rounded-full bg-black/60 px-2 py-0.5 text-[11px] whitespace-nowrap text-white/90 backdrop-blur transition hover:bg-accent hover:text-ink ${className}`}
      title={`Search for ${tag}`}
    >
      {tag}
    </Link>
  );
}

function Overflow({ item, hidden, limit }: { item: ListItem; hidden: number; limit: number }) {
  return (
    <Link
      to={`/m/${item.id}`}
      className="pointer-events-auto shrink-0 rounded-full bg-black/60 px-2 py-0.5 text-[11px] whitespace-nowrap text-white/70 backdrop-blur hover:text-white"
      title={(item.tags ?? []).slice(limit).join(', ')}
    >
      +{hidden}
    </Link>
  );
}

/** A subfolder in the grid: opens on click, takes dropped tiles. */
function FolderTile({ folder }: { folder: Folder }) {
  const { over, handlers } = useMoveDrop({ rootId: folder.rootId, folder: folder.path });
  return (
    <Link
      to={libraryUrl({ rootId: folder.rootId, folder: folder.path })}
      {...handlers}
      className={`group relative block overflow-hidden rounded-lg border bg-panel ${
        over ? 'border-accent ring-2 ring-accent' : 'border-edge'
      }`}
      title={folder.path}
    >
      <div className="flex aspect-video items-center justify-center bg-black/40">
        {folder.thumbUrl ? (
          <img
            src={folder.thumbUrl}
            alt=""
            loading="lazy"
            draggable={false}
            className="h-full w-full object-cover opacity-40 transition group-hover:opacity-55"
          />
        ) : null}
        <FolderIcon className="absolute size-10 text-white/70" />
      </div>
      <div className="absolute inset-x-0 bottom-0 flex items-center gap-2 bg-gradient-to-t from-black/85 to-transparent px-2 pt-6 pb-1.5 text-xs">
        <span className="truncate font-medium text-white/95">{folderName(folder.path)}</span>
        <span className="ml-auto shrink-0 text-white/60 tabular-nums">{folder.total}</span>
      </div>
    </Link>
  );
}

function FolderRow({ folder }: { folder: Folder }) {
  const { over, handlers } = useMoveDrop({ rootId: folder.rootId, folder: folder.path });
  return (
    <Link
      to={libraryUrl({ rootId: folder.rootId, folder: folder.path })}
      {...handlers}
      className={`flex items-center gap-3 px-2 py-1.5 ${over ? 'bg-accent/20' : 'bg-panel hover:bg-edge/50'}`}
    >
      <div className="ml-7 flex h-11 w-16 shrink-0 items-center justify-center rounded bg-black/40">
        <FolderIcon className="size-6 text-white/70" />
      </div>
      <span className="min-w-0 flex-1 truncate text-sm">{folderName(folder.path)}</span>
      <span className="w-56 text-right text-xs text-muted tabular-nums">{folder.total} item{folder.total === 1 ? '' : 's'}</span>
    </Link>
  );
}

export function FolderIcon({ className = '' }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden className={className}>
      <path d="M10 4H4a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-8l-2-2z" />
    </svg>
  );
}
