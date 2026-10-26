import { useEffect, useState } from 'react';
import { Link, useLocation, useNavigate } from 'react-router';
import { useQueryClient } from '@tanstack/react-query';
import { createFolder, type Folder, type Root } from '../api';
import { childFolders, folderName, libraryUrl, placeLabel, useFolders, useRoots } from '../places';
import { DeleteFolderDialog } from './DeleteFolderDialog';
import { useMoveDrop } from './dnd';
import { FolderIcon } from './MediaGrid';

/**
 * The sidebar's folder tree: each storage root, and the folders inside it as
 * they are on disk. Every node is a drop target for dragged tiles.
 */
const EXPANDED_KEY = 'hoard.tree';

function readExpanded(): Set<string> {
  try {
    return new Set(JSON.parse(localStorage.getItem(EXPANDED_KEY) ?? '[]'));
  } catch {
    return new Set();
  }
}

const keyOf = (rootId: number, path: string) => `${rootId}:${path}`;

export function FolderTree() {
  const roots = useRoots();
  const folders = useFolders();
  const location = useLocation();
  const params = new URLSearchParams(location.search);
  const currentRoot = location.pathname === '/library' ? Number(params.get('root') ?? NaN) : NaN;
  const currentFolder = params.get('folder') ?? '';
  const [expanded, setExpanded] = useState(readExpanded);
  const [deleting, setDeleting] = useState<Folder | null>(null);
  const navigate = useNavigate();

  // The folder being viewed is always visible in the tree, however it was
  // reached — a link from a tile, the breadcrumb, the address bar.
  useEffect(() => {
    if (Number.isNaN(currentRoot)) return;
    setExpanded((prev) => {
      const next = new Set(prev);
      next.add(keyOf(currentRoot, ''));
      const parts = currentFolder.split('/').filter(Boolean);
      for (let i = 1; i < parts.length; i++) next.add(keyOf(currentRoot, parts.slice(0, i).join('/')));
      return next.size === prev.size ? prev : next;
    });
  }, [currentRoot, currentFolder]);

  useEffect(() => {
    try {
      localStorage.setItem(EXPANDED_KEY, JSON.stringify([...expanded]));
    } catch {
      // Forgetting the tree state is harmless.
    }
  }, [expanded]);

  const toggle = (key: string) =>
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });

  if (roots && roots.length === 0) {
    return (
      <p className="px-2 py-1 text-xs text-muted">
        No storage folders yet.{' '}
        <Link to="/settings" className="text-accent hover:underline">
          Add one
        </Link>
      </p>
    );
  }

  return (
    <>
      {roots?.map((root) => {
        const top = folders?.find((f) => f.rootId === root.id && f.path === '') ?? {
          rootId: root.id,
          path: '',
          count: 0,
          total: root.count,
        };
        return (
          <Node
            key={root.id}
            root={root}
            folder={top}
            folders={folders}
            depth={0}
            expanded={expanded}
            onToggle={toggle}
            current={{ rootId: currentRoot, folder: currentFolder }}
            onDelete={setDeleting}
          />
        );
      })}

      {deleting && (
        <DeleteFolderDialog
          folder={deleting}
          label={placeLabel({ rootId: deleting.rootId, folder: deleting.path }, roots)}
          onClose={() => setDeleting(null)}
          onDeleted={() => {
            const parent = deleting.path.split('/').slice(0, -1).join('/');
            setDeleting(null);
            navigate(libraryUrl({ rootId: deleting.rootId, folder: parent }));
          }}
        />
      )}
    </>
  );
}

function Node({
  root,
  folder,
  folders,
  depth,
  expanded,
  onToggle,
  current,
  onDelete,
}: {
  root: Root;
  folder: Folder;
  folders: Folder[] | undefined;
  depth: number;
  expanded: Set<string>;
  onToggle: (key: string) => void;
  current: { rootId: number; folder: string };
  onDelete: (f: Folder) => void;
}) {
  const queryClient = useQueryClient();
  const key = keyOf(root.id, folder.path);
  const children = childFolders(folders, root.id, folder.path);
  const open = expanded.has(key);
  const active = current.rootId === root.id && current.folder === folder.path;
  const { over, handlers } = useMoveDrop({ rootId: root.id, folder: folder.path });
  const [adding, setAdding] = useState(false);
  const [name, setName] = useState('');
  const [error, setError] = useState('');

  async function add() {
    const trimmed = name.trim();
    if (!trimmed) {
      setAdding(false);
      return;
    }
    try {
      await createFolder(root.id, folder.path ? `${folder.path}/${trimmed}` : trimmed);
      await queryClient.invalidateQueries({ queryKey: ['folders'] });
      if (!open) onToggle(key);
      setAdding(false);
      setName('');
      setError('');
    } catch (err) {
      setError((err as Error).message);
    }
  }

  return (
    <div>
      <div
        {...handlers}
        className={`group/node relative flex items-center rounded text-sm ${
          over ? 'bg-accent/25 ring-1 ring-accent' : active ? 'bg-edge text-white' : 'text-white/75 hover:bg-edge/60'
        }`}
        style={{ paddingLeft: depth * 12 }}
      >
        <button
          onClick={() => onToggle(key)}
          className={`flex size-5 shrink-0 items-center justify-center text-muted hover:text-white ${
            children.length ? '' : 'invisible'
          }`}
          aria-label={open ? 'Collapse' : 'Expand'}
        >
          <svg
            width="10"
            height="10"
            viewBox="0 0 24 24"
            fill="currentColor"
            aria-hidden
            className={`transition-transform ${open ? 'rotate-90' : ''}`}
          >
            <path d="M8 5v14l11-7z" />
          </svg>
        </button>
        <Link
          to={libraryUrl({ rootId: root.id, folder: folder.path })}
          className="flex min-w-0 flex-1 items-center gap-1.5 py-1.5 pr-12"
          title={folder.path || root.path}
        >
          {depth === 0 ? (
            <DriveIcon offline={!root.online} />
          ) : (
            <FolderIcon className="size-3.5 shrink-0 text-muted" />
          )}
          <span className={`truncate ${depth === 0 ? 'font-medium' : ''}`}>
            {depth === 0 ? root.name : folderName(folder.path)}
          </span>
          {root.scanning && depth === 0 && (
            <span className="size-1.5 shrink-0 animate-pulse rounded-full bg-accent" title="Scanning" />
          )}
        </Link>
        <span className="pointer-events-none absolute right-2 text-xs text-muted group-hover/node:opacity-0">
          {folder.total}
        </span>
        <div className="absolute right-1 flex opacity-0 transition group-hover/node:opacity-100 focus-within:opacity-100">
          <IconButton label="New folder here" onClick={() => setAdding(true)}>
            <path d="M19 13h-6v6h-2v-6H5v-2h6V5h2v6h6v2z" />
          </IconButton>
          {depth > 0 && (
            <IconButton label="Delete folder" danger onClick={() => onDelete(folder)}>
              <path d="M6 19a2 2 0 0 0 2 2h8a2 2 0 0 0 2-2V7H6v12zM19 4h-3.5l-1-1h-5l-1 1H5v2h14V4z" />
            </IconButton>
          )}
        </div>
      </div>

      {adding && (
        <div style={{ paddingLeft: depth * 12 + 20 }} className="py-1 pr-1">
          <input
            autoFocus
            value={name}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') void add();
              if (e.key === 'Escape') setAdding(false);
            }}
            onBlur={() => !name.trim() && setAdding(false)}
            placeholder="Folder name"
            className="w-full rounded border border-edge bg-ink px-2 py-1 text-xs outline-none focus:border-accent"
          />
          {error && <p className="mt-1 text-[11px] text-red-400">{error}</p>}
        </div>
      )}

      {open &&
        children.map((child) => (
          <Node
            key={child.path}
            root={root}
            folder={child}
            folders={folders}
            depth={depth + 1}
            expanded={expanded}
            onToggle={onToggle}
            current={current}
            onDelete={onDelete}
          />
        ))}
    </div>
  );
}

function IconButton({
  label,
  onClick,
  danger = false,
  children,
}: {
  label: string;
  onClick: () => void;
  danger?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      aria-label={label}
      title={label}
      className={`rounded p-1 text-muted hover:text-white ${danger ? 'hover:bg-red-900/60' : 'hover:bg-edge'}`}
    >
      <svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor" aria-hidden>
        {children}
      </svg>
    </button>
  );
}

function DriveIcon({ offline }: { offline: boolean }) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="currentColor"
      aria-hidden
      className={`size-3.5 shrink-0 ${offline ? 'text-red-400' : 'text-muted'}`}
    >
      <title>{offline ? 'Not reachable' : 'Storage folder'}</title>
      <path d="M20 13H4a2 2 0 0 0-2 2v4a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-4a2 2 0 0 0-2-2zm-3 5a1.5 1.5 0 1 1 0-3 1.5 1.5 0 0 1 0 3zM5.6 4.3 2.4 11h19.2l-3.2-6.7A2 2 0 0 0 16.6 3H7.4a2 2 0 0 0-1.8 1.3z" />
    </svg>
  );
}
