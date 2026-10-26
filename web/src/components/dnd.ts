import { useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { moveMedia, type Place } from '../api';

/**
 * Dragging tiles onto a folder moves them there.
 *
 * The payload is a custom type rather than text, so the upload dropzone can
 * tell a tile being refiled from files arriving from the desktop.
 */
export const MEDIA_MIME = 'application/x-hoard-media';

export function startMediaDrag(e: React.DragEvent, ids: number[]) {
  e.dataTransfer.setData(MEDIA_MIME, JSON.stringify(ids));
  e.dataTransfer.effectAllowed = 'move';
}

export const isMediaDrag = (e: React.DragEvent) => e.dataTransfer.types.includes(MEDIA_MIME);

export function useMoveDrop(place: Place, onMoved?: (count: number) => void) {
  const queryClient = useQueryClient();
  const [over, setOver] = useState(false);

  return {
    over,
    handlers: {
      onDragOver: (e: React.DragEvent) => {
        if (!isMediaDrag(e)) return;
        e.preventDefault();
        e.stopPropagation();
        e.dataTransfer.dropEffect = 'move';
        setOver(true);
      },
      onDragLeave: () => setOver(false),
      onDrop: (e: React.DragEvent) => {
        if (!isMediaDrag(e)) return;
        e.preventDefault();
        e.stopPropagation();
        setOver(false);
        let ids: number[] = [];
        try {
          ids = JSON.parse(e.dataTransfer.getData(MEDIA_MIME));
        } catch {
          return;
        }
        if (ids.length === 0) return;
        void moveMedia({ ids }, place)
          .then((res) => {
            onMoved?.(res.count);
            return queryClient.invalidateQueries({ queryKey: ['tasks'] });
          })
          .catch((err) => alert((err as Error).message));
      },
    },
  };
}
