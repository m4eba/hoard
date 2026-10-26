import { useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { login } from '../api';

/** The one screen shown while a password is configured and not yet entered. */
export function Login() {
  const queryClient = useQueryClient();
  const [password, setPassword] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    if (!password || busy) return;
    setBusy(true);
    setError('');
    try {
      await login(password);
      // Everything cached so far was a refusal.
      await queryClient.invalidateQueries();
    } catch (err) {
      setError((err as Error).message);
      setBusy(false);
    }
  }

  return (
    <div className="flex h-full items-center justify-center px-6">
      <form onSubmit={submit} className="w-full max-w-sm rounded-lg border border-edge bg-panel p-6">
        <h1 className="text-2xl font-semibold tracking-tight">hoard</h1>
        <p className="mt-1 text-sm text-muted">This library is password protected.</p>
        <input
          type="password"
          autoFocus
          autoComplete="current-password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          placeholder="Password"
          className="mt-5 w-full rounded-md border border-edge bg-ink px-3 py-2 text-sm outline-none focus:border-accent"
        />
        {error && <p className="mt-2 text-sm text-red-400">{error}</p>}
        <button
          type="submit"
          disabled={!password || busy}
          className="mt-4 w-full rounded-md bg-accent px-4 py-2 text-sm font-medium text-ink disabled:opacity-40"
        >
          {busy ? 'Signing in…' : 'Sign in'}
        </button>
      </form>
    </div>
  );
}
