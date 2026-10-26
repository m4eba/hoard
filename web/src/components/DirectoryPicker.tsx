import { useEffect, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { browse } from '../api';

/**
 * Walks the server's directories, so a storage folder can be picked rather
 * than typed from memory. Paths are the server's, not the browser's machine.
 */
export function DirectoryPicker({
  initial,
  onPick,
  onClose,
}: {
  initial?: string;
  onPick: (path: string) => void;
  onClose: () => void;
}) {
  const [path, setPath] = useState<string | undefined>(initial || undefined);
  const { data, error, isLoading } = useQuery({
    queryKey: ['browse', path ?? ''],
    queryFn: () => browse(path),
    retry: false,
  });

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose();
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  const join = (dir: string) => (data!.path.endsWith('/') ? `${data!.path}${dir}` : `${data!.path}/${dir}`);

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4" onClick={onClose} role="presentation">
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Choose a directory"
        onClick={(e) => e.stopPropagation()}
        className="flex max-h-[80vh] w-full max-w-lg flex-col rounded-lg border border-edge bg-panel shadow-2xl"
      >
        <div className="border-b border-edge p-3">
          <p className="truncate font-mono text-sm" title={data?.path}>
            {data?.path ?? path ?? '…'}
          </p>
        </div>
        <div className="min-h-40 flex-1 overflow-y-auto p-1">
          {isLoading && <p className="p-3 text-sm text-muted">Loading…</p>}
          {error && <p className="p-3 text-sm text-red-400">{(error as Error).message}</p>}
          {data?.parent && (
            <button
              onClick={() => setPath(data.parent)}
              className="block w-full rounded px-3 py-1.5 text-left text-sm text-muted hover:bg-edge"
            >
              ↑ ..
            </button>
          )}
          {data?.dirs.map((d) => (
            <button
              key={d}
              onClick={() => setPath(join(d))}
              className="block w-full truncate rounded px-3 py-1.5 text-left text-sm hover:bg-edge"
            >
              {d}
            </button>
          ))}
          {data && data.dirs.length === 0 && <p className="p-3 text-sm text-muted">No subdirectories.</p>}
        </div>
        <div className="flex justify-end gap-2 border-t border-edge p-3">
          <button onClick={onClose} className="rounded-md border border-edge px-3 py-1.5 text-sm hover:bg-edge">
            Cancel
          </button>
          <button
            onClick={() => data && onPick(data.path)}
            disabled={!data}
            className="rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-ink disabled:opacity-40"
          >
            Use this directory
          </button>
        </div>
      </div>
    </div>
  );
}
