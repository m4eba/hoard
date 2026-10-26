import { useCallback, useMemo, useRef, useState } from 'react';
import type { Filter, ListItem, Selection } from '../api';

/**
 * Which tiles are picked, and how a click changes that.
 *
 * Two kinds of selection, because a grid pages: an explicit set of ids, or
 * "everything matching what I am looking at" — the second is a filter the
 * server resolves, since the browser has only the tiles it has scrolled to.
 * `allMatching` is the flag that switches between them.
 */
export function useSelection(items: ListItem[], filter: Filter) {
  const [ids, setIds] = useState<Set<number>>(new Set());
  const [allMatching, setAllMatching] = useState(false);
  // The anchor a shift-click extends from.
  const anchor = useRef<number | null>(null);

  const clear = useCallback(() => {
    setIds(new Set());
    setAllMatching(false);
    anchor.current = null;
  }, []);

  const toggle = useCallback(
    (id: number, extend = false) => {
      setAllMatching(false);
      setIds((current) => {
        const next = new Set(current);
        if (extend && anchor.current !== null) {
          // A range picks up everything between the two, and always adds:
          // shift-clicking to widen a selection must not punch holes in it.
          const from = items.findIndex((i) => i.id === anchor.current);
          const to = items.findIndex((i) => i.id === id);
          if (from >= 0 && to >= 0) {
            for (let i = Math.min(from, to); i <= Math.max(from, to); i++) {
              next.add(items[i].id);
            }
            return next;
          }
        }
        if (next.has(id)) next.delete(id);
        else next.add(id);
        anchor.current = id;
        return next;
      });
    },
    [items],
  );

  /** Picks the tiles that are loaded; the bar then offers the rest. */
  const selectLoaded = useCallback(() => {
    setIds(new Set(items.map((i) => i.id)));
    setAllMatching(false);
  }, [items]);

  const selectAllMatching = useCallback(() => setAllMatching(true), []);

  const selection: Selection = useMemo(
    () => (allMatching ? { query: filter } : { ids: [...ids] }),
    [allMatching, filter, ids],
  );

  return {
    ids,
    allMatching,
    // While "all matching" is on, every loaded tile reads as selected — the
    // ones off-screen are in the selection too, and showing the visible half
    // unticked would misrepresent what a delete is about to take.
    isSelected: (id: number) => allMatching || ids.has(id),
    count: allMatching ? -1 : ids.size,
    active: allMatching || ids.size > 0,
    selection,
    toggle,
    clear,
    selectLoaded,
    selectAllMatching,
  };
}

export type SelectionState = ReturnType<typeof useSelection>;
