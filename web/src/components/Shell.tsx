import { Link, useLocation } from 'react-router';
import { useQuery, useQueryClient, type InfiniteData, type QueryClient } from '@tanstack/react-query';
import { formatBytes, getStats, logout, type ListItem, type ServerEvent, type Task } from '../api';
import { Activity } from './Activity';
import { FolderTree } from './FolderTree';
import { SearchBar } from './search/SearchBar';
import { useServerEvents } from './useServerEvents';

/**
 * The layout: the folder tree down the left with Import above it, one search
 * box across the top.
 */
export function Shell({ authRequired, children }: { authRequired: boolean; children: React.ReactNode }) {
  const { data: stats } = useQuery({ queryKey: ['stats'], queryFn: getStats });
  useLibraryEvents();

  return (
    <div className="flex h-full flex-col">
      <TopBar authRequired={authRequired} />
      <div className="flex min-h-0 flex-1">
        <aside className="hidden w-60 shrink-0 flex-col border-r border-edge bg-panel p-3 sm:flex">
          <Link
            to="/import"
            className="mb-4 rounded-md bg-accent px-3 py-2 text-center text-sm font-medium text-ink hover:brightness-110"
          >
            + Import
          </Link>

          <nav className="flex-1 space-y-0.5 overflow-y-auto">
            <SidebarLink to="/library">All media</SidebarLink>
            <SidebarLink to="/library?kind=video">Videos</SidebarLink>
            <SidebarLink to="/library?kind=image">Images</SidebarLink>

            <p className="px-2 pt-4 pb-1 text-[11px] tracking-wide text-muted uppercase">Folders</p>
            <FolderTree />
          </nav>

          {stats && (
            <div className="mt-3 flex justify-between border-t border-edge pt-3 text-xs text-muted">
              <span>
                {stats.count} item{stats.count === 1 ? '' : 's'}
              </span>
              <span>{formatBytes(stats.bytes)}</span>
            </div>
          )}
        </aside>

        <main className="min-w-0 flex-1 overflow-y-auto p-4">{children}</main>
      </div>
    </div>
  );
}

/**
 * A NavLink compares paths only, so every /library?… link would light up at
 * once; this compares the query too.
 */
function SidebarLink({ to, children }: { to: string; children: React.ReactNode }) {
  const location = useLocation();
  const active = location.pathname + location.search === to;
  return (
    <Link
      to={to}
      className={`flex items-center gap-2 rounded px-2 py-1.5 text-sm ${
        active ? 'bg-edge text-white' : 'text-white/75 hover:bg-edge/60'
      }`}
    >
      {children}
    </Link>
  );
}

function TopBar({ authRequired }: { authRequired: boolean }) {
  const queryClient = useQueryClient();

  return (
    <header className="flex shrink-0 items-center gap-3 border-b border-edge bg-panel px-4 py-2">
      <Link to="/library" className="font-semibold tracking-tight">
        hoard
      </Link>

      <div className="mx-auto w-full max-w-2xl">
        <SearchBar />
      </div>

      <div className="flex items-center gap-3 text-sm">
        <Activity />
        <Link to="/settings" className="text-white/80 hover:text-white">
          Settings
        </Link>
        {authRequired && (
          <button
            onClick={async () => {
              await logout();
              queryClient.clear();
              await queryClient.invalidateQueries({ queryKey: ['config'] });
            }}
            className="text-muted hover:text-white"
          >
            Sign out
          </button>
        )}
      </div>
    </header>
  );
}

/**
 * Keeps cached data in step with what the server announces.
 *
 * Refetches are coalesced: a scan of a large folder finishes thousands of
 * thumbnails, and refetching every listing for each would be a storm. A new
 * thumbnail is patched into the cached pages directly instead.
 */
function useLibraryEvents() {
  const queryClient = useQueryClient();
  useServerEvents((e: ServerEvent) => {
    switch (e.type) {
      case 'task':
        queryClient.setQueryData<{ tasks: Task[] }>(['tasks'], (prev) => {
          const tasks = (prev?.tasks ?? []).filter((t) => t.id !== e.task.id);
          return { tasks: [...tasks, e.task].sort((a, b) => a.id - b.id) };
        });
        if (e.task.state !== 'running') refreshLibrary(queryClient);
        break;
      case 'library':
        refreshLibrary(queryClient);
        break;
      case 'media':
        patchThumb(queryClient, e.id, e.thumbUrl);
        break;
      case 'storyboard':
        void queryClient.invalidateQueries({ queryKey: ['media-item', e.id] });
        break;
    }
  });
}

let pending: number | null = null;

function refreshLibrary(queryClient: QueryClient) {
  if (pending !== null) return;
  pending = window.setTimeout(() => {
    pending = null;
    for (const key of ['media', 'folders', 'roots', 'stats', 'count', 'media-item']) {
      void queryClient.invalidateQueries({ queryKey: [key] });
    }
  }, 800);
}

function patchThumb(queryClient: QueryClient, id: number, thumbUrl?: string) {
  queryClient.setQueriesData<InfiniteData<{ media: ListItem[]; nextOffset: number }>>(
    { queryKey: ['media'] },
    (data) =>
      data && {
        ...data,
        pages: data.pages.map((p) => ({
          ...p,
          media: p.media.map((m) => (m.id === id ? { ...m, thumbUrl } : m)),
        })),
      },
  );
}
