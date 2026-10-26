import { useEffect, useRef, useState } from 'react';
import { formatDuration, type Storyboard } from '../api';
import { StoryboardFrame } from './StoryboardOverlay';

type ReadyBoard = Extract<Storyboard, { state: 'ready' }>;

/**
 * The player's own control bar.
 *
 * The <video> is rendered without the `controls` attribute, because the big
 * centred play button that appears on pause is the browser's built-in chrome
 * and the only reliable way to be rid of it is not to ask for it. That means
 * everything here — scrubber, time, volume, fullscreen — is ours.
 *
 * Clicking the video toggles playback with no overlay of any kind: no button,
 * no flash, nothing drawn over the frame.
 */
export function VideoControls({
  video,
  container,
  storyboard,
}: {
  video: React.RefObject<HTMLVideoElement | null>;
  container: React.RefObject<HTMLElement | null>;
  /** When present, hovering the scrubber previews the frame under the pointer. */
  storyboard?: ReadyBoard;
}) {
  const [playing, setPlaying] = useState(false);
  const [time, setTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const [volume, setVolume] = useState(1);
  const [muted, setMuted] = useState(false);
  const [visible, setVisible] = useState(true);
  const [fullscreen, setFullscreen] = useState(false);
  const hideTimer = useRef<number | null>(null);

  // Keep the bar in step with the element, which can also be driven by the
  // keyboard or by the browser itself.
  useEffect(() => {
    const el = video.current;
    if (!el) return;
    const onPlay = () => setPlaying(true);
    const onPause = () => setPlaying(false);
    const onTime = () => setTime(el.currentTime);
    const onMeta = () => setDuration(el.duration || 0);
    const onVolume = () => {
      setVolume(el.volume);
      setMuted(el.muted);
    };
    // Seed from the element, not from `false`: with autoplay the play event
    // can land before this effect runs, and the button would then say Play
    // over a playing video.
    setPlaying(!el.paused);
    onVolume();
    el.addEventListener('play', onPlay);
    el.addEventListener('pause', onPause);
    el.addEventListener('timeupdate', onTime);
    el.addEventListener('loadedmetadata', onMeta);
    el.addEventListener('volumechange', onVolume);
    onMeta();
    return () => {
      el.removeEventListener('play', onPlay);
      el.removeEventListener('pause', onPause);
      el.removeEventListener('timeupdate', onTime);
      el.removeEventListener('loadedmetadata', onMeta);
      el.removeEventListener('volumechange', onVolume);
    };
  }, [video]);

  // Auto-hide, so a paused frame is unobstructed after a moment. Any pointer
  // movement brings it back.
  useEffect(() => {
    const show = () => {
      setVisible(true);
      if (hideTimer.current) window.clearTimeout(hideTimer.current);
      hideTimer.current = window.setTimeout(() => setVisible(false), 2500);
    };
    show();
    const host = container.current ?? document;
    host.addEventListener('pointermove', show);
    host.addEventListener('pointerdown', show);
    return () => {
      host.removeEventListener('pointermove', show);
      host.removeEventListener('pointerdown', show);
      if (hideTimer.current) window.clearTimeout(hideTimer.current);
    };
  }, [container]);

  useEffect(() => {
    const onChange = () => setFullscreen(Boolean(document.fullscreenElement));
    document.addEventListener('fullscreenchange', onChange);
    return () => document.removeEventListener('fullscreenchange', onChange);
  }, []);

  // Keyboard: the shortcuts a player is expected to have.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const el = video.current;
      if (!el) return;
      // Never steal keys from a field the user is typing in.
      const target = e.target as HTMLElement | null;
      if (target && /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName)) return;

      switch (e.key) {
        case ' ':
        case 'k':
          e.preventDefault();
          void toggle(el);
          break;
        case 'ArrowLeft':
          e.preventDefault();
          el.currentTime = Math.max(0, el.currentTime - 5);
          break;
        case 'ArrowRight':
          e.preventDefault();
          el.currentTime = Math.min(el.duration || 0, el.currentTime + 5);
          break;
        case 'm':
          el.muted = !el.muted;
          break;
        case 'f':
          void toggleFullscreen(container.current);
          break;
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [container, video]);

  const seek = (fraction: number) => {
    const el = video.current;
    if (!el || !el.duration) return;
    el.currentTime = fraction * el.duration;
  };

  return (
    <div
      className={`pointer-events-none absolute inset-x-0 bottom-0 z-20 transition-opacity duration-200 ${
        visible || !playing ? 'opacity-100' : 'opacity-0'
      }`}
    >
      <div
        className="pointer-events-auto bg-gradient-to-t from-black/80 to-transparent px-4 pt-10 pb-4"
        // The bar lives inside the stage, and the stage toggles playback on
        // click and zooms on double click. Without this, pressing Play
        // toggled twice and came out level, and grabbing the scrubber started
        // playback.
        onPointerDown={(e) => e.stopPropagation()}
        onClick={(e) => e.stopPropagation()}
        onDoubleClick={(e) => e.stopPropagation()}
      >
        <Scrubber
          value={duration ? time / duration : 0}
          onSeek={seek}
          duration={duration}
          storyboard={storyboard}
        />
        <div className="mt-2 flex items-center gap-3 text-sm text-white">
          <button
            onClick={() => video.current && void toggle(video.current)}
            className="rounded p-1 hover:bg-white/15"
            aria-label={playing ? 'Pause' : 'Play'}
          >
            {playing ? <PauseIcon /> : <PlayIcon />}
          </button>

          <span className="tabular-nums text-xs text-white/80">
            {formatDuration(time)} / {formatDuration(duration)}
          </span>

          <div className="ml-auto flex items-center gap-2">
            <button
              onClick={() => {
                const el = video.current;
                if (el) el.muted = !el.muted;
              }}
              className="rounded p-1 hover:bg-white/15"
              aria-label={muted ? 'Unmute' : 'Mute'}
            >
              {muted || volume === 0 ? <MutedIcon /> : <SoundIcon />}
            </button>
            <input
              type="range"
              min={0}
              max={1}
              step={0.01}
              value={muted ? 0 : volume}
              onChange={(e) => {
                const el = video.current;
                if (!el) return;
                el.volume = Number(e.target.value);
                el.muted = Number(e.target.value) === 0;
              }}
              className="h-1 w-20 accent-white"
              aria-label="Volume"
            />
            <button
              onClick={() => void toggleFullscreen(container.current)}
              className="rounded p-1 hover:bg-white/15"
              aria-label={fullscreen ? 'Exit fullscreen' : 'Fullscreen'}
            >
              <FullscreenIcon />
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

/**
 * A wide hit area, because a 4px line is not something anyone can grab.
 * Hovering shows the time under the pointer, and the frame there once a
 * storyboard exists.
 */
function Scrubber({
  value,
  onSeek,
  duration,
  storyboard,
}: {
  value: number;
  onSeek: (fraction: number) => void;
  duration: number;
  storyboard?: ReadyBoard;
}) {
  const track = useRef<HTMLDivElement | null>(null);
  const dragging = useRef(false);
  const [hover, setHover] = useState<{ fraction: number; x: number; width: number } | null>(null);

  const fractionAt = (clientX: number) => {
    const el = track.current;
    if (!el) return null;
    const rect = el.getBoundingClientRect();
    return { fraction: Math.min(1, Math.max(0, (clientX - rect.left) / rect.width)), rect };
  };

  const seekTo = (clientX: number) => {
    const at = fractionAt(clientX);
    if (at) onSeek(at.fraction);
  };

  const preview = hover && duration > 0 ? hover.fraction * duration : null;
  const previewWidth = storyboard ? storyboard.tileWidth : 56;

  return (
    <div
      ref={track}
      className="group relative -my-2 cursor-pointer py-2"
      onPointerDown={(e) => {
        dragging.current = true;
        try {
          (e.target as Element).setPointerCapture?.(e.pointerId);
        } catch {
          // A pointer the browser no longer tracks throws here, and losing
          // capture is not a reason to swallow the seek.
        }
        seekTo(e.clientX);
      }}
      onPointerMove={(e) => {
        const at = fractionAt(e.clientX);
        if (at) setHover({ fraction: at.fraction, x: e.clientX - at.rect.left, width: at.rect.width });
        if (dragging.current) seekTo(e.clientX);
      }}
      onPointerLeave={() => setHover(null)}
      onPointerUp={() => (dragging.current = false)}
      role="slider"
      aria-label="Seek"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.round(value * 100)}
      tabIndex={0}
    >
      <div className="h-1 w-full rounded-full bg-white/25 transition-all group-hover:h-1.5">
        <div
          className="h-full rounded-full bg-white"
          style={{ width: `${Math.min(100, value * 100)}%` }}
        />
      </div>
      {hover && preview !== null && (
        <div
          className="pointer-events-none absolute bottom-5 flex flex-col items-center"
          style={{
            // Kept inside the track, so the preview never hangs off the edge.
            left: Math.min(Math.max(hover.x - previewWidth / 2, 0), hover.width - previewWidth),
            width: previewWidth,
          }}
        >
          {storyboard && (
            <StoryboardFrame
              storyboard={storyboard}
              index={Math.min(storyboard.count - 1, Math.floor(preview / storyboard.interval))}
              className="rounded border border-white/30 shadow-lg"
            />
          )}
          <span className="mt-1 rounded bg-black/80 px-1.5 py-0.5 text-[11px] text-white tabular-nums">
            {formatDuration(preview)}
          </span>
        </div>
      )}
    </div>
  );
}

async function toggle(el: HTMLVideoElement) {
  if (el.paused) {
    try {
      await el.play();
    } catch {
      // Autoplay policies reject play() that was not user-driven; the button
      // press is, so this is only reached in odd cases and pausing is fine.
    }
  } else {
    el.pause();
  }
}

async function toggleFullscreen(el: HTMLElement | null) {
  if (!el) return;
  try {
    if (document.fullscreenElement) await document.exitFullscreen();
    else await el.requestFullscreen();
  } catch {
    // Denied or unsupported: not worth interrupting playback over.
  }
}

const iconProps = {
  width: 18,
  height: 18,
  viewBox: '0 0 24 24',
  fill: 'currentColor',
  'aria-hidden': true,
} as const;

const PlayIcon = () => (
  <svg {...iconProps}>
    <path d="M8 5v14l11-7z" />
  </svg>
);
const PauseIcon = () => (
  <svg {...iconProps}>
    <path d="M6 5h4v14H6zM14 5h4v14h-4z" />
  </svg>
);
const SoundIcon = () => (
  <svg {...iconProps}>
    <path d="M3 9v6h4l5 5V4L7 9H3zm13.5 3a4.5 4.5 0 0 0-2.5-4v8a4.5 4.5 0 0 0 2.5-4z" />
  </svg>
);
const MutedIcon = () => (
  <svg {...iconProps}>
    <path d="M3 9v6h4l5 5V4L7 9H3zm18 .41L19.59 8 17 10.59 14.41 8 13 9.41 15.59 12 13 14.59 14.41 16 17 13.41 19.59 16 21 14.59 18.41 12 21 9.41z" />
  </svg>
);
const FullscreenIcon = () => (
  <svg {...iconProps}>
    <path d="M7 14H5v5h5v-2H7v-3zm-2-4h2V7h3V5H5v5zm12 7h-3v2h5v-5h-2v3zM14 5v2h3v3h2V5h-5z" />
  </svg>
);
