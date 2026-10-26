// The API client. Everything is same-origin and cookie-authenticated, so
// there is no token to carry and no base URL to configure.

export class ApiError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message);
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, {
    credentials: 'same-origin',
    ...init,
    headers: { 'Content-Type': 'application/json', ...init?.headers },
  });
  const text = await res.text();
  let body: { error?: string } = {};
  try {
    body = text ? JSON.parse(text) : {};
  } catch {
    // A non-JSON body means something upstream answered instead of the API.
  }
  if (res.status === 401 && !path.startsWith('/api/auth')) {
    // The session expired or was never there: the app shell shows the login.
    window.dispatchEvent(new Event('hoard:unauthorized'));
  }
  if (!res.ok) {
    // The server answers { error } uniformly, so the message is worth showing
    // rather than replacing with a generic apology.
    throw new ApiError(res.status, body.error ?? `request failed (${res.status})`);
  }
  return body as T;
}

export const api = {
  get: <T>(path: string) => request<T>(path),
  post: <T>(path: string, body?: unknown) =>
    request<T>(path, { method: 'POST', body: body === undefined ? undefined : JSON.stringify(body) }),
  put: <T>(path: string, body: unknown) => request<T>(path, { method: 'PUT', body: JSON.stringify(body) }),
  patch: <T>(path: string, body: unknown) =>
    request<T>(path, { method: 'PATCH', body: JSON.stringify(body) }),
  del: <T>(path: string) => request<T>(path, { method: 'DELETE' }),
};

// ── types ──────────────────────────────────────────────────────────────

export type Config = { authRequired: boolean; authenticated: boolean; version: string };

/** A folder within a storage root; '' is the root's top level. */
export type Place = { rootId: number; folder: string };

export type Root = {
  id: number;
  name: string;
  path: string;
  online: boolean;
  scanning: boolean;
  count: number;
  bytes: number;
  lastScanAt?: string;
};

export type Folder = {
  rootId: number;
  path: string;
  /** Media directly inside. */
  count: number;
  /** Media inside, including subfolders. */
  total: number;
  thumbUrl?: string;
};

export type ListItem = {
  id: number;
  kind: 'image' | 'video';
  title: string;
  filename: string;
  rootId: number;
  folder: string;
  width: number;
  height: number;
  duration: number;
  size: number;
  thumbUrl?: string;
  tags: string[];
  createdAt: string;
};

export type Storyboard =
  | { state: 'none' | 'pending' }
  | {
      state: 'ready';
      url: string;
      cols: number;
      rows: number;
      tileWidth: number;
      tileHeight: number;
      interval: number;
      count: number;
    };

export type Media = ListItem & {
  description: string;
  sourceUrl: string;
  url: string;
  mime: string;
  videoCodec: string;
  audioCodec: string;
  rootName: string;
  fileModifiedAt: string;
  processed: number;
  storyboard: Storyboard;
};

export type ImportJob = {
  id: number;
  batchId: string;
  url: string;
  state: 'queued' | 'extracting' | 'downloading' | 'processing' | 'done' | 'failed' | 'cancelled';
  title: string;
  bytesTotal: number;
  bytesDone: number;
  percent: number;
  tags: string[];
  rootId?: number;
  folder: string;
  mediaId?: number;
  thumbUrl?: string;
  error?: string;
  createdAt: string;
};

export type BatchSummary = {
  batchId: string;
  total: number;
  done: number;
  failed: number;
  running: number;
  firstTitle: string;
  createdAt: string;
};

export type Task = {
  id: number;
  kind: string;
  label: string;
  done: number;
  total: number;
  state: 'running' | 'done' | 'failed';
  detail: string;
};

export type YtdlpStatus = {
  available: boolean;
  managed: boolean;
  installed: boolean;
  version?: string;
  updating: boolean;
  lastUpdate?: string;
  error?: string;
  python?: string;
  envPath: string;
};

export type Settings = {
  downloadPlace: Place | null;
  ytdlpArgs: string;
  defaultYtdlpArgs: string;
  trashDays: number;
};

/** What the server pushes over /api/events. */
export type ServerEvent =
  | { type: 'media'; id: number; thumbUrl?: string }
  | { type: 'storyboard'; id: number; ready: boolean }
  | { type: 'task'; task: Task }
  | { type: 'job'; job: ImportJob }
  | { type: 'library' };

export const jobIsTerminal = (state: ImportJob['state']) =>
  state === 'done' || state === 'failed' || state === 'cancelled';

// ── endpoints ──────────────────────────────────────────────────────────

export const getConfig = () => api.get<Config>('/api/config');
export const login = (password: string) => api.post<{ ok: boolean }>('/api/auth/login', { password });
export const logout = () => api.post<{ ok: boolean }>('/api/auth/logout');

export const getRoots = () => api.get<{ roots: Root[] }>('/api/roots');
export const addRoot = (path: string, name?: string) =>
  api.post<{ id: number }>('/api/roots', { path, name: name || undefined });
export const renameRoot = (id: number, name: string) => api.patch<{ ok: boolean }>(`/api/roots/${id}`, { name });
export const removeRoot = (id: number) => api.del<{ ok: boolean }>(`/api/roots/${id}`);
export const scanRoot = (id: number) => api.post<{ ok: boolean }>(`/api/roots/${id}/scan`);
export const browse = (path?: string) =>
  api.get<{ path: string; parent?: string; dirs: string[] }>(
    `/api/fs${path ? `?path=${encodeURIComponent(path)}` : ''}`,
  );

export const getFolders = () => api.get<{ folders: Folder[] }>('/api/folders');
export const createFolder = (rootId: number, path: string) =>
  api.post<{ rootId: number; path: string }>('/api/folders', { rootId, path });

/**
 * Deletes a folder and the media in it. A non-empty folder needs its name
 * echoed back — the server checks it too.
 */
export const deleteFolder = (rootId: number, path: string, confirm: string) =>
  api.del<{ ok: boolean; deletedMedia: number }>(
    `/api/folders?rootId=${rootId}&path=${encodeURIComponent(path)}&confirm=${encodeURIComponent(confirm)}`,
  );

export const getStats = () =>
  api.get<{ count: number; bytes: number; videos: number; images: number }>('/api/stats');

/** What a listing is filtered by; also what "select all matching" sends. */
export type Filter = {
  q?: string;
  rootId?: number;
  folder?: string;
  recursive?: boolean;
  kind?: string;
};

export const listMedia = (filter: Filter, offset = 0, sort?: string, limit?: number, signal?: AbortSignal) => {
  const q = new URLSearchParams();
  if (filter.q) q.set('q', filter.q);
  if (filter.rootId !== undefined) q.set('rootId', String(filter.rootId));
  if (filter.folder) q.set('folder', filter.folder);
  if (filter.recursive) q.set('recursive', 'true');
  if (filter.kind) q.set('kind', filter.kind);
  if (sort) q.set('sort', sort);
  q.set('offset', String(offset));
  if (limit) q.set('limit', String(limit));
  return request<{ media: ListItem[]; nextOffset: number }>(`/api/media?${q}`, { signal });
};

export const getMedia = (id: number) => api.get<Media>(`/api/media/${id}`);

export const updateMedia = (id: number, body: { title?: string; description?: string; tags?: string[] }) =>
  api.patch<Media>(`/api/media/${id}`, body);

export const deleteMedia = (id: number) => api.del<{ ok: boolean }>(`/api/media/${id}`);

export const requestStoryboard = (id: number) => api.post<Storyboard>(`/api/media/${id}/storyboard`);

/**
 * Queues storyboards for every video in a selection — a folder is a filter
 * selection. Videos that already have one are skipped unless `rebuild`.
 */
export const queueStoryboards = (select: Selection, rebuild = false) =>
  api.post<{ queued: number; skipped: number }>('/api/storyboards', { select, rebuild });

/** Empties the storyboard queue; the video being built finishes. */
export const clearStoryboards = () => api.del<{ cleared: number }>('/api/storyboards');

export const suggestTags = (q: string, signal?: AbortSignal) =>
  fetch(`/api/tags/suggest?q=${encodeURIComponent(q)}`, { signal, credentials: 'same-origin' })
    .then((r) => (r.ok ? r.json() : { tags: [] }))
    .then((j: { tags: string[] }) => j.tags);

export const getTags = () => api.get<{ tags: { tag: string; count: number }[] }>('/api/tags');

/**
 * What a bulk operation acts on: a hand-picked set of ids, or the filter the
 * user is looking at — "everything matching this" cannot be sent as a list,
 * because the grid pages and most of a large result set is not in the browser.
 */
export type Selection = { ids: number[]; query?: never } | { query: Filter; ids?: never };

/** One item's tags as they were before a bulk edit, which is what undo replays. */
export type TagState = { id: number; tags: string[] };

export type BulkEdit = { addTags?: string[]; removeTags?: string[]; setTags?: string[] };

export const countSelection = (select: Selection) => api.post<{ count: number }>('/api/media/count', { select });

export const bulkUpdate = (select: Selection, edit: BulkEdit) =>
  api.patch<{ updated: number; previous: TagState[] }>('/api/media', { select, ...edit });

/** Undo: puts each item back exactly as it was, rather than replaying an inverse. */
export const bulkRestore = (previous: TagState[]) =>
  api.patch<{ updated: number }>('/api/media', { restore: previous });

/** The count has to be echoed back, and the server re-checks it. */
export const bulkDelete = (select: Selection, confirm: number) =>
  api.post<{ deleted: number }>('/api/media/delete', { select, confirm });

/** Moves files on disk; runs as a background task. */
export const moveMedia = (select: Selection, place: Place) =>
  api.post<{ taskId: number; count: number }>('/api/media/move', { select, ...place });

export const startImport = (urls: string[], tags: string[], place: Place | null) =>
  api.post<{ batchId: string; jobs: ImportJob[] }>('/api/import', {
    urls,
    tags,
    rootId: place?.rootId,
    folder: place?.folder ?? '',
  });

export const recentBatches = () => api.get<{ batches: BatchSummary[] }>('/api/import');

export const getBatch = (batchId: string) => api.get<{ jobs: ImportJob[] }>(`/api/import/${batchId}`);

type Refile = { tags?: string[]; rootId?: number; folder?: string };

export const patchBatch = (batchId: string, body: Refile) =>
  api.patch<{ updated: number }>(`/api/import/${batchId}`, body);

export const patchJob = (batchId: string, jobId: number, body: Refile) =>
  api.patch<{ updated: number }>(`/api/import/${batchId}/job/${jobId}`, body);

export const cancelJob = (batchId: string, jobId: number) =>
  api.del<{ ok: boolean }>(`/api/import/${batchId}/job/${jobId}`);

export const retryJob = (batchId: string, jobId: number) =>
  api.post<{ ok: boolean }>(`/api/import/${batchId}/job/${jobId}/retry`);

export const getTasks = () => api.get<{ tasks: Task[] }>('/api/tasks');
export const getSettings = () => api.get<Settings>('/api/settings');
export const saveSettings = (body: { downloadPlace?: Place | null; ytdlpArgs?: string }) =>
  api.put<Settings>('/api/settings', body);
export const getYtdlp = () => api.get<YtdlpStatus>('/api/ytdlp');
export const installYtdlp = () => api.post<{ ok: boolean }>('/api/ytdlp/install');
export const getTrash = () =>
  api.get<{ trashDays: number; roots: { rootId: number; name: string; bytes: number }[] }>('/api/trash');
export const emptyTrash = () => api.del<{ freed: number }>('/api/trash');

/**
 * Uploads one file into a folder, reporting progress.
 *
 * XMLHttpRequest rather than fetch: fetch still cannot report *upload*
 * progress in any shipping browser, and a progress bar that jumps from 0 to
 * 100 is worse than none for a file that takes a minute.
 *
 * The body is the raw file rather than multipart, so the bytes stream straight
 * to disk instead of the browser and the server both buffering part
 * boundaries.
 */
export function upload(
  file: File,
  opts: { title?: string; tags?: string[]; place?: Place | null },
  onProgress?: (sent: number, total: number) => void,
): Promise<Media> {
  const q = new URLSearchParams();
  q.set('name', file.name);
  if (opts.title) q.set('title', opts.title);
  if (opts.tags?.length) q.set('tags', opts.tags.join(','));
  if (opts.place) {
    q.set('rootId', String(opts.place.rootId));
    q.set('folder', opts.place.folder);
  }

  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open('POST', `/api/upload?${q}`);
    xhr.withCredentials = true;

    xhr.upload.onprogress = (e) => {
      if (e.lengthComputable) onProgress?.(e.loaded, e.total);
    };
    xhr.onload = () => {
      let body: { error?: string } = {};
      try {
        body = JSON.parse(xhr.responseText);
      } catch {
        // A non-JSON body means something upstream answered instead of the
        // API; the status is the useful part.
      }
      if (xhr.status >= 200 && xhr.status < 300) resolve(body as Media);
      else reject(new ApiError(xhr.status, body.error ?? `upload failed (${xhr.status})`));
    };
    xhr.onerror = () => reject(new ApiError(0, 'upload failed: connection lost'));
    xhr.onabort = () => reject(new ApiError(0, 'upload cancelled'));
    xhr.send(file);
  });
}

// ── formatting ─────────────────────────────────────────────────────────

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let value = n / 1024;
  let i = 0;
  while (value >= 1024 && i < units.length - 1) {
    value /= 1024;
    i++;
  }
  return `${value.toFixed(value < 10 ? 1 : 0)} ${units[i]}`;
}

export function formatDuration(seconds: number): string {
  if (!seconds || seconds < 0) return '0:00';
  const total = Math.floor(seconds);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const mm = h > 0 ? String(m).padStart(2, '0') : String(m);
  return h > 0 ? `${h}:${mm}:${String(s).padStart(2, '0')}` : `${mm}:${String(s).padStart(2, '0')}`;
}
