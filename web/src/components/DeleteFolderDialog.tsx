import { useEffect, useRef, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { deleteFolder, type Folder } from '../api';
import { folderName } from '../places';

/**
 * Confirmation for deleting a folder and everything in it.
 *
 * A non-empty folder has to be typed out, the way a repository host makes you
 * name what you are about to destroy. The point is not the keystrokes: it is
 * that you have to read the name, which is what catches deleting the wrong
 * one. An empty folder asks for nothing — there is nothing to lose.
 */
export function DeleteFolderDialog({
  folder,
  label,
  onClose,
  onDeleted,
}: {
  folder: Folder;
  /** "Root/sub/dir", as shown in the title. */
  label: string;
  onClose: () => void;
  onDeleted: () => void;
}) {
  const name = folderName(folder.path);
  const queryClient = useQueryClient();
  const [typed, setTyped] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const input = useRef<HTMLInputElement>(null);

  const needsConfirmation = folder.total > 0;
  const armed = !needsConfirmation || typed === name;

  useEffect(() => {
    input.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  async function confirm() {
    if (!armed || busy) return;
    setBusy(true);
    setError('');
    try {
      await deleteFolder(folder.rootId, folder.path, typed);
      await queryClient.invalidateQueries();
      onDeleted();
    } catch (err) {
      setError((err as Error).message);
      setBusy(false);
    }
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4"
      // A click on the backdrop is a way out; a click inside must not be.
      onClick={onClose}
      role="presentation"
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="delete-folder-title"
        onClick={(e) => e.stopPropagation()}
        className="w-full max-w-md rounded-lg border border-edge bg-panel p-5 shadow-2xl"
      >
        <h2 id="delete-folder-title" className="text-base font-semibold break-all">
          Delete “{label}”?
        </h2>

        {needsConfirmation ? (
          <>
            <p className="mt-2 text-sm text-muted">
              This deletes the folder and the{' '}
              <span className="text-white/90">
                {folder.total} {folder.total === 1 ? 'file' : 'files'}
              </span>{' '}
              in it, subfolders included.
            </p>
            <p className="mt-1 text-xs text-muted">
              Media files go to the storage folder's trash and stay recoverable until it is
              emptied. Files the library does not manage are left where they are.
            </p>

            <label className="mt-4 block">
              <span className="mb-1 block text-xs text-muted">
                Type <span className="font-mono text-white/90">{name}</span> to confirm
              </span>
              <input
                ref={input}
                value={typed}
                onChange={(e) => setTyped(e.target.value)}
                onKeyDown={(e) => e.key === 'Enter' && void confirm()}
                className="w-full rounded-md border border-edge bg-ink px-2 py-1.5 font-mono text-sm outline-none focus:border-accent"
                autoComplete="off"
                spellCheck={false}
              />
            </label>
          </>
        ) : (
          <p className="mt-2 text-sm text-muted">This folder is empty.</p>
        )}

        {error && <p className="mt-3 text-sm text-red-400">{error}</p>}

        <div className="mt-5 flex justify-end gap-2">
          <button
            onClick={onClose}
            className="rounded-md border border-edge px-3 py-1.5 text-sm hover:bg-edge"
          >
            Cancel
          </button>
          <button
            onClick={() => void confirm()}
            disabled={!armed || busy}
            className="rounded-md bg-red-700 px-3 py-1.5 text-sm font-medium text-white hover:bg-red-600 disabled:cursor-not-allowed disabled:opacity-40"
          >
            {busy ? 'Deleting…' : needsConfirmation ? 'Delete folder and files' : 'Delete folder'}
          </button>
        </div>
      </div>
    </div>
  );
}
