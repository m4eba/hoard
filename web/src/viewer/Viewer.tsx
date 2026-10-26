import { useCallback, useEffect, useRef, useState } from 'react';
import { useNavigate, useParams } from 'react-router';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { getMedia, requestStoryboard, type Media } from '../api';
import { useFifflyLink } from '../fiffly';
import { useZoomPan } from './useZoomPan';
import { VideoControls } from './VideoControls';
import { MediaDetails } from './MediaDetails';
import { capture, paintedRect, screenshotName, type Drawable, type Rect } from './capture';
import { RegionSelect } from './RegionSelect';
import { ScreenshotPreview } from './ScreenshotPreview';
import { StoryboardOverlay } from './StoryboardOverlay';

/**
 * Full-viewport media viewer.
 *
 * One transform layer sits between the stage and the media, so an <img> and a
 * <video> get identical zoom and pan behaviour and the gesture handling is
 * written once.
 */
export function Viewer() {
  const { id = '' } = useParams();
  const mediaId = Number(id);
  const navigate = useNavigate();
  const { data, isLoading, error } = useQuery({
    queryKey: ['media-item', mediaId],
    queryFn: () => getMedia(mediaId),
  });

  // Back where you came from, or to the library when the viewer was opened
  // directly and there is nothing to go back to.
  const close = () => (window.history.state?.idx > 0 ? navigate(-1) : navigate('/library'));

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !document.fullscreenElement) close();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div className="fixed inset-0 z-50 bg-black">
      {isLoading && <Centered>Loading…</Centered>}
      {error && <Centered>{(error as Error).message}</Centered>}
      {data && <Stage key={data.id} media={data} />}

      <button
        onClick={close}
        className="absolute top-3 right-3 z-30 rounded-full bg-black/50 p-2 text-white/90 backdrop-blur hover:bg-black/70"
        aria-label="Close"
      >
        <svg width="20" height="20" viewBox="0 0 24 24" fill="currentColor" aria-hidden>
          <path d="M19 6.41 17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z" />
        </svg>
      </button>

      {data && <MediaDetails media={data} />}

      {data && (
        <div className="pointer-events-none absolute top-3 left-3 z-30 max-w-[50vw]">
          <p className="truncate rounded bg-black/50 px-3 py-1.5 text-sm text-white/90 backdrop-blur">
            {data.title || data.filename}
          </p>
        </div>
      )}
    </div>
  );
}

function Stage({ media: item }: { media: Media }) {
  const queryClient = useQueryClient();
  const video = useRef<HTMLVideoElement | null>(null);
  // Whichever element is showing. One ref, because everything downstream —
  // capture, measurement — only cares that it is drawable.
  const media = useRef<Drawable | null>(null);
  const { stageRef, layerRef, handlers } = useZoomPan();

  const fiffly = useFifflyLink(item.id);
  const [selecting, setSelecting] = useState(false);
  const [shot, setShot] = useState<{ blob: Blob; name: string } | null>(null);
  const [error, setError] = useState('');
  const [unplayable, setUnplayable] = useState(false);
  const [board, setBoard] = useState(false);
  const storyboard = item.storyboard;

  const toggleStoryboard = useCallback(() => {
    if (item.kind !== 'video') return;
    if (storyboard.state === 'ready') {
      setBoard((b) => !b);
      return;
    }
    if (storyboard.state === 'pending') return;
    // Built on request rather than at import: it is a hundred seeks into the
    // file, and most videos are never scrubbed through.
    void requestStoryboard(item.id)
      .then(() => queryClient.invalidateQueries({ queryKey: ['media-item', item.id] }))
      .catch((err) => setError((err as Error).message));
  }, [item.id, item.kind, queryClient, storyboard.state]);

  const take = useCallback(
    async (region: Rect) => {
      const el = media.current;
      if (!el) return;
      setError('');
      try {
        const blob = await capture(el, region);
        setShot({ blob, name: screenshotName(item.title || item.filename) });
      } catch (err) {
        setError((err as Error).message);
      }
    },
    [item.filename, item.title],
  );

  /** Everything currently on screen, zoom and pan included. */
  const captureVisible = useCallback(() => {
    const stage = stageRef.current;
    if (!stage) return;
    const box = stage.getBoundingClientRect();
    void take({ x: box.x, y: box.y, width: box.width, height: box.height });
  }, [stageRef, take]);

  // Where the press started, so a pan that happens to end over the media is
  // not mistaken for a click on it. The old test was "are we zoomed", which
  // meant playback could not be toggled at all once you had zoomed in.
  const pressAt = useRef<{ x: number; y: number } | null>(null);
  const toggleTimer = useRef<number | null>(null);

  const cancelToggle = useCallback(() => {
    if (toggleTimer.current) window.clearTimeout(toggleTimer.current);
    toggleTimer.current = null;
  }, []);

  useEffect(() => cancelToggle, [cancelToggle]);

  const togglePlayback = useCallback(
    (e: React.MouseEvent) => {
      if (item.kind !== 'video' || !video.current) return;
      // The overlays own their own clicks.
      if (selecting || shot || board) return;
      const from = pressAt.current;
      if (from && Math.hypot(e.clientX - from.x, e.clientY - from.y) > 5) return;

      // Held briefly, because a double click means zoom and its first click
      // must not have already started playback.
      cancelToggle();
      toggleTimer.current = window.setTimeout(() => {
        const el = video.current;
        if (!el) return;
        if (el.paused) void el.play().catch(() => {});
        else el.pause();
      }, 220);
    },
    [item.kind, board, cancelToggle, selecting, shot],
  );

  const startSelecting = useCallback(() => {
    // A moving picture makes a moving target, and the frame that ends up in
    // the shot would not be the one that was being framed.
    video.current?.pause();
    setSelecting(true);
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      if (target && /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName)) return;
      if (e.key === 's') captureVisible();
      if (e.key === 'r') startSelecting();
      if (e.key === 'b') toggleStoryboard();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [captureVisible, startSelecting, toggleStoryboard]);

  // The selection is clamped to the picture, not the window: a region dragged
  // over the letterboxing would capture nothing.
  const bounds = (media.current && paintedRect(media.current)) ?? {
    x: 0,
    y: 0,
    width: window.innerWidth,
    height: window.innerHeight,
  };

  return (
    <div
      ref={stageRef}
      className="absolute inset-0 overflow-hidden"
      // touch-action none: the browser must not claim the gestures for
      // scrolling or its own pinch-zoom.
      style={{ touchAction: 'none' }}
      {...handlers}
      onPointerDown={(e) => {
        pressAt.current = { x: e.clientX, y: e.clientY };
        handlers.onPointerDown(e);
      }}
      onClick={togglePlayback}
      onDoubleClick={(e) => {
        // The first click of a double click has already been scheduled; the
        // gesture is a zoom, not a play.
        cancelToggle();
        handlers.onDoubleClick(e);
      }}
    >
      <div
        ref={layerRef}
        className="flex h-full w-full items-center justify-center will-change-transform"
        style={{ transformOrigin: '0 0' }}
      >
        {item.kind === 'image' ? (
          <img
            ref={(el) => {
              media.current = el;
            }}
            src={item.url}
            alt={item.title || item.filename}
            // h/w-full rather than max-*: the requirement is to use the whole
            // viewport, and max-* only ever shrinks — a small source would sit
            // tiny in the middle of a large screen. object-contain keeps the
            // aspect ratio while scaling up to fit.
            className="h-full w-full select-none object-contain"
            draggable={false}
          />
        ) : (
          <video
            ref={(el) => {
              video.current = el;
              media.current = el;
            }}
            src={item.url}
            poster={item.thumbUrl}
            onError={() => setUnplayable(true)}
            className="h-full w-full select-none object-contain"
            // No `controls`: that attribute is what draws the browser's own
            // chrome, including the large centred play button on pause.
            playsInline
            preload="metadata"
            autoPlay
          />
        )}
      </div>

      {unplayable && (
        <div className="absolute inset-x-0 top-1/3 z-20 mx-auto max-w-md rounded-lg bg-black/80 p-4 text-center text-sm">
          <p>This browser cannot play this file{item.videoCodec ? ` (${item.videoCodec})` : ''}.</p>
          <a href={item.url} download={item.filename} className="mt-2 inline-block text-accent underline">
            Download it to play locally
          </a>
        </div>
      )}

      {item.kind === 'video' && !selecting && (
        <VideoControls
          video={video}
          container={stageRef}
          storyboard={storyboard.state === 'ready' ? storyboard : undefined}
        />
      )}

      {board && storyboard.state === 'ready' && (
        <StoryboardOverlay
          storyboard={storyboard}
          video={video}
          onClose={() => setBoard(false)}
        />
      )}

      {!selecting && !shot && (
        <div className="absolute top-3 right-40 z-30 flex gap-1.5">
          {item.kind === 'video' && (
            <ToolButton
              onClick={toggleStoryboard}
              label={
                storyboard.state === 'pending'
                  ? 'Building storyboard…'
                  : storyboard.state === 'ready'
                    ? 'Storyboard (b)'
                    : 'Build storyboard (b)'
              }
            >
              <svg
                width="18"
                height="18"
                viewBox="0 0 24 24"
                fill="currentColor"
                aria-hidden
                className={storyboard.state === 'pending' ? 'animate-pulse' : ''}
              >
                <path d="M3 3h8v8H3V3zm10 0h8v8h-8V3zM3 13h8v8H3v-8zm10 0h8v8h-8v-8z" />
              </svg>
            </ToolButton>
          )}
          <ToolButton onClick={captureVisible} label="Screenshot the visible view (s)">
            <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor" aria-hidden>
              <path d="M9 3 7.2 5H4a2 2 0 0 0-2 2v11a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V7a2 2 0 0 0-2-2h-3.2L15 3H9zm3 5a5 5 0 1 1 0 10 5 5 0 0 1 0-10zm0 2a3 3 0 1 0 0 6 3 3 0 0 0 0-6z" />
            </svg>
          </ToolButton>
          <ToolButton onClick={startSelecting} label="Screenshot a region (r)">
            <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor" aria-hidden>
              <path d="M7 2h2v3H7V2zm0 17h2v3H7v-3zM2 7h3v2H2V7zm17 0h3v2h-3V7zM2 15h3v2H2v-2zm17 0h3v2h-3v-2zM15 2h2v3h-2V2zm0 17h2v3h-2v-3zM7 7h10v10H7V7z" />
            </svg>
          </ToolButton>
          {fiffly && (
            <a
              href={fiffly}
              target="_blank"
              rel="noreferrer"
              onClick={(e) => e.stopPropagation()}
              aria-label="Open in fiffly"
              title="Open in fiffly"
              className={toolClass}
            >
              <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor" aria-hidden>
                <path d="M9.64 7.64a3 3 0 1 0-1.27 1.54L10.59 12l-2.22 2.82a3 3 0 1 0 1.27 1.54L12 13.4l7 8.6h3v-1L9.64 7.64zM7 8a1 1 0 1 1 0-2 1 1 0 0 1 0 2zm0 10a1 1 0 1 1 0-2 1 1 0 0 1 0 2zm6.4-9.3L19 2h3v1l-6.2 7.6-2.4-1.9z" />
              </svg>
            </a>
          )}
        </div>
      )}

      {selecting && (
        <RegionSelect
          bounds={bounds}
          onCapture={(region) => {
            setSelecting(false);
            void take(region);
          }}
          onCancel={() => setSelecting(false)}
        />
      )}

      {shot && (
        <ScreenshotPreview
          blob={shot.blob}
          name={shot.name}
          source={item}
          onClose={() => setShot(null)}
        />
      )}

      {error && (
        <p className="absolute inset-x-0 bottom-24 z-50 text-center text-sm text-red-400">
          {error}
        </p>
      )}
    </div>
  );
}

const toolClass = 'rounded-full bg-black/50 p-2 text-white/90 backdrop-blur hover:bg-black/70';

function ToolButton({
  onClick,
  label,
  children,
}: {
  onClick: () => void;
  label: string;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={(e) => {
        // The stage below toggles playback on click.
        e.stopPropagation();
        onClick();
      }}
      aria-label={label}
      title={label}
      className={toolClass}
    >
      {children}
    </button>
  );
}

function Centered({ children }: { children: React.ReactNode }) {
  return (
    <div className="absolute inset-0 flex items-center justify-center text-muted">{children}</div>
  );
}
