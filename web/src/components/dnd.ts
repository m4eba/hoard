import { useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { moveMedia, type Place } from '../api';
import { useFolderMove } from './useFolderMove';

/**
 * Dragging tiles or folders onto a folder moves them there.
 *
 * The payloads are custom types rather than text, so the upload dropzone can
 * tell a tile being refiled from files arriving from the desktop.
 */
export const MEDIA_MIME = 'application/x-hoard-media';
export const FOLDER_MIME = 'application/x-hoard-folder';

export function startMediaDrag(e: React.DragEvent, ids: number[]) {
  e.dataTransfer.setData(MEDIA_MIME, JSON.stringify(ids));
  e.dataTransfer.effectAllowed = 'move';
}

export function startFolderDrag(e: React.DragEvent, folder: Place) {
  // The tree nests links inside drop targets; only the innermost one drags.
  e.stopPropagation();
  e.dataTransfer.setData(FOLDER_MIME, JSON.stringify(folder));
  e.dataTransfer.effectAllowed = 'move';
}

export const isMediaDrag = (e: React.DragEvent) => e.dataTransfer.types.includes(MEDIA_MIME);
const isFolderDrag = (e: React.DragEvent) => e.dataTransfer.types.includes(FOLDER_MIME);

export function useMoveDrop(place: Place, onMoved?: (count: number) => void) {
  const queryClient = useQueryClient();
  const moveFolderTo = useFolderMove();
  const [over, setOver] = useState(false);

  return {
    over,
    handlers: {
      onDragOver: (e: React.DragEvent) => {
        // The dragged folder itself cannot be read until the drop, so every
        // folder accepts it and the drop decides whether it means anything.
        if (!isMediaDrag(e) && !isFolderDrag(e)) return;
        e.preventDefault();
        e.stopPropagation();
        e.dataTransfer.dropEffect = 'move';
        setOver(true);
      },
      onDragLeave: () => setOver(false),
      onDrop: (e: React.DragEvent) => {
        if (isFolderDrag(e)) {
          e.preventDefault();
          e.stopPropagation();
          setOver(false);
          try {
            void moveFolderTo(JSON.parse(e.dataTransfer.getData(FOLDER_MIME)) as Place, place);
          } catch {
            // Not one of ours.
          }
          return;
        }
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
