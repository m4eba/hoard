import { useEffect } from 'react';
import { formatDuration, type Storyboard } from '../api';

type ReadyBoard = Extract<Storyboard, { state: 'ready' }>;

/** One tile of the sprite sheet, cut out with a background offset. */
export function StoryboardFrame({
  storyboard,
  index,
  className = '',
  scale = 1,
}: {
  storyboard: ReadyBoard;
  index: number;
  className?: string;
  scale?: number;
}) {
  const { cols, rows, tileWidth, tileHeight, url } = storyboard;
  const col = index % cols;
  const row = Math.floor(index / cols);
  return (
    <div
      className={className}
      style={{
        width: tileWidth * scale,
        height: tileHeight * scale,
        backgroundImage: `url(${url})`,
        backgroundSize: `${cols * tileWidth * scale}px ${rows * tileHeight * scale}px`,
        backgroundPosition: `-${col * tileWidth * scale}px -${row * tileHeight * scale}px`,
      }}
    />
  );
}

/**
 * The whole video at a glance: every storyboard frame in a grid, each one a
 * jump to that moment.
 */
export function StoryboardOverlay({
  storyboard,
  video,
  onClose,
}: {
  storyboard: ReadyBoard;
  video: React.RefObject<HTMLVideoElement | null>;
  onClose: () => void;
}) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        // The viewer closes on Escape too; this one is only for the overlay.
        e.stopImmediatePropagation();
        onClose();
      }
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [onClose]);

  const current = video.current ? Math.floor(video.current.currentTime / storyboard.interval) : -1;

  return (
    <div
      className="absolute inset-0 z-40 overflow-y-auto bg-black/85 p-6 pt-16 backdrop-blur-sm"
      onClick={(e) => {
        e.stopPropagation();
        onClose();
      }}
      onPointerDown={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
    >
      <div
        className="grid justify-center gap-2"
        style={{ gridTemplateColumns: `repeat(auto-fill, ${storyboard.tileWidth}px)` }}
      >
        {Array.from({ length: storyboard.count }, (_, i) => {
          const at = i * storyboard.interval;
          return (
            <button
              key={i}
              onClick={(e) => {
                e.stopPropagation();
                const el = video.current;
                if (el) el.currentTime = at;
                onClose();
              }}
              className={`group relative overflow-hidden rounded ${
                i === current ? 'ring-2 ring-accent' : 'ring-1 ring-white/10 hover:ring-white/60'
              }`}
              title={`Jump to ${formatDuration(at)}`}
            >
              <StoryboardFrame storyboard={storyboard} index={i} />
              <span className="absolute right-1 bottom-1 rounded bg-black/75 px-1 text-[10px] text-white/90 tabular-nums">
                {formatDuration(at)}
              </span>
            </button>
          );
        })}
      </div>
    </div>
  );
}
