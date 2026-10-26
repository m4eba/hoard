import { useRef, useState } from 'react';
import { Link, useNavigate } from 'react-router';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { getSettings, getYtdlp, installYtdlp, recentBatches, startImport, type Place } from '../api';
import { CardGrid } from '../components/CardGrid';
import { FolderInput } from '../components/FolderInput';
import { TagInput } from '../components/TagInput';
import { useUploads } from '../components/useUploads';
import { formatDateTime } from '../format';
import { placeLabel, useRoots } from '../places';

/**
 * The import screen: a box for a wall of links, where they should land, a
 * drop area for files, and the cards for whatever is currently arriving.
 */
export function ImportPage() {
  const navigate = useNavigate();
  const roots = useRoots();
  const { data: settings } = useQuery({ queryKey: ['settings'], queryFn: getSettings });
  const [text, setText] = useState('');
  const [place, setPlace] = useState<Place | null>(null);
  const [tags, setTags] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const destination = place ?? settings?.downloadPlace ?? null;
  const { items, accept, retag, clear, shared, setShared } = useUploads(destination);

  const urls = text
    .split(/\s+/)
    .map((s) => s.trim())
    .filter((s) => /^https?:\/\//.test(s));

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    if (urls.length === 0 || busy) return;
    setBusy(true);
    setError('');
    try {
      // Per-item tagging and filing happens on the cards once the batch
      // exists; what is chosen here is the starting point for all of them.
      const batch = await startImport(urls, tags, place);
      navigate(`/import/${batch.batchId}`);
    } catch (err) {
      setError((err as Error).message);
      setBusy(false);
    }
  }

  const noRoots = roots && roots.length === 0;

  return (
    <div className="mx-auto max-w-4xl">
      {noRoots && (
        <p className="mb-4 rounded-md border border-amber-700/40 bg-amber-950/30 px-3 py-2 text-sm text-amber-200/90">
          Downloads and uploads need somewhere to go.{' '}
          <Link to="/settings" className="font-medium underline">
            Add a storage folder
          </Link>{' '}
          first.
        </p>
      )}
      <YtdlpNotice />

      <form onSubmit={submit}>
        <label htmlFor="urls" className="mb-1 block text-sm font-medium">
          Download from links
        </label>
        <textarea
          id="urls"
          value={text}
          onChange={(e) => setText(e.target.value)}
          rows={6}
          autoFocus
          placeholder={'https://example.com/video\nhttps://example.com/another'}
          className="w-full resize-y rounded-lg border border-edge bg-panel p-3 font-mono text-sm outline-none focus:border-accent"
          onKeyDown={(e) => {
            // Ctrl/Cmd+Enter submits; Enter has to stay a newline in a box
            // whose whole purpose is many lines.
            if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') void submit(e);
          }}
        />

        <div className="mt-2 grid gap-2 sm:grid-cols-[1fr_1fr]">
          <FolderInput
            value={place}
            allowEmpty
            onCommit={setPlace}
            placeholder={
              settings?.downloadPlace
                ? `Default: ${placeLabel(settings.downloadPlace, roots)}`
                : roots?.length
                  ? `Default: ${roots[0].name}`
                  : 'Folder'
            }
          />
          <TagInput tags={tags} onChange={setTags} compact placeholder="Tags for these links…" />
        </div>

        <div className="mt-2 flex items-center gap-3">
          <button
            type="submit"
            disabled={urls.length === 0 || busy || noRoots}
            className="rounded-md bg-accent px-4 py-1.5 text-sm font-medium text-ink disabled:opacity-40"
          >
            {busy ? 'Starting…' : `Download ${urls.length || ''} ${urls.length === 1 ? 'link' : 'links'}`}
          </button>
          <span className="text-xs text-muted">⌘/Ctrl + Enter</span>
        </div>
        {error && <p className="mt-2 text-sm text-red-400">{error}</p>}
      </form>

      <DropArea onFiles={accept} />

      <CardGrid
        heading="Uploads"
        items={items}
        shared={shared}
        onRetag={retag}
        onShared={setShared}
        onClear={clear}
      />

      <RecentBatches />
    </div>
  );
}

/** Says plainly why links cannot be downloaded yet, and fixes it in one click. */
function YtdlpNotice() {
  const queryClient = useQueryClient();
  const { data } = useQuery({
    queryKey: ['ytdlp'],
    queryFn: getYtdlp,
    // Poll while an install runs; it takes a minute and has no other signal
    // on this page.
    refetchInterval: (q) => (q.state.data?.updating ? 2000 : false),
  });
  const [error, setError] = useState('');
  if (!data || data.installed) return null;

  return (
    <div className="mb-4 rounded-md border border-edge bg-panel px-3 py-2 text-sm">
      {data.updating ? (
        <p className="text-muted">Installing yt-dlp… this takes a minute.</p>
      ) : data.available && data.managed ? (
        <div className="flex flex-wrap items-center gap-3">
          <p className="text-muted">
            Downloading from links needs yt-dlp. It is installed into its own Python environment in
            the data folder and kept up to date.
          </p>
          <button
            onClick={async () => {
              setError('');
              try {
                await installYtdlp();
                await queryClient.invalidateQueries({ queryKey: ['ytdlp'] });
              } catch (err) {
                setError((err as Error).message);
              }
            }}
            className="rounded-md bg-accent px-3 py-1 text-sm font-medium text-ink"
          >
            Install yt-dlp
          </button>
        </div>
      ) : data.available ? (
        <p className="text-muted">The configured yt-dlp binary could not be run.</p>
      ) : (
        <p className="text-muted">
          Downloading from links needs Python 3 for yt-dlp, and none was found. Install python3 or set
          HOARD_PYTHON; uploads work without it.
        </p>
      )}
      {(error || data.error) && <p className="mt-1 text-xs text-red-400">{error || data.error}</p>}
    </div>
  );
}

function RecentBatches() {
  const { data } = useQuery({ queryKey: ['batches'], queryFn: recentBatches });
  const batches = data?.batches ?? [];
  if (batches.length === 0) return null;
  return (
    <section className="mt-8">
      <h2 className="mb-2 text-sm font-medium">Recent downloads</h2>
      <div className="divide-y divide-edge overflow-hidden rounded-lg border border-edge">
        {batches.map((b) => (
          <Link
            key={b.batchId}
            to={`/import/${b.batchId}`}
            className="flex items-center gap-3 bg-panel px-3 py-2 text-sm hover:bg-edge/50"
          >
            <span className="min-w-0 flex-1 truncate">
              {b.firstTitle}
              {b.total > 1 && <span className="text-muted"> and {b.total - 1} more</span>}
            </span>
            {b.running > 0 && <span className="text-xs text-accent">{b.running} running</span>}
            {b.failed > 0 && <span className="text-xs text-red-400">{b.failed} failed</span>}
            <span className="text-xs text-muted tabular-nums">
              {b.done}/{b.total}
            </span>
            <span className="hidden w-36 text-right text-xs text-muted tabular-nums sm:block">
              {formatDateTime(b.createdAt)}
            </span>
          </Link>
        ))}
      </div>
    </section>
  );
}

/** Drop target that is also a button, since not everyone drags. */
function DropArea({ onFiles }: { onFiles: (files: FileList | null) => void }) {
  const [over, setOver] = useState(false);
  const input = useRef<HTMLInputElement>(null);

  return (
    <div
      onDragOver={(e) => {
        e.preventDefault();
        setOver(true);
      }}
      onDragLeave={() => setOver(false)}
      onDrop={(e) => {
        e.preventDefault();
        setOver(false);
        onFiles(e.dataTransfer.files);
      }}
      onClick={() => input.current?.click()}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') input.current?.click();
      }}
      className={`mt-6 cursor-pointer rounded-lg border-2 border-dashed p-8 text-center transition ${
        over ? 'border-accent bg-accent/10' : 'border-edge hover:border-muted'
      }`}
    >
      <p className="text-sm">
        Drop files here, or <span className="text-accent underline">choose files</span>
      </p>
      <p className="mt-1 text-xs text-muted">Images and video, into the folder chosen above</p>
      <input
        ref={input}
        type="file"
        multiple
        accept="video/*,image/*,.mkv,.ts,.m2ts,.mts"
        className="hidden"
        onChange={(e) => {
          onFiles(e.target.files);
          // Reset, so choosing the same file twice in a row still fires.
          e.target.value = '';
        }}
      />
    </div>
  );
}
