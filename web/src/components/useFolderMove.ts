import { useLocation, useNavigate } from 'react-router';
import { useQueryClient } from '@tanstack/react-query';
import { moveFolder, type Place } from '../api';
import { folderName, libraryUrl, placeLabel, useRoots } from '../places';

const parentOf = (folder: string) => folder.split('/').slice(0, -1).join('/');

/** Whether moving `from` into `to` would change anything at all. */
export function canMoveFolder(from: Place, to: Place): boolean {
  if (!from.folder) return false;
  if (from.rootId !== to.rootId) return true;
  return to.folder !== parentOf(from.folder) && to.folder !== from.folder && !to.folder.startsWith(`${from.folder}/`);
}

/**
 * Moves a folder after asking, and keeps the page on it: if you are looking
 * at the folder or something inside it, the view follows it to its new place.
 */
export function useFolderMove() {
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const location = useLocation();
  const roots = useRoots();

  return async (from: Place, to: Place) => {
    if (!canMoveFolder(from, to)) return;
    const name = folderName(from.folder);
    const across = from.rootId !== to.rootId ? ' This copies everything to the other storage folder first.' : '';
    if (!window.confirm(`Move “${name}” into ${placeLabel(to, roots)}?${across}`)) return;
    try {
      const res = await moveFolder(from, to);
      await queryClient.invalidateQueries({ queryKey: ['tasks'] });
      const params = new URLSearchParams(location.search);
      const viewing = params.get('folder') ?? '';
      if (
        location.pathname === '/library' &&
        Number(params.get('root')) === from.rootId &&
        (viewing === from.folder || viewing.startsWith(`${from.folder}/`))
      ) {
        navigate(libraryUrl({ rootId: res.place.rootId, folder: res.place.folder + viewing.slice(from.folder.length) }));
      }
    } catch (err) {
      alert((err as Error).message);
    }
  };
}
