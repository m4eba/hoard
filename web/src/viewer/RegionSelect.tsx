import { useEffect, useRef, useState } from 'react';
import type { Rect } from './capture';

/**
 * Drag out a region, then adjust it with handles.
 *
 * Sits above the stage and takes the pointer, so a drag here selects rather
 * than panning the media underneath. Coordinates are client coordinates
 * throughout, which is what the capture maths expects — converting to and from
 * some local space would be a second place for an off-by-one to live.
 */
export function RegionSelect({
  bounds,
  onCapture,
  onCancel,
}: {
  /** The picture's on-screen rectangle: selection is clamped to it. */
  bounds: Rect;
  onCapture: (region: Rect) => void;
  onCancel: () => void;
}) {
  const [rect, setRect] = useState<Rect | null>(null);
  const drag = useRef<{ mode: string; startX: number; startY: number; origin: Rect } | null>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onCancel();
      if (e.key === 'Enter' && rect && rect.width > 4 && rect.height > 4) onCapture(rect);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [rect, onCapture, onCancel]);

  const clamp = (r: Rect): Rect => {
    const width = Math.min(r.width, bounds.width);
    const height = Math.min(r.height, bounds.height);
    return {
      width,
      height,
      x: Math.min(Math.max(r.x, bounds.x), bounds.x + bounds.width - width),
      y: Math.min(Math.max(r.y, bounds.y), bounds.y + bounds.height - height),
    };
  };

  function onPointerDown(e: React.PointerEvent, mode: string) {
    e.preventDefault();
    e.stopPropagation();
    try {
      // Keeps the drag alive when the pointer leaves the element. Not worth
      // aborting the drag over: a pointer id the browser no longer tracks
      // throws, and the drag still works without capture.
      (e.target as Element).setPointerCapture?.(e.pointerId);
    } catch {
      /* ignore */
    }
    const origin = rect ?? { x: e.clientX, y: e.clientY, width: 0, height: 0 };
    drag.current = { mode, startX: e.clientX, startY: e.clientY, origin };
    if (mode === 'new') setRect({ x: e.clientX, y: e.clientY, width: 0, height: 0 });
  }

  function onPointerMove(e: React.PointerEvent) {
    const d = drag.current;
    if (!d) return;
    const dx = e.clientX - d.startX;
    const dy = e.clientY - d.startY;

    if (d.mode === 'new') {
      // Normalised, so dragging up or left works as well as down and right.
      const x = Math.min(d.startX, e.clientX);
      const y = Math.min(d.startY, e.clientY);
      setRect(clamp({ x, y, width: Math.abs(dx), height: Math.abs(dy) }));
      return;
    }
    if (d.mode === 'move') {
      setRect(clamp({ ...d.origin, x: d.origin.x + dx, y: d.origin.y + dy }));
      return;
    }

    // A resize handle: each letter of the mode is an edge it moves.
    let { x, y, width, height } = d.origin;
    if (d.mode.includes('w')) {
      x = d.origin.x + dx;
      width = d.origin.width - dx;
    }
    if (d.mode.includes('e')) width = d.origin.width + dx;
    if (d.mode.includes('n')) {
      y = d.origin.y + dy;
      height = d.origin.height - dy;
    }
    if (d.mode.includes('s')) height = d.origin.height + dy;

    // Dragging an edge past the opposite one flips the rectangle rather than
    // producing a negative size.
    if (width < 0) {
      x += width;
      width = -width;
    }
    if (height < 0) {
      y += height;
      height = -height;
    }
    setRect(clamp({ x, y, width, height }));
  }

  const endDrag = () => {
    drag.current = null;
  };

  const usable = rect && rect.width > 4 && rect.height > 4;

  return (
    <div
      className="absolute inset-0 z-40 cursor-crosshair"
      onPointerDown={(e) => onPointerDown(e, 'new')}
      onPointerMove={onPointerMove}
      onPointerUp={endDrag}
      onPointerCancel={endDrag}
      style={{ touchAction: 'none' }}
    >
      {/* Four panels rather than one box with a huge shadow: the dimmed area
          stays crisp at any size and needs no compositing tricks. */}
      {rect && (
        <>
          <Dim style={{ left: 0, top: 0, right: 0, height: Math.max(0, rect.y) }} />
          <Dim style={{ left: 0, top: rect.y + rect.height, right: 0, bottom: 0 }} />
          <Dim style={{ left: 0, top: rect.y, width: Math.max(0, rect.x), height: rect.height }} />
          <Dim style={{ left: rect.x + rect.width, top: rect.y, right: 0, height: rect.height }} />
        </>
      )}
      {!rect && <div className="absolute inset-0 bg-black/40" />}

      {rect && (
        <div
          className="absolute border border-white/90 shadow-[0_0_0_1px_rgba(0,0,0,0.6)]"
          style={{ left: rect.x, top: rect.y, width: rect.width, height: rect.height }}
          onPointerDown={(e) => onPointerDown(e, 'move')}
        >
          <span className="absolute -top-6 left-0 rounded bg-black/70 px-1.5 py-0.5 text-[11px] tabular-nums text-white">
            {Math.round(rect.width)} × {Math.round(rect.height)}
          </span>
          {HANDLES.map(({ mode, className }) => (
            <span
              key={mode}
              onPointerDown={(e) => onPointerDown(e, mode)}
              className={`absolute h-2.5 w-2.5 rounded-full border border-black/60 bg-white ${className}`}
            />
          ))}
        </div>
      )}

      <div className="absolute inset-x-0 bottom-6 flex justify-center gap-2">
        <div className="pointer-events-auto flex items-center gap-2 rounded-full bg-black/80 px-3 py-2 text-sm backdrop-blur">
          <span className="px-1 text-white/70">
            {usable ? 'Drag the handles to adjust' : 'Drag to select a region'}
          </span>
          <button
            onPointerDown={(e) => e.stopPropagation()}
            onClick={() => usable && onCapture(rect!)}
            disabled={!usable}
            className="rounded-md bg-accent px-3 py-1 font-medium text-ink disabled:opacity-40"
          >
            Capture
          </button>
          <button
            onPointerDown={(e) => e.stopPropagation()}
            onClick={onCancel}
            className="rounded-md border border-white/25 px-3 py-1 text-white/90"
          >
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
}

function Dim({ style }: { style: React.CSSProperties }) {
  return <div className="absolute bg-black/60" style={style} />;
}

const HANDLES = [
  { mode: 'nw', className: '-top-1.5 -left-1.5 cursor-nwse-resize' },
  { mode: 'n', className: '-top-1.5 left-1/2 -translate-x-1/2 cursor-ns-resize' },
  { mode: 'ne', className: '-top-1.5 -right-1.5 cursor-nesw-resize' },
  { mode: 'e', className: 'top-1/2 -right-1.5 -translate-y-1/2 cursor-ew-resize' },
  { mode: 'se', className: '-bottom-1.5 -right-1.5 cursor-nwse-resize' },
  { mode: 's', className: '-bottom-1.5 left-1/2 -translate-x-1/2 cursor-ns-resize' },
  { mode: 'sw', className: '-bottom-1.5 -left-1.5 cursor-nesw-resize' },
  { mode: 'w', className: 'top-1/2 -left-1.5 -translate-y-1/2 cursor-ew-resize' },
];
