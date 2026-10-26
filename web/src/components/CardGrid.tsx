import { Link } from 'react-router';
import type { Place } from '../api';
import { FolderInput } from './FolderInput';
import { MediaCard, type CardState } from './MediaCard';
import { TagInput } from './TagInput';

/**
 * One card shape for everything that arrives.
 *
 * A URL download and a file upload differ only in where the bytes come from:
 * both progress, both can fail, both want tagging and filing while they are
 * still going. Normalising them to this means the grid, the per-card controls
 * and the apply-to-all exist once rather than twice, and a change to how
 * progress reads applies to both.
 */
export type CardItem = {
  id: string;
  title: string;
  subtitle?: string;
  thumbUrl?: string;
  state: CardState;
  percent: number;
  bytesDone: number;
  bytesTotal: number;
  error?: string;
  tags: string[];
  /** Where it lands; null means the default download folder. */
  place: Place | null;
  /** Set once the media exists, which is what makes it viewable. */
  mediaId?: number;
};

export type Shared = { tags: string[]; place: Place | null };

export function CardGrid({
  heading,
  items,
  shared,
  onRetag,
  onShared,
  onCancel,
  onRetry,
  onClear,
}: {
  heading: string;
  items: CardItem[];
  shared: Shared;
  onRetag: (id: string, next: { tags?: string[]; place?: Place }) => void;
  onShared: (next: { tags?: string[]; place?: Place }) => void;
  /** Omitted where there is nothing to cancel. */
  onCancel?: (id: string) => void;
  onRetry?: (id: string) => void;
  onClear?: () => void;
}) {
  if (items.length === 0) return null;

  const finished = items.filter(
    (i) => i.state === 'done' || i.state === 'failed' || i.state === 'cancelled',
  ).length;

  return (
    <section className="mt-6">
      <div className="mb-2 flex items-center gap-3">
        <h2 className="text-sm font-medium">
          {heading} · {finished} of {items.length} finished
        </h2>
        {onClear && finished > 0 && (
          <button onClick={onClear} className="ml-auto text-xs text-muted hover:text-white">
            Clear finished
          </button>
        )}
      </div>

      <div className="grid grid-cols-[repeat(auto-fill,minmax(280px,1fr))] gap-3">
        {items.map((item) => (
          <Card key={item.id} item={item} onRetag={onRetag} onCancel={onCancel} onRetry={onRetry} />
        ))}
      </div>

      {/* Under the cards, and only once there is a card: until something is
          arriving there is nothing to tag, and an idle pair of inputs is just
          furniture. */}
      <div className="mt-3 grid gap-2 rounded-lg border border-dashed border-edge bg-panel/60 p-3 sm:grid-cols-[1fr_260px]">
        <TagInput
          tags={shared.tags}
          onChange={(tags) => onShared({ tags })}
          compact
          placeholder="Tags for all of these…"
        />
        <FolderInput
          value={shared.place}
          placeholder="Folder for all of these…"
          onCommit={(place) => place && onShared({ place })}
        />
      </div>
    </section>
  );
}

function Card({
  item,
  onRetag,
  onCancel,
  onRetry,
}: {
  item: CardItem;
  onRetag: (id: string, next: { tags?: string[]; place?: Place }) => void;
  onCancel?: (id: string) => void;
  onRetry?: (id: string) => void;
}) {
  const terminal = item.state === 'done' || item.state === 'failed' || item.state === 'cancelled';

  let action: React.ReactNode = null;
  if (item.mediaId) {
    action = (
      <Link to={`/m/${item.mediaId}`} className="rounded-md border border-edge px-2 py-1 text-sm hover:bg-edge">
        View
      </Link>
    );
  } else if (!terminal && onCancel) {
    action = (
      <button
        onClick={() => onCancel(item.id)}
        className="rounded-md border border-edge px-2 py-1 text-sm text-muted hover:bg-edge hover:text-white"
      >
        Cancel
      </button>
    );
  } else if ((item.state === 'failed' || item.state === 'cancelled') && onRetry) {
    action = (
      <button
        onClick={() => onRetry(item.id)}
        className="rounded-md border border-edge px-2 py-1 text-sm hover:bg-edge"
      >
        Retry
      </button>
    );
  }

  return (
    <MediaCard
      title={item.title}
      subtitle={item.subtitle}
      thumbUrl={item.thumbUrl}
      state={item.state}
      percent={item.percent}
      bytesDone={item.bytesDone}
      bytesTotal={item.bytesTotal}
      error={item.error}
      controls={
        <>
          <TagInput tags={item.tags} onChange={(tags) => onRetag(item.id, { tags })} compact />
          <FolderInput
            value={item.place}
            placeholder="Default download folder"
            onCommit={(place) => place && onRetag(item.id, { place })}
          />
        </>
      }
      action={action}
    />
  );
}
