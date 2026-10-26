import { useEffect, useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import {
  addRoot,
  emptyTrash,
  formatBytes,
  getSettings,
  getTrash,
  getYtdlp,
  installYtdlp,
  removeRoot,
  renameRoot,
  saveSettings,
  scanRoot,
  type Root,
} from '../api';
import { DirectoryPicker } from '../components/DirectoryPicker';
import { FolderInput } from '../components/FolderInput';
import { formatDateTime, formatRelative, locale, setLocale } from '../format';
import { useRoots } from '../places';

export function Settings({ version }: { version: string }) {
  return (
    <div className="mx-auto max-w-3xl space-y-8 pb-12">
      <StorageSection />
      <DownloadSection />
      <TrashSection />
      <DisplaySection />
      <p className="text-xs text-muted">hoard {version}</p>
    </div>
  );
}

function Section({ title, hint, children }: { title: string; hint?: string; children: React.ReactNode }) {
  return (
    <section>
      <h2 className="text-base font-semibold">{title}</h2>
      {hint && <p className="mt-0.5 text-sm text-muted">{hint}</p>}
      <div className="mt-3">{children}</div>
    </section>
  );
}

const button = 'rounded-md border border-edge px-3 py-1.5 text-sm hover:bg-edge disabled:opacity-40';
const input = 'w-full rounded-md border border-edge bg-ink px-2 py-1.5 text-sm outline-none focus:border-accent';

function StorageSection() {
  const queryClient = useQueryClient();
  const roots = useRoots();
  const [path, setPath] = useState('');
  const [name, setName] = useState('');
  const [picking, setPicking] = useState(false);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);

  async function add(e: React.FormEvent) {
    e.preventDefault();
    if (!path.trim() || busy) return;
    setBusy(true);
    setError('');
    try {
      await addRoot(path.trim(), name.trim());
      setPath('');
      setName('');
      await queryClient.invalidateQueries();
    } catch (err) {
      setError((err as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Section
      title="Storage folders"
      hint="Directories on the server that make up the library. Each is scanned for videos and images; the folders inside them are the library's folders."
    >
      <div className="divide-y divide-edge overflow-hidden rounded-lg border border-edge">
        {roots?.length === 0 && <p className="bg-panel p-3 text-sm text-muted">None yet — add one below.</p>}
        {roots?.map((r) => (
          <RootRow key={r.id} root={r} />
        ))}
      </div>

      <form onSubmit={add} className="mt-3 grid gap-2 sm:grid-cols-[1fr_10rem_auto_auto]">
        <input value={path} onChange={(e) => setPath(e.target.value)} placeholder="/path/on/the/server" className={`${input} font-mono`} />
        <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Name (optional)" className={input} />
        <button type="button" onClick={() => setPicking(true)} className={button}>
          Browse…
        </button>
        <button type="submit" disabled={!path.trim() || busy} className="rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-ink disabled:opacity-40">
          {busy ? 'Adding…' : 'Add'}
        </button>
      </form>
      {error && <p className="mt-2 text-sm text-red-400">{error}</p>}

      {picking && (
        <DirectoryPicker
          initial={path}
          onClose={() => setPicking(false)}
          onPick={(p) => {
            setPath(p);
            setPicking(false);
          }}
        />
      )}
    </Section>
  );
}

function RootRow({ root }: { root: Root }) {
  const queryClient = useQueryClient();
  const [name, setName] = useState(root.name);
  const [error, setError] = useState('');
  useEffect(() => setName(root.name), [root.name]);

  const run = async (f: () => Promise<unknown>) => {
    setError('');
    try {
      await f();
      await queryClient.invalidateQueries();
    } catch (err) {
      setError((err as Error).message);
    }
  };

  return (
    <div className="bg-panel p-3">
      <div className="flex flex-wrap items-center gap-2">
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          onBlur={() => name.trim() && name !== root.name && void run(() => renameRoot(root.id, name.trim()))}
          onKeyDown={(e) => e.key === 'Enter' && (e.target as HTMLInputElement).blur()}
          className="w-40 rounded border border-transparent bg-transparent px-1 py-0.5 text-sm font-medium outline-none hover:border-edge focus:border-accent"
          aria-label="Name"
        />
        <span className={`text-xs ${root.online ? 'text-green-400' : 'text-red-400'}`}>
          {root.online ? (root.scanning ? 'scanning…' : 'online') : 'not reachable'}
        </span>
        <span className="ml-auto text-xs text-muted tabular-nums">
          {root.count} item{root.count === 1 ? "" : "s"} · {formatBytes(root.bytes)}
        </span>
        <button onClick={() => void run(() => scanRoot(root.id))} disabled={root.scanning} className={button}>
          Rescan
        </button>
        <button
          onClick={() => {
            if (confirm(`Remove “${root.name}” from the library? Files on disk are not touched; tags and titles are forgotten.`))
              void run(() => removeRoot(root.id));
          }}
          className="rounded-md border border-red-900/60 px-3 py-1.5 text-sm text-red-300 hover:bg-red-950/50"
        >
          Remove
        </button>
      </div>
      <p className="mt-1 truncate px-1 font-mono text-xs text-muted" title={root.path}>
        {root.path}
        {root.lastScanAt && <span className="font-sans"> · scanned {formatRelative(root.lastScanAt)}</span>}
      </p>
      {error && <p className="mt-1 px-1 text-xs text-red-400">{error}</p>}
    </div>
  );
}

function DownloadSection() {
  const queryClient = useQueryClient();
  const { data: settings } = useQuery({ queryKey: ['settings'], queryFn: getSettings });
  const { data: ytdlp } = useQuery({
    queryKey: ['ytdlp'],
    queryFn: getYtdlp,
    refetchInterval: (q) => (q.state.data?.updating ? 2000 : false),
  });
  const [args, setArgs] = useState('');
  const [status, setStatus] = useState('');
  useEffect(() => setArgs(settings?.ytdlpArgs ?? ''), [settings?.ytdlpArgs]);

  async function save(body: Parameters<typeof saveSettings>[0]) {
    setStatus('');
    try {
      const next = await saveSettings(body);
      queryClient.setQueryData(['settings'], next);
      setStatus('Saved');
      setTimeout(() => setStatus(''), 1500);
    } catch (err) {
      setStatus((err as Error).message);
    }
  }

  return (
    <Section title="Downloads" hint="Links are downloaded with yt-dlp.">
      <div className="space-y-4 rounded-lg border border-edge bg-panel p-4">
        <div className="flex flex-wrap items-center gap-3 text-sm">
          <span className="font-medium">yt-dlp</span>
          {ytdlp?.installed ? (
            <span className="text-muted">
              {ytdlp.version}
              {ytdlp.lastUpdate && ` · updated ${formatRelative(ytdlp.lastUpdate)}`}
            </span>
          ) : (
            <span className="text-amber-300/90">{ytdlp?.available ? 'not installed' : 'unavailable — no Python 3 found'}</span>
          )}
          {ytdlp?.managed && ytdlp.available && (
            <button
              onClick={async () => {
                try {
                  await installYtdlp();
                  await queryClient.invalidateQueries({ queryKey: ['ytdlp'] });
                } catch (err) {
                  setStatus((err as Error).message);
                }
              }}
              disabled={ytdlp.updating}
              className={`${button} ml-auto`}
            >
              {ytdlp.updating ? 'Working…' : ytdlp.installed ? 'Update now' : 'Install'}
            </button>
          )}
        </div>
        {ytdlp?.managed && (
          <p className="-mt-2 text-xs text-muted">
            Installed in <span className="font-mono">{ytdlp.envPath}</span> and updated daily.
          </p>
        )}
        {ytdlp?.error && <p className="text-xs text-red-400">{ytdlp.error}</p>}

        <div>
          <span className="mb-1 block text-xs text-muted">Default download folder</span>
          <FolderInput
            value={settings?.downloadPlace ?? null}
            allowEmpty
            placeholder="The first storage folder"
            onCommit={(place) => void save({ downloadPlace: place })}
          />
        </div>

        <div>
          <span className="mb-1 block text-xs text-muted">Extra yt-dlp arguments</span>
          <textarea
            value={args}
            onChange={(e) => setArgs(e.target.value)}
            rows={2}
            spellCheck={false}
            className={`${input} font-mono`}
          />
          <div className="mt-1 flex items-center gap-2">
            <button onClick={() => void save({ ytdlpArgs: args })} disabled={args === settings?.ytdlpArgs} className={button}>
              Save
            </button>
            {settings && args !== settings.defaultYtdlpArgs && (
              <button onClick={() => setArgs(settings.defaultYtdlpArgs)} className="text-xs text-muted hover:text-white">
                Reset to default
              </button>
            )}
            <span className="text-xs text-muted">{status}</span>
          </div>
          <p className="mt-1 text-xs text-muted">
            Format choice, cookies (<span className="font-mono">--cookies /path</span>), rate limits — anything yt-dlp
            accepts. The default prefers H.264 in MP4, which every browser plays.
          </p>
        </div>
      </div>
    </Section>
  );
}

function TrashSection() {
  const queryClient = useQueryClient();
  const { data } = useQuery({ queryKey: ['trash'], queryFn: getTrash });
  const [status, setStatus] = useState('');
  const total = data?.roots.reduce((n, r) => n + r.bytes, 0) ?? 0;

  return (
    <Section
      title="Trash"
      hint={
        data?.trashDays
          ? `Deleted files are moved to a .hoard-trash folder inside their storage folder and removed for good after ${data.trashDays} days.`
          : 'Deleted files are removed immediately (HOARD_TRASH_DAYS=0).'
      }
    >
      <div className="flex flex-wrap items-center gap-3 rounded-lg border border-edge bg-panel p-4 text-sm">
        <span>{formatBytes(total)} in the trash</span>
        {data?.roots
          .filter((r) => r.bytes > 0)
          .map((r) => (
            <span key={r.rootId} className="text-xs text-muted">
              {r.name}: {formatBytes(r.bytes)}
            </span>
          ))}
        <button
          onClick={async () => {
            if (!confirm('Permanently delete everything in the trash?')) return;
            try {
              const res = await emptyTrash();
              setStatus(`${formatBytes(res.freed)} freed`);
              await queryClient.invalidateQueries({ queryKey: ['trash'] });
            } catch (err) {
              setStatus((err as Error).message);
            }
          }}
          disabled={total === 0}
          className="ml-auto rounded-md border border-red-900/60 px-3 py-1.5 text-sm text-red-300 hover:bg-red-950/50 disabled:opacity-40"
        >
          Empty trash
        </button>
        {status && <span className="w-full text-xs text-muted">{status}</span>}
      </div>
    </Section>
  );
}

function DisplaySection() {
  const [value, setValue] = useState(locale());
  const sample = formatDateTime(new Date().toISOString());
  return (
    <Section title="Display" hint="How dates are written, in this browser.">
      <div className="flex flex-wrap items-center gap-3 rounded-lg border border-edge bg-panel p-4 text-sm">
        <label className="flex items-center gap-2">
          <span className="text-muted">Locale</span>
          <input
            value={value}
            onChange={(e) => setValue(e.target.value)}
            onBlur={() => {
              if (value.trim() === locale()) return;
              try {
                // Rejects anything Intl does not know, before it is stored.
                new Intl.DateTimeFormat(value);
                setLocale(value.trim());
                window.location.reload();
              } catch {
                setValue(locale());
              }
            }}
            placeholder="en-CA"
            className="w-28 rounded-md border border-edge bg-ink px-2 py-1 font-mono text-sm outline-none focus:border-accent"
          />
        </label>
        <span className="text-xs text-muted">e.g. en-US, en-GB, de-DE · now: {sample}</span>
      </div>
    </Section>
  );
}
