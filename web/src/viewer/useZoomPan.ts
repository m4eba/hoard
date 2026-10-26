import { useCallback, useEffect, useRef } from 'react';

/**
 * Zoom and pan over an arbitrary child.
 *
 * The transform is written straight to the DOM node through a ref rather than
 * held in React state: a pan is a stream of pointermove events, and
 * re-rendering the tree on each one would make dragging a large image stutter.
 *
 * "fit" is scale 1 — the stage sizes the media with CSS so it already fills the
 * viewport — so zooming out is bounded at 1 and there is no empty space to pan
 * into until the user has zoomed in.
 */
export const MAX_SCALE = 8;

type Transform = { x: number; y: number; k: number };

export function useZoomPan(enabled = true) {
  const stageRef = useRef<HTMLDivElement | null>(null);
  const layerRef = useRef<HTMLDivElement | null>(null);
  const transform = useRef<Transform>({ x: 0, y: 0, k: 1 });
  // Live pointers, so one finger pans and two pinch without separate paths.
  const pointers = useRef(new Map<number, { x: number; y: number }>());
  const pinchStart = useRef<{ distance: number; k: number } | null>(null);

  const apply = useCallback(() => {
    const layer = layerRef.current;
    if (!layer) return;
    const { x, y, k } = transform.current;
    layer.style.transform = `translate3d(${x}px, ${y}px, 0) scale(${k})`;
    // Only offer the grab cursor when there is somewhere to drag to.
    if (stageRef.current) stageRef.current.style.cursor = k > 1 ? 'grab' : '';
  }, []);

  const reset = useCallback(() => {
    transform.current = { x: 0, y: 0, k: 1 };
    apply();
  }, [apply]);

  /**
   * Zooms about a point in stage coordinates, keeping whatever is under that
   * point exactly where it is — the behaviour that makes wheel-zoom feel like
   * it is aimed rather than merely scaling.
   */
  const zoomAt = useCallback(
    (clientX: number, clientY: number, factor: number) => {
      const stage = stageRef.current;
      if (!stage) return;
      const rect = stage.getBoundingClientRect();
      const px = clientX - rect.left;
      const py = clientY - rect.top;

      const current = transform.current;
      const k = Math.min(MAX_SCALE, Math.max(1, current.k * factor));
      if (k === current.k) return;

      // Solve for the translation that leaves (px, py) fixed.
      const ratio = k / current.k;
      transform.current = {
        k,
        x: px - (px - current.x) * ratio,
        y: py - (py - current.y) * ratio,
      };
      if (k === 1) transform.current = { x: 0, y: 0, k: 1 }; // snap back to fit
      apply();
    },
    [apply],
  );

  useEffect(() => {
    const stage = stageRef.current;
    if (!stage || !enabled) return;

    // Non-passive, because zooming has to stop the page scrolling behind it.
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      // Exponential so a trackpad's small deltas and a mouse wheel's large
      // ones both feel proportional.
      zoomAt(e.clientX, e.clientY, Math.exp(-e.deltaY / 500));
    };
    stage.addEventListener('wheel', onWheel, { passive: false });
    return () => stage.removeEventListener('wheel', onWheel);
  }, [enabled, zoomAt]);

  const onPointerDown = useCallback((e: React.PointerEvent) => {
    pointers.current.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (pointers.current.size === 2) {
      const [a, b] = [...pointers.current.values()];
      pinchStart.current = {
        distance: Math.hypot(a.x - b.x, a.y - b.y),
        k: transform.current.k,
      };
      return;
    }
    if (transform.current.k > 1) {
      // Capture so a drag that leaves the element still tracks.
      (e.target as Element).setPointerCapture?.(e.pointerId);
      if (stageRef.current) stageRef.current.style.cursor = 'grabbing';
    }
  }, []);

  const onPointerMove = useCallback(
    (e: React.PointerEvent) => {
      const previous = pointers.current.get(e.pointerId);
      if (!previous) return;
      pointers.current.set(e.pointerId, { x: e.clientX, y: e.clientY });

      if (pointers.current.size === 2 && pinchStart.current) {
        const [a, b] = [...pointers.current.values()];
        const distance = Math.hypot(a.x - b.x, a.y - b.y);
        const target = (pinchStart.current.k * distance) / pinchStart.current.distance;
        // Express the pinch as a zoom about the midpoint, so it uses the same
        // fixed-point maths as the wheel.
        zoomAt((a.x + b.x) / 2, (a.y + b.y) / 2, target / transform.current.k);
        return;
      }

      if (pointers.current.size !== 1 || transform.current.k <= 1) return;
      transform.current = {
        ...transform.current,
        x: transform.current.x + (e.clientX - previous.x),
        y: transform.current.y + (e.clientY - previous.y),
      };
      apply();
    },
    [apply, zoomAt],
  );

  const onPointerUp = useCallback(
    (e: React.PointerEvent) => {
      pointers.current.delete(e.pointerId);
      if (pointers.current.size < 2) pinchStart.current = null;
      if (stageRef.current) {
        stageRef.current.style.cursor = transform.current.k > 1 ? 'grab' : '';
      }
    },
    [],
  );

  /** Double click toggles between fit and 2x, aimed at the cursor. */
  const onDoubleClick = useCallback(
    (e: React.MouseEvent) => {
      if (transform.current.k > 1) reset();
      else zoomAt(e.clientX, e.clientY, 2);
    },
    [reset, zoomAt],
  );

  const isZoomed = () => transform.current.k > 1;

  return {
    stageRef,
    layerRef,
    reset,
    isZoomed,
    handlers: {
      onPointerDown,
      onPointerMove,
      onPointerUp,
      onPointerCancel: onPointerUp,
      onDoubleClick,
    },
  };
}
