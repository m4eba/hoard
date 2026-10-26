import { useQuery } from '@tanstack/react-query';
import { getFolders, getRoots, type Folder, type Place, type Root } from './api';

/**
 * Folders are addressed as a storage root plus a path inside it. Everywhere a
 * person reads or types one, it is written "Root/sub/dir" — root names cannot
 * contain a slash, which is what makes that unambiguous.
 */

export function useRoots() {
  const { data } = useQuery({ queryKey: ['roots'], queryFn: getRoots });
  return data?.roots;
}

export function useFolders() {
  const { data } = useQuery({ queryKey: ['folders'], queryFn: getFolders });
  return data?.folders;
}

export function placeLabel(place: Place, roots: Root[] | undefined): string {
  const root = roots?.find((r) => r.id === place.rootId)?.name ?? `#${place.rootId}`;
  return place.folder ? `${root}/${place.folder}` : root;
}

export const samePlace = (a: Place | null | undefined, b: Place | null | undefined) =>
  Boolean(a && b && a.rootId === b.rootId && a.folder === b.folder);

/**
 * Reads "Root/sub/dir" back into a place. A path whose first segment is not a
 * root's name is taken as relative to `fallback` — typing just "new folder"
 * creates it in the root you are already working in.
 */
export function parsePlace(text: string, roots: Root[] | undefined, fallback?: number): Place | null {
  const parts = text
    .split('/')
    .map((p) => p.trim())
    .filter(Boolean);
  if (parts.length === 0 || !roots?.length) return null;
  const root = roots.find((r) => r.name.toLowerCase() === parts[0].toLowerCase());
  if (root) return { rootId: root.id, folder: parts.slice(1).join('/') };
  const rootId = fallback ?? roots[0].id;
  return { rootId, folder: parts.join('/') };
}

export function libraryUrl(place: Place): string {
  const q = new URLSearchParams({ root: String(place.rootId) });
  if (place.folder) q.set('folder', place.folder);
  return `/library?${q}`;
}

/** The last path segment, which is what a folder is called. */
export const folderName = (path: string) => path.split('/').pop() ?? path;

/** Direct children of a folder, sorted by name. */
export function childFolders(folders: Folder[] | undefined, rootId: number, parent: string): Folder[] {
  if (!folders) return [];
  const prefix = parent ? `${parent}/` : '';
  return folders
    .filter(
      (f) =>
        f.rootId === rootId &&
        f.path !== parent &&
        f.path.startsWith(prefix) &&
        !f.path.slice(prefix.length).includes('/'),
    )
    .sort((a, b) => a.path.localeCompare(b.path, undefined, { sensitivity: 'base', numeric: true }));
}
