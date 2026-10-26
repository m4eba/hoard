import { useEffect, useRef, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { formatDuration, requestStoryboard, type ListItem, type Storyboard } from '../api';

type Ready = Extract<Storyboard, { state: 'ready' }>;

/** How long each frame shows while a hovered tile plays through its storyboard. */
const FRAME_MS = 450;

/**
 * Hover preview for a video tile, the way a video site previews a video
 * under the cursor: the storyboard plays through on its own, and moving
 * across the tile scrubs to that point instead.
 */
export function useHoverPreview(storyboard: Storyboard | undefined) {
  const ready = storyboard?.state === 'ready' ? storyboard : undefined;
  const [frame, setFrame] = useState<number | null>(null);
  // Scrubbing pauses the autoplay until the pointer rests again.
  const lastMove = useRef(0);

  useEffect(() => {
    if (frame === null || !ready) return;
    const id = window.setInterval(() => {
      if (Date.now() - lastMove.current < 800) return;
      setFrame((f) => (f === null ? f : (f + 1) % ready.count));
    }, FRAME_MS);
    return () => window.clearInterval(id);
  }, [frame === null, ready]); // eslint-disable-line react-hooks/exhaustive-deps

  const handlers = ready
    ? {
        onMouseEnter: () => setFrame(0),
        onMouseLeave: () => setFrame(null),
        onMouseMove: (e: React.MouseEvent<HTMLElement>) => {
          const rect = e.currentTarget.getBoundingClientRect();
          const fraction = Math.min(0.999, Math.max(0, (e.clientX - rect.left) / rect.width));
          lastMove.current = Date.now();
          setFrame(Math.floor(fraction * ready.count));
        },
      }
    : {};

  return { ready, frame: ready ? frame : null, handlers };
}

/** The current frame, letterboxed into the tile, with a progress line. */
export function PreviewFrame({ storyboard, frame }: { storyboard: Ready; frame: number }) {
  const { cols, rows, tileWidth, tileHeight, url, count, interval } = storyboard;
  const col = frame % cols;
  const row = Math.floor(frame / cols);
  // Percent positions put tile n's edge on the container's edge: the image
  // is cols × wider than the box, so tile c sits at c / (cols − 1).
  const x = cols > 1 ? (col / (cols - 1)) * 100 : 0;
  const y = rows > 1 ? (row / (rows - 1)) * 100 : 0;
  const wide = tileWidth / tileHeight >= 16 / 9;

  return (
    <div className="pointer-events-none absolute inset-0 flex items-center justify-center bg-black">
      <div
        className={wide ? 'w-full' : 'h-full'}
        style={{
          aspectRatio: `${tileWidth} / ${tileHeight}`,
          backgroundImage: `url(${url})`,
          backgroundSize: `${cols * 100}% ${rows * 100}%`,
          backgroundPosition: `${x}% ${y}%`,
        }}
      />
      <span className="absolute right-1.5 bottom-2.5 rounded bg-black/75 px-1.5 py-0.5 text-[11px] text-white/90 tabular-nums">
        {formatDuration(frame * interval)}
      </span>
      <div className="absolute inset-x-0 bottom-0 h-1 bg-white/20">
        <div className="h-full bg-red-500" style={{ width: `${((frame + 1) / count) * 100}%` }} />
      </div>
    </div>
  );
}

/**
 * Shown on a video with no storyboard yet: clicking queues one. While queued
 * it pulses, so a folder full of them visibly works its way through.
 */
export function StoryboardBadge({ item }: { item: ListItem }) {
  const queryClient = useQueryClient();
  const state = item.storyboard?.state;
  if (item.kind !== 'video' || !state || state === 'ready') return null;
  const pending = state === 'pending';

  return (
    <button
      type="button"
      disabled={pending}
      onClick={(e) => {
        // The tile is a link; this must not open the viewer.
        e.preventDefault();
        e.stopPropagation();
        void requestStoryboard(item.id).then(() => queryClient.invalidateQueries({ queryKey: ['media'] }));
      }}
      title={pending ? 'Storyboard queued' : 'No storyboard yet — click to build one'}
      aria-label={pending ? 'Storyboard queued' : 'Build storyboard'}
      className={`flex size-[22px] items-center justify-center rounded bg-black/70 text-white/80 hover:bg-black/90 hover:text-white ${
        pending ? 'animate-pulse cursor-default' : ''
      }`}
    >
      <svg width="13" height="13" viewBox="0 0 24 24" fill="currentColor" aria-hidden>
        <path d="M3 3h8v8H3V3zm10 0h8v8h-8V3zM3 13h8v8H3v-8zm10 0h8v8h-8v-8z" opacity={pending ? 1 : 0.55} />
        {!pending && <path d="M2.3 3.7 3.7 2.3l18 18-1.4 1.4z" />}
      </svg>
    </button>
  );
}
