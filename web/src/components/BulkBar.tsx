import { useEffect, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { bulkDelete, bulkRestore, bulkUpdate, moveMedia, type BulkEdit, type Place, type TagState } from '../api';
import { FolderInput } from './FolderInput';
import { TagInput } from './TagInput';
import type { SelectionState } from './useSelection';

/**
 * The bar that appears once anything is selected.
 *
 * Add and remove are separate fields rather than one "tags" box with a mode
 * switch: the destructive verb should never be the one you reach for by
 * accident, and in bulk, replacing tags is how forty files lose the tags that
 * made them findable.
 *
 * Edits are undoable from the toast rather than confirmed up front — a dialog
 * on every bulk edit only teaches people to dismiss dialogs.
 */
export function BulkBar({
  selection,
  loaded,
  total,
  fallbackRoot,
}: {
  selection: SelectionState;
  /** How many tiles are on screen, which is what "select all" first offers. */
  loaded: number;
  /** How many the current filter matches in total, once it has been counted. */
  total?: number;
  /** The root a bare folder name typed into "move to" is created in. */
  fallbackRoot?: number;
}) {
  const queryClient = useQueryClient();
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState('');
  const [undo, setUndo] = useState<TagState[] | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [addTags, setAddTags] = useState<string[]>([]);
  const [removeTags, setRemoveTags] = useState<string[]>([]);

  // Escape drops the selection — the way out of a mode should not require
  // finding the button that ends it.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      if (target && /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName)) return;
      if (e.key === 'Escape' && !confirming) selection.clear();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [confirming, selection]);

  if (!selection.active) return null;

  const shown = selection.allMatching ? (total ?? loaded) : selection.count;

  async function apply(edit: BulkEdit) {
    setBusy(true);
    setStatus('');
    try {
      const res = await bulkUpdate(selection.selection, edit);
      setUndo(res.previous.length > 0 ? res.previous : null);
      setStatus(`${res.updated} file${res.updated === 1 ? '' : 's'} updated`);
      setAddTags([]);
      setRemoveTags([]);
      await queryClient.invalidateQueries();
    } catch (err) {
      setStatus((err as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function move(place: Place) {
    setBusy(true);
    setStatus('');
    try {
      const res = await moveMedia(selection.selection, place);
      setUndo(null);
      setStatus(`Moving ${res.count} file${res.count === 1 ? '' : 's'}…`);
      selection.clear();
      await queryClient.invalidateQueries({ queryKey: ['tasks'] });
    } catch (err) {
      setStatus((err as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function revert() {
    if (!undo) return;
    setBusy(true);
    try {
      const res = await bulkRestore(undo);
      setUndo(null);
      setStatus(`${res.updated} file${res.updated === 1 ? '' : 's'} put back`);
      await queryClient.invalidateQueries();
    } catch (err) {
      setStatus((err as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function destroy(confirm: number) {
    setBusy(true);
    try {
      const res = await bulkDelete(selection.selection, confirm);
      setStatus(`${res.deleted} file${res.deleted === 1 ? '' : 's'} deleted`);
      setConfirming(false);
      selection.clear();
      await queryClient.invalidateQueries();
    } catch (err) {
      setStatus((err as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="sticky bottom-0 z-30 -mx-4 mt-4 border-t border-edge bg-panel/95 px-4 py-3 backdrop-blur">
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-sm font-medium whitespace-nowrap">
          {shown} selected
          {selection.allMatching && <span className="ml-1 text-muted">(everything matching)</span>}
        </span>

        {!selection.allMatching && total !== undefined && total > selection.count && (
          <button
            onClick={selection.selectAllMatching}
            className="rounded-md border border-edge px-2 py-1 text-xs text-accent hover:bg-edge"
          >
            Select all {total}
          </button>
        )}
        {!selection.allMatching && selection.count < loaded && (
          <button
            onClick={selection.selectLoaded}
            className="rounded-md border border-edge px-2 py-1 text-xs hover:bg-edge"
          >
            Select these {loaded}
          </button>
        )}

        <div className="ml-auto flex flex-wrap items-center gap-2">
          <div className="w-56">
            <TagInput tags={addTags} onChange={setAddTags} placeholder="Add tags…" compact />
          </div>
          <button
            onClick={() => void apply({ addTags })}
            disabled={busy || addTags.length === 0}
            className="rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-ink disabled:opacity-40"
          >
            Add
          </button>

          <div className="w-48">
            <TagInput
              tags={removeTags}
              onChange={setRemoveTags}
              placeholder="Remove tags…"
              compact
            />
          </div>
          <button
            onClick={() => void apply({ removeTags })}
            disabled={busy || removeTags.length === 0}
            className="rounded-md border border-edge px-3 py-1.5 text-sm hover:bg-edge disabled:opacity-40"
          >
            Remove
          </button>

          <div className="w-52">
            <FolderInput
              value={null}
              placeholder="Move to folder…"
              fallbackRoot={fallbackRoot}
              onCommit={(place) => place && void move(place)}
            />
          </div>

          <button
            onClick={() => setConfirming(true)}
            disabled={busy}
            className="rounded-md border border-red-900 px-3 py-1.5 text-sm text-red-400 hover:bg-red-950 disabled:opacity-40"
          >
            Delete
          </button>
          <button
            onClick={selection.clear}
            aria-label="Clear selection"
            className="rounded-md border border-edge px-2 py-1.5 text-sm text-muted hover:bg-edge hover:text-white"
          >
            ✕
          </button>
        </div>
      </div>

      {(status || undo) && (
        <div className="mt-2 flex items-center gap-3 text-xs">
          {status && <span className="text-muted">{status}</span>}
          {undo && (
            <button
              onClick={() => void revert()}
              disabled={busy}
              className="rounded border border-edge px-2 py-0.5 text-accent hover:bg-edge"
            >
              Undo
            </button>
          )}
        </div>
      )}

      {confirming && (
        <DeleteConfirm
          count={shown}
          busy={busy}
          onCancel={() => setConfirming(false)}
          onConfirm={destroy}
        />
      )}
    </div>
  );
}

/**
 * Typing the count is the same gate the album delete uses, for the same
 * reason: the number has to be read before it can be typed, and reading it is
 * what catches "I meant the other twelve".
 */
function DeleteConfirm({
  count,
  busy,
  onCancel,
  onConfirm,
}: {
  count: number;
  busy: boolean;
  onCancel: () => void;
  onConfirm: (count: number) => void;
}) {
  const [typed, setTyped] = useState('');
  const armed = typed.trim() === String(count);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4"
      onClick={onCancel}
      role="presentation"
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="bulk-delete-title"
        onClick={(e) => e.stopPropagation()}
        className="w-full max-w-md rounded-lg border border-edge bg-panel p-5 shadow-2xl"
      >
        <h2 id="bulk-delete-title" className="text-base font-semibold">
          Delete {count} file{count === 1 ? '' : 's'}?
        </h2>
        <p className="mt-2 text-sm text-muted">
          The files go to their storage folder's trash and are removed for good when the trash is
          emptied.
        </p>
        <label className="mt-4 block">
          <span className="mb-1 block text-xs text-muted">
            Type <span className="font-mono text-white/90">{count}</span> to confirm
          </span>
          <input
            autoFocus
            value={typed}
            onChange={(e) => setTyped(e.target.value)}
            onKeyDown={(e) => e.key === 'Enter' && armed && onConfirm(count)}
            className="w-full rounded-md border border-edge bg-ink px-2 py-1.5 font-mono text-sm outline-none focus:border-accent"
            autoComplete="off"
          />
        </label>
        <div className="mt-5 flex justify-end gap-2">
          <button
            onClick={onCancel}
            className="rounded-md border border-edge px-3 py-1.5 text-sm hover:bg-edge"
          >
            Cancel
          </button>
          <button
            onClick={() => onConfirm(count)}
            disabled={!armed || busy}
            className="rounded-md bg-red-700 px-3 py-1.5 text-sm font-medium text-white hover:bg-red-600 disabled:cursor-not-allowed disabled:opacity-40"
          >
            {busy ? 'Deleting…' : 'Delete'}
          </button>
        </div>
      </div>
    </div>
  );
}
