/**
 * Turning what is on screen back into pixels.
 *
 * The media sits inside a zoomed and panned layer, so "what you can see" is a
 * rectangle of the *source* that has to be worked out from the element's
 * on-screen position. Reading getBoundingClientRect rather than the transform
 * matrix means zoom, pan and the browser's own layout are all accounted for by
 * construction — there is no second copy of the transform maths to drift.
 */

export type Rect = { x: number; y: number; width: number; height: number };

export type Drawable = HTMLImageElement | HTMLVideoElement;

/** The media's own pixel dimensions, or null before it has loaded. */
export function sourceSize(el: Drawable): { w: number; h: number } | null {
  const w = el instanceof HTMLVideoElement ? el.videoWidth : el.naturalWidth;
  const h = el instanceof HTMLVideoElement ? el.videoHeight : el.naturalHeight;
  return w > 0 && h > 0 ? { w, h } : null;
}

/**
 * Where the painted pixels actually are on screen.
 *
 * object-contain letterboxes inside the element box, and that padding is not
 * part of the picture — capturing the element box would put black bars into
 * every screenshot of a squarish image.
 */
export function paintedRect(el: Drawable): Rect | null {
  const source = sourceSize(el);
  if (!source) return null;
  const box = el.getBoundingClientRect();
  const scale = Math.min(box.width / source.w, box.height / source.h);
  const width = source.w * scale;
  const height = source.h * scale;
  return {
    x: box.x + (box.width - width) / 2,
    y: box.y + (box.height - height) / 2,
    width,
    height,
  };
}

export function intersect(a: Rect, b: Rect): Rect | null {
  const x = Math.max(a.x, b.x);
  const y = Math.max(a.y, b.y);
  const right = Math.min(a.x + a.width, b.x + b.width);
  const bottom = Math.min(a.y + a.height, b.y + b.height);
  if (right <= x || bottom <= y) return null;
  return { x, y, width: right - x, height: bottom - y };
}

// A ceiling on either dimension. Zoomed right in, one screen of a 4K frame is
// a small crop, but a screenshot of a huge source at 1:1 could otherwise be
// tens of megapixels.
const MAX_DIMENSION = 8192;

/**
 * Renders the part of the media under `region` (in client coordinates) to a
 * PNG, at the source's own resolution rather than the screen's.
 *
 * Capturing at source resolution is the point: zoomed out, a screenshot of a
 * 4K video should still be 4K, not the 900 pixels the window happened to show.
 */
export async function capture(el: Drawable, region: Rect): Promise<Blob> {
  const source = sourceSize(el);
  const painted = paintedRect(el);
  if (!source || !painted) throw new Error('the media has not loaded yet');

  const visible = intersect(region, painted);
  if (!visible) throw new Error('that selection is outside the picture');

  // Screen pixels to source pixels.
  const ratio = source.w / painted.width;
  const sx = (visible.x - painted.x) * ratio;
  const sy = (visible.y - painted.y) * ratio;
  const sw = visible.width * ratio;
  const sh = visible.height * ratio;

  const shrink = Math.min(1, MAX_DIMENSION / Math.max(sw, sh));
  const width = Math.max(1, Math.round(sw * shrink));
  const height = Math.max(1, Math.round(sh * shrink));

  const canvas = document.createElement('canvas');
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext('2d');
  if (!ctx) throw new Error('this browser cannot render a screenshot');
  ctx.imageSmoothingQuality = 'high';
  ctx.drawImage(el, sx, sy, sw, sh, 0, 0, width, height);

  return new Promise((resolve, reject) => {
    // The media is served same-origin, so the canvas is not tainted and this
    // is allowed. A cross-origin source would reject here rather than silently
    // producing nothing.
    canvas.toBlob((blob) => {
      if (blob) resolve(blob);
      else reject(new Error('could not encode the screenshot'));
    }, 'image/png');
  });
}

/** A filename that says where the shot came from. */
export function screenshotName(title: string): string {
  const base =
    title
      .replace(/\.[a-z0-9]{2,5}$/i, '')
      .replace(/[^\w\d-]+/g, '-')
      .replace(/^-+|-+$/g, '')
      .slice(0, 60) || 'screenshot';
  // A timestamp, so a second shot of the same thing does not overwrite the
  // first in the downloads folder.
  const stamp = new Date().toISOString().replace(/[:.]/g, '-').slice(0, 19);
  return `${base}-${stamp}.png`;
}
