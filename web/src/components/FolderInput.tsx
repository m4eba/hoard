import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react';
import type { Place } from '../api';
import { parsePlace, placeLabel, samePlace, useFolders, useRoots } from '../places';

/**
 * Folder picker: choose an existing folder or type a new path.
 *
 * Paths read "Root/sub/dir". A path that starts with something other than a
 * root's name lands in `fallbackRoot`, so typing a bare name creates a folder
 * next to where you already are.
 *
 * The value is committed on blur, on Enter, or on picking a suggestion —
 * never per keystroke: a commit can create a folder on disk, and a
 * per-keystroke one would leave H, Ho, Hol, Holi… behind.
 */
export function FolderInput({
  value,
  onCommit,
  placeholder = 'Folder',
  fallbackRoot,
  allowEmpty = false,
  autoFocus = false,
  onCancel,
  narrow = false,
  className = '',
}: {
  value: Place | null;
  onCommit: (place: Place | null) => void;
  placeholder?: string;
  fallbackRoot?: number;
  /** Clearing the box commits null, which callers read as "the default". */
  allowEmpty?: boolean;
  autoFocus?: boolean;
  /** Escape, or leaving the box without a folder. */
  onCancel?: () => void;
  /** In a tight container that clips, like the sidebar: the list keeps to
   *  the box's width instead of growing past it. */
  narrow?: boolean;
  className?: string;
}) {
  const roots = useRoots();
  const folders = useFolders();
  const label = value ? placeLabel(value, roots) : '';
  const [text, setText] = useState(label);
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(-1);
  const boxRef = useRef<HTMLDivElement>(null);
  // Whether an edit is in progress. A ref, because Enter commits and then
  // blurs, and the blur's stale closure must not commit a second time.
  const editing = useRef(false);

  // Follow the value when it changes elsewhere, but never mid-edit — the
  // dropdown being open is the signal for that.
  useEffect(() => {
    if (!open) setText(label);
  }, [label, open]);

  const all = useMemo(
    () =>
      (folders ?? [])
        .map((f) => ({ place: { rootId: f.rootId, folder: f.path }, total: f.total }))
        .map((o) => ({ ...o, label: placeLabel(o.place, roots) }))
        .sort((a, b) => a.label.localeCompare(b.label, undefined, { sensitivity: 'base', numeric: true })),
    [folders, roots],
  );
  const query = text.trim().toLowerCase();
  const matches = (query ? all.filter((o) => o.label.toLowerCase().includes(query)) : all).slice(0, 50);
  const typed = parsePlace(text, roots, fallbackRoot ?? value?.rootId);
  const exists = typed && all.some((o) => samePlace(o.place, typed));

  function commit(next: Place | null) {
    editing.current = false;
    setOpen(false);
    if (!next) {
      setText(label);
      if (allowEmpty && value) onCommit(null);
      else onCancel?.();
      return;
    }
    setText(placeLabel(next, roots));
    if (!samePlace(next, value)) onCommit(next);
  }

  function onKeyDown(e: KeyboardEvent<HTMLInputElement>) {
    if (open && e.key === 'ArrowDown') {
      e.preventDefault();
      setActive((i) => Math.min(i + 1, matches.length - 1));
      return;
    }
    if (open && e.key === 'ArrowUp') {
      e.preventDefault();
      setActive((i) => Math.max(i - 1, -1));
      return;
    }
    if (e.key === 'Enter') {
      e.preventDefault();
      commit(active >= 0 && matches[active] ? matches[active].place : typed);
      (e.target as HTMLInputElement).blur();
      return;
    }
    if (e.key === 'Escape') {
      editing.current = false;
      setText(label);
      setOpen(false);
      onCancel?.();
    }
  }

  return (
    <div ref={boxRef} className={`relative ${className}`}>
      <input
        autoFocus={autoFocus}
        value={text}
        onChange={(e) => {
          setText(e.target.value);
          editing.current = true;
          setOpen(true);
          setActive(-1);
        }}
        onFocus={() => {
          editing.current = true;
          setOpen(true);
        }}
        // A short delay, or the blur fires before a click on a suggestion
        // registers and the choice is lost.
        onBlur={() =>
          setTimeout(() => {
            if (!editing.current) return;
            commit(text.trim() ? typed : null);
          }, 120)
        }
        onKeyDown={onKeyDown}
        placeholder={placeholder}
        className="w-full rounded-md border border-edge bg-ink px-2 py-1.5 text-sm outline-none focus:border-accent"
      />

      {open && (matches.length > 0 || (typed && !exists)) && (
        <ul className={`absolute z-40 mt-1 max-h-60 w-full ${narrow ? '' : 'min-w-56'} overflow-auto rounded-md border border-edge bg-panel py-1 shadow-xl`}>
          {typed && !exists && query && (
            <li>
              <button
                type="button"
                onMouseDown={(e) => {
                  e.preventDefault();
                  commit(typed);
                }}
                className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm text-accent"
                title={placeLabel(typed, roots)}
              >
                <span className="truncate">+ Create “{placeLabel(typed, roots)}”</span>
              </button>
            </li>
          )}
          {matches.map((o, i) => (
            <li key={o.label}>
              <button
                type="button"
                onMouseDown={(e) => {
                  // mousedown, not click: blur would close the list first.
                  e.preventDefault();
                  commit(o.place);
                }}
                onMouseEnter={() => setActive(i)}
                className={`flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm ${
                  i === active ? 'bg-edge text-white' : 'text-white/80'
                }`}
              >
                <span className="truncate" title={o.label}>
                  {o.label}
                </span>
                <span className="ml-auto shrink-0 text-xs text-muted">{o.total}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
