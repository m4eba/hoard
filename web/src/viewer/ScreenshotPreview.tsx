import { useEffect, useRef, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { Link } from 'react-router';
import { formatBytes, upload, type Media } from '../api';

/**
 * A shot that was just taken, on its way into the library.
 *
 * Saving is automatic, into the source's folder with its tags: a screenshot of
 * a clip belongs next to the clip, and having to re-file it every time would
 * make the feature tedious exactly when it is used most. The bar is a receipt
 * with a download button, not a prompt.
 */
export function ScreenshotPreview({
  blob,
  name,
  source,
  onClose,
}: {
  blob: Blob;
  name: string;
  /** The media the shot was taken from — its folder and tags carry over. */
  source: Media;
  onClose: () => void;
}) {
  const queryClient = useQueryClient();
  const [url, setUrl] = useState('');
  const [saved, setSaved] = useState<Media | null>(null);
  const [error, setError] = useState('');
  const started = useRef(false);

  useEffect(() => {
    const objectUrl = URL.createObjectURL(blob);
    setUrl(objectUrl);
    // Revoking matters here: a few uncollected 4K PNGs is tens of megabytes
    // held for the life of the tab.
    return () => URL.revokeObjectURL(objectUrl);
  }, [blob]);

  useEffect(() => {
    // Guard against StrictMode's double-invoke, which would otherwise upload
    // the same screenshot twice.
    if (started.current) return;
    started.current = true;
    void save();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function save() {
    setError('');
    try {
      const media = await upload(new File([blob], name, { type: 'image/png' }), {
        title: `${source.title || source.filename} — screenshot`,
        tags: source.tags,
        place: { rootId: source.rootId, folder: source.folder },
      });
      setSaved(media);
      await queryClient.invalidateQueries();
    } catch (err) {
      setError((err as Error).message);
    }
  }

  function download() {
    const link = document.createElement('a');
    link.href = url;
    link.download = name;
    link.click();
  }

  return (
    <div className="absolute inset-x-0 bottom-0 z-50 flex justify-center p-4">
      <div className="flex max-w-full items-center gap-3 rounded-lg border border-edge bg-panel/95 p-3 shadow-2xl backdrop-blur">
        {url && (
          <img
            src={url}
            alt="Screenshot preview"
            className="h-16 w-28 rounded border border-edge object-contain"
          />
        )}
        <div className="min-w-0">
          <p className="truncate text-sm">{name}</p>
          <p className="text-xs text-muted">
            {formatBytes(blob.size)}
            {source.tags.length > 0 && ` · ${source.tags.join(', ')}`}
          </p>
          {error ? (
            <p className="text-xs text-red-400">{error}</p>
          ) : saved ? (
            <p className="text-xs text-accent">
              Saved next to the original ·{' '}
              <Link to={`/m/${saved.id}`} className="underline">
                open
              </Link>
            </p>
          ) : (
            <p className="text-xs text-muted">Saving…</p>
          )}
        </div>
        <div className="ml-2 flex shrink-0 gap-2">
          {error && (
            <button
              onClick={() => void save()}
              className="rounded-md border border-edge px-3 py-1.5 text-sm hover:bg-edge"
            >
              Retry
            </button>
          )}
          <button
            onClick={download}
            className="rounded-md border border-edge px-3 py-1.5 text-sm hover:bg-edge"
          >
            Download
          </button>
          <button
            onClick={onClose}
            aria-label="Dismiss screenshot"
            className="rounded-md border border-edge px-2 py-1.5 text-sm text-muted hover:bg-edge hover:text-white"
          >
            ✕
          </button>
        </div>
      </div>
    </div>
  );
}
