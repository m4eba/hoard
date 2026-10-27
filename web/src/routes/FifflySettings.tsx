import { useEffect, useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { getSettings, saveSettings } from '../api';
import { FolderInput } from '../components/FolderInput';
import { addSiteUrl, fifflyTarget, listConnections, revokeConnection } from '../fiffly';
import { formatRelative } from '../format';
import { button, input, Section } from './settingsUi';

export function FifflySection() {
  const queryClient = useQueryClient();
  const { data: settings } = useQuery({ queryKey: ['settings'], queryFn: getSettings });
  const target = fifflyTarget(settings);
  const [fiffly, setFiffly] = useState('');
  const [site, setSite] = useState('');
  const [status, setStatus] = useState('');
  useEffect(() => setFiffly(settings?.fifflyUrl ?? ''), [settings?.fifflyUrl]);
  useEffect(() => setSite(settings?.fifflySite ?? ''), [settings?.fifflySite]);

  async function save(body: Parameters<typeof saveSettings>[0]) {
    setStatus('');
    try {
      queryClient.setQueryData(['settings'], await saveSettings(body));
      setStatus('Saved');
      setTimeout(() => setStatus(''), 1500);
    } catch (err) {
      setStatus((err as Error).message);
    }
  }

  const dirty = settings && (fiffly !== settings.fifflyUrl || site !== (settings.fifflySite ?? ''));

  return (
    <Section title="fiffly" hint="Open videos and images in the fiffly editor, and save its exports back here.">
      <div className="space-y-4 rounded-lg border border-edge bg-panel p-4">
        <div className="flex flex-wrap items-center gap-3">
          <a
            href={addSiteUrl(target)}
            target="_blank"
            rel="noreferrer"
            className="rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-ink hover:opacity-90"
          >
            Add hoard to fiffly
          </a>
          <p className="min-w-0 flex-1 text-xs text-muted">
            fiffly asks you to confirm, then to connect — which brings you back here to allow access. Every video
            and image also has an <span className="text-white/80">Open in fiffly</span> link, which works without
            connecting.
          </p>
        </div>
        {target.problem && (
          <p className="rounded-md border border-amber-400/30 bg-amber-400/10 px-3 py-2 text-xs text-amber-200">
            {target.problem} Open hoard as <span className="font-mono">http://localhost:…</span> on this computer, or
            put it behind https and enter that address below.
          </p>
        )}

        <div className="grid gap-3 sm:grid-cols-2">
          <label className="block">
            <span className="mb-1 block text-xs text-muted">fiffly address</span>
            <input
              value={fiffly}
              onChange={(e) => setFiffly(e.target.value)}
              placeholder={settings?.defaultFifflyUrl}
              spellCheck={false}
              className={`${input} font-mono`}
            />
          </label>
          <label className="block">
            <span className="mb-1 block text-xs text-muted">hoard’s address, as fiffly reaches it</span>
            <input
              value={site}
              onChange={(e) => setSite(e.target.value)}
              placeholder={window.location.origin}
              spellCheck={false}
              className={`${input} font-mono`}
            />
          </label>
        </div>
        <div className="-mt-1 flex items-center gap-2">
          <button
            onClick={() => void save({ fifflyUrl: fiffly || settings?.defaultFifflyUrl, fifflySite: site || null })}
            disabled={!dirty}
            className={button}
          >
            Save
          </button>
          <span className="text-xs text-muted">{status}</span>
        </div>
        <p className="-mt-2 text-xs text-muted">
          fiffly runs in your browser, so it uses the address you have hoard open at — fill this in only if that
          differs, e.g. an https address in front of hoard. Only the fiffly address above may ask for access.
        </p>

        <div>
          <span className="mb-1 block text-xs text-muted">Default folder for exports saved from fiffly (the save dialog can pick another)</span>
          <FolderInput
            value={settings?.fifflyPlace ?? null}
            allowEmpty
            placeholder="The default download folder"
            onCommit={(place) => void save({ fifflyPlace: place })}
          />
        </div>

        <Connections />
      </div>
    </Section>
  );
}

function Connections() {
  const queryClient = useQueryClient();
  const { data } = useQuery({ queryKey: ['fiffly-connections'], queryFn: listConnections });
  const connections = data?.connections ?? [];

  return (
    <div>
      <span className="mb-1 block text-xs text-muted">Connected</span>
      {connections.length === 0 ? (
        <p className="text-sm text-muted">Nothing connected yet.</p>
      ) : (
        <ul className="divide-y divide-edge rounded-md border border-edge">
          {connections.map((c) => (
            <li key={c.id} className="flex items-center gap-3 px-3 py-2 text-sm">
              <span className="font-medium">{c.client}</span>
              <span className="min-w-0 flex-1 truncate text-xs text-muted">
                {c.scopes.includes('media:write') ? 'open and save' : 'open'} · connected {formatRelative(c.createdAt)}
                {c.lastUsed && ` · last used ${formatRelative(c.lastUsed)}`}
              </span>
              <button
                onClick={async () => {
                  await revokeConnection(c.id);
                  await queryClient.invalidateQueries({ queryKey: ['fiffly-connections'] });
                }}
                className="text-xs text-muted hover:text-red-400"
              >
                Revoke
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
