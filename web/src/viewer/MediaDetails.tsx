import { useEffect, useState } from 'react';
import { useNavigate } from 'react-router';
import { useQueryClient } from '@tanstack/react-query';
import { deleteMedia, formatBytes, formatDuration, moveMedia, updateMedia, type Media } from '../api';
import { FolderInput } from '../components/FolderInput';
import { TagInput } from '../components/TagInput';
import { formatDateTime } from '../format';
import { libraryUrl } from '../places';

/**
 * The viewer's edit panel: title, description, tags and folder for the item
 * on screen, plus what the file actually is.
 *
 * Collapsed by default. The viewer is for looking at things, and a permanent
 * sidebar over the media would defeat the full-viewport display.
 */
export function MediaDetails({ media }: { media: Media }) {
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const [open, setOpen] = useState(false);
  const [tags, setTags] = useState(media.tags ?? []);
  const [title, setTitle] = useState(media.title);
  const [description, setDescription] = useState(media.description);
  const [status, setStatus] = useState('');
  const place = { rootId: media.rootId, folder: media.folder };

  useEffect(() => {
    setTags(media.tags ?? []);
    setTitle(media.title);
    setDescription(media.description);
  }, [media.id, media.tags, media.title, media.description]);

  async function save(next: Partial<{ tags: string[]; title: string; description: string }>) {
    setStatus('Saving…');
    try {
      await updateMedia(media.id, next);
      setStatus('Saved');
      // The grid shows tags and titles, so both are stale now.
      void queryClient.invalidateQueries();
      setTimeout(() => setStatus(''), 1500);
    } catch (err) {
      setStatus((err as Error).message);
    }
  }

  if (!open) {
    return (
      <button
        onClick={() => setOpen(true)}
        className="absolute top-3 right-14 z-30 rounded-full bg-black/50 px-3 py-2 text-sm text-white/90 backdrop-blur hover:bg-black/70"
      >
        Edit
      </button>
    );
  }

  return (
    <div className="absolute top-0 right-0 z-30 flex h-full w-80 max-w-full flex-col gap-3 overflow-y-auto border-l border-edge bg-panel/95 p-4 backdrop-blur">
      <div className="flex items-center">
        <h2 className="text-sm font-medium">Details</h2>
        <button onClick={() => setOpen(false)} className="ml-auto text-muted hover:text-white" aria-label="Close details">
          ✕
        </button>
      </div>

      <label className="block">
        <span className="mb-1 block text-xs text-muted">Title</span>
        <input
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          onBlur={() => title !== media.title && void save({ title })}
          className="w-full rounded-md border border-edge bg-ink px-2 py-1.5 text-sm outline-none focus:border-accent"
        />
      </label>

      <div>
        <span className="mb-1 block text-xs text-muted">Tags</span>
        {/* Same component as the import cards, so completion behaves
            identically wherever you tag something. */}
        <TagInput
          tags={tags}
          onChange={(next) => {
            setTags(next);
            void save({ tags: next });
          }}
        />
      </div>

      <div>
        <span className="mb-1 block text-xs text-muted">Folder</span>
        <FolderInput
          value={place}
          placeholder="Move to folder…"
          onCommit={async (next) => {
            if (!next) return;
            setStatus('Moving…');
            try {
              await moveMedia({ ids: [media.id] }, next);
              // The move runs in the background; the event stream refreshes
              // this panel when it lands.
              setStatus('Moving in the background');
            } catch (err) {
              setStatus((err as Error).message);
            }
          }}
        />
      </div>

      <label className="block">
        <span className="mb-1 block text-xs text-muted">Description</span>
        <textarea
          value={description}
          onChange={(e) => setDescription(e.target.value)}
          onBlur={() => description !== media.description && void save({ description })}
          rows={4}
          className="w-full resize-y rounded-md border border-edge bg-ink px-2 py-1.5 text-sm outline-none focus:border-accent"
        />
      </label>

      <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
        <dt className="text-muted">File</dt>
        <dd className="break-all">
          <button onClick={() => navigate(libraryUrl(place))} className="text-left hover:underline">
            {media.rootName}/{media.folder ? `${media.folder}/` : ''}
          </button>
          {media.filename}
        </dd>
        <dt className="text-muted">Size</dt>
        <dd>{formatBytes(media.size)}</dd>
        {media.width > 0 && (
          <>
            <dt className="text-muted">Dimensions</dt>
            <dd>
              {media.width}×{media.height}
            </dd>
          </>
        )}
        {media.kind === 'video' && (
          <>
            <dt className="text-muted">Duration</dt>
            <dd>{formatDuration(media.duration)}</dd>
            <dt className="text-muted">Codecs</dt>
            <dd>{[media.videoCodec, media.audioCodec].filter(Boolean).join(' / ') || '—'}</dd>
          </>
        )}
        <dt className="text-muted">Added</dt>
        <dd>{formatDateTime(media.createdAt)}</dd>
        <dt className="text-muted">Modified</dt>
        <dd>{formatDateTime(media.fileModifiedAt)}</dd>
      </dl>

      {media.sourceUrl && (
        <p className="text-xs break-all text-muted">
          Downloaded from{' '}
          <a href={media.sourceUrl} target="_blank" rel="noreferrer" className="underline">
            {media.sourceUrl}
          </a>
        </p>
      )}

      <div className="mt-auto flex items-center gap-2 pt-2">
        <span className="text-xs text-muted">{status}</span>
        <a
          href={media.url}
          download={media.filename}
          className="ml-auto rounded-md border border-edge px-2 py-1 text-sm hover:bg-edge"
        >
          Download
        </a>
        <button
          onClick={async () => {
            if (!confirm('Delete this file? It goes to the storage folder’s trash.')) return;
            await deleteMedia(media.id);
            void queryClient.invalidateQueries();
            navigate(-1);
          }}
          className="rounded-md border border-red-900/60 px-2 py-1 text-sm text-red-300 hover:bg-red-950/50"
        >
          Delete
        </button>
      </div>
    </div>
  );
}
