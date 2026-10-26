import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { getSettings } from '../api';
import { answerAuthorize, checkAuthorize } from '../fiffly';
import { placeLabel, useRoots } from '../places';

/**
 * The page fiffly sends you to when you connect hoard there. It asks once;
 * either answer goes straight back to fiffly.
 */
export function OAuthConsent() {
  const search = window.location.search;
  const { data, error } = useQuery({ queryKey: ['oauth', search], queryFn: () => checkAuthorize(search), retry: false });
  const { data: settings } = useQuery({ queryKey: ['settings'], queryFn: getSettings });
  const roots = useRoots();
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState('');

  async function answer(approve: boolean) {
    setBusy(true);
    try {
      const params = Object.fromEntries(new URLSearchParams(search));
      window.location.href = (await answerAuthorize(params, approve)).redirectTo;
    } catch (err) {
      setFailure((err as Error).message);
      setBusy(false);
    }
  }

  const exports = settings?.fifflyPlace ?? settings?.downloadPlace;
  const scopeText: Record<string, string> = {
    'media:read': 'See everything in your library and open it in the editor',
    'media:write': `Save exports into ${exports ? placeLabel(exports, roots) : 'your default download folder'}`,
  };

  return (
    <div className="flex min-h-full items-center justify-center p-6">
      <div className="w-full max-w-md rounded-xl border border-edge bg-panel p-6">
        {error ? (
          <>
            <h1 className="text-lg font-semibold">This request can’t be shown</h1>
            <p className="mt-2 text-sm text-red-400">{(error as Error).message}</p>
          </>
        ) : !data ? (
          <p className="text-sm text-muted">Loading…</p>
        ) : (
          <>
            <h1 className="text-lg font-semibold">Connect {data.client} to hoard?</h1>
            <p className="mt-1 text-sm text-muted">{data.client} will be able to:</p>
            <ul className="mt-3 space-y-1.5 text-sm">
              {data.scopes.map((s) => (
                <li key={s} className="flex gap-2">
                  <span className="text-accent">✓</span>
                  {scopeText[s] ?? s}
                </li>
              ))}
            </ul>
            <p className="mt-4 text-xs text-muted">
              Either way, you go back to <span className="font-mono">{data.redirectHost}</span>. Access stays until you
              disconnect there or revoke it in Settings.
            </p>
            {failure && <p className="mt-3 text-sm text-red-400">{failure}</p>}
            <div className="mt-5 flex justify-end gap-2">
              <button
                onClick={() => void answer(false)}
                disabled={busy}
                className="rounded-md border border-edge px-3 py-1.5 text-sm hover:bg-edge disabled:opacity-40"
              >
                Deny
              </button>
              <button
                onClick={() => void answer(true)}
                disabled={busy}
                className="rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-ink hover:opacity-90 disabled:opacity-40"
              >
                Allow
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
