import { useCallback, useRef, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { moveMedia, updateMedia, upload, type Media, type Place } from '../api';
import { samePlace } from '../places';
import type { CardItem, Shared } from './CardGrid';
import { useServerEvents } from './useServerEvents';

/** One file on its way in. */
export type Upload = {
  id: string;
  name: string;
  size: number;
  sent: number;
  state: CardItem['state'];
  error?: string;
  media?: Media;
  thumbUrl?: string;
  /** Chosen on this card: staged before the file is sent, applied after. */
  tags: string[];
  place: Place | null;
};

/**
 * The upload queue behind the cards.
 *
 * Files go up one at a time. Several large uploads in parallel share the same
 * bandwidth and all finish later than they would in sequence, and every
 * progress bar crawling at once reads as a stall.
 *
 * `target` is where files land when no card or shared choice says otherwise —
 * the folder being looked at, for a drop onto the library.
 */
export function useUploads(target: Place | null = null) {
  const queryClient = useQueryClient();
  const [uploads, setUploads] = useState<Upload[]>([]);
  const [shared, setSharedState] = useState<Shared>({ tags: [], place: null });

  // Refs mirror the state so the sequential runner can read current values
  // without every progress tick rebuilding callbacks.
  const queue = useRef<Upload[]>([]);
  const sharedRef = useRef<Shared>(shared);
  const targetRef = useRef(target);
  targetRef.current = target;
  const running = useRef(false);

  const patch = useCallback((id: string, next: Partial<Upload>) => {
    queue.current = queue.current.map((u) => (u.id === id ? { ...u, ...next } : u));
    setUploads(queue.current);
  }, []);

  // The thumbnail arrives after the upload response, once the worker has
  // processed the file; the server pushes it.
  useServerEvents(
    useCallback(
      (e) => {
        if (e.type !== 'media') return;
        const entry = queue.current.find((u) => u.media?.id === e.id);
        if (entry && e.thumbUrl) patch(entry.id, { thumbUrl: e.thumbUrl });
      },
      [patch],
    ),
    uploads.length > 0,
  );

  const run = useCallback(
    async (files: File[]) => {
      if (running.current) return;
      running.current = true;
      try {
        for (const file of files) {
          const entry = queue.current.find((u) => u.name === file.name && u.state === 'queued');
          if (!entry) continue;
          patch(entry.id, { state: 'uploading' });
          try {
            // Read the choice at send time, not when the file was queued: the
            // controls exist precisely so they can be filled in while the
            // first uploads are already running.
            const now = queue.current.find((u) => u.id === entry.id) ?? entry;
            const media = await upload(
              file,
              { tags: now.tags, place: now.place ?? targetRef.current },
              (sent) => patch(entry.id, { sent }),
            );
            patch(entry.id, { state: 'done', media, sent: file.size, thumbUrl: media.thumbUrl });
          } catch (err) {
            patch(entry.id, { state: 'failed', error: (err as Error).message });
          }
        }
      } finally {
        running.current = false;
        void queryClient.invalidateQueries();
      }
    },
    [patch, queryClient],
  );

  /** Queues files and starts uploading. */
  const accept = useCallback(
    (list: FileList | File[] | null) => {
      const files = Array.from(list ?? []);
      if (files.length === 0) return;
      const entries: Upload[] = files.map((f, i) => ({
        // Name and size alone collide when the same file is added twice.
        id: `${Date.now()}-${i}-${f.name}`,
        name: f.name,
        size: f.size,
        sent: 0,
        state: 'queued',
        // A file added after the batch was tagged inherits that choice; being
        // asked to retype it for every drop would defeat the point.
        tags: [...sharedRef.current.tags],
        place: sharedRef.current.place,
      }));
      queue.current = [...queue.current, ...entries];
      setUploads(queue.current);
      void run(files);
    },
    [run],
  );

  /** Writes a card's choice through to its media, once there is some. */
  const persist = useCallback(
    (entry: Upload, movedTo?: Place) => {
      if (!entry.media) return;
      const media = entry.media;
      void updateMedia(media.id, { tags: entry.tags })
        .then(() => {
          const here = { rootId: media.rootId, folder: media.folder };
          if (movedTo && !samePlace(movedTo, here)) return moveMedia({ ids: [media.id] }, movedTo);
        })
        .then(() => queryClient.invalidateQueries())
        .catch(() => {});
    },
    [queryClient],
  );

  /** Retags or refiles one card. Staged before it lands, applied in place afterwards. */
  const retag = useCallback(
    (id: string, next: { tags?: string[]; place?: Place }) => {
      patch(id, next);
      const entry = queue.current.find((u) => u.id === id);
      if (entry) persist(entry, next.place);
    },
    [patch, persist],
  );

  /**
   * Applies a choice to the whole batch, and to files added afterwards.
   * Deliberately overwrites per-card edits: an "apply to all" that left some
   * cards alone would be worse than not offering it.
   */
  const setShared = useCallback(
    (next: { tags?: string[]; place?: Place }) => {
      const merged = { ...sharedRef.current, ...next };
      sharedRef.current = merged;
      setSharedState(merged);

      queue.current = queue.current.map((u) => ({ ...u, tags: [...merged.tags], place: merged.place }));
      setUploads(queue.current);
      queue.current.forEach((u) => persist(u, next.place));
    },
    [persist],
  );

  const clear = useCallback(() => {
    // Only finished cards go; anything still running would lose its progress.
    queue.current = queue.current.filter((u) => u.state !== 'done');
    setUploads(queue.current);
  }, []);

  /** The uploads as the shared card grid wants them. */
  const items: CardItem[] = uploads.map((u) => ({
    id: u.id,
    title: u.name,
    subtitle: u.media ? `${u.media.rootName}/${u.media.folder}` : undefined,
    thumbUrl: u.thumbUrl,
    state: u.state,
    percent: u.size > 0 ? Math.min(99, Math.floor((u.sent / u.size) * 100)) : 0,
    bytesDone: u.sent,
    bytesTotal: u.size,
    error: u.error,
    tags: u.tags,
    place: u.place ?? (u.media ? { rootId: u.media.rootId, folder: u.media.folder } : targetRef.current),
    mediaId: u.media?.id,
  }));

  return { items, accept, retag, clear, shared, setShared };
}
