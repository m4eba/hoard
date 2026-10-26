import { useEffect } from 'react';
import { Navigate, Route, Routes } from 'react-router';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { getConfig } from './api';
import { Shell } from './components/Shell';
import { Library, SearchResults } from './routes/Library';
import { ImportBatch } from './routes/ImportBatch';
import { ImportPage } from './routes/ImportPage';
import { Login } from './routes/Login';
import { OAuthConsent } from './routes/OAuthConsent';
import { Settings } from './routes/Settings';
import { Viewer } from './viewer/Viewer';

export function App() {
  const queryClient = useQueryClient();
  const { data: config, isLoading, error } = useQuery({
    queryKey: ['config'],
    queryFn: getConfig,
    staleTime: 30_000,
  });

  // Any request answered 401 means the session is gone; asking the server
  // again is what flips the app back to the login screen.
  useEffect(() => {
    const onUnauthorized = () => void queryClient.invalidateQueries({ queryKey: ['config'] });
    window.addEventListener('hoard:unauthorized', onUnauthorized);
    return () => window.removeEventListener('hoard:unauthorized', onUnauthorized);
  }, [queryClient]);

  if (isLoading) {
    return <div className="flex h-full items-center justify-center text-muted">Loading…</div>;
  }
  if (error || !config) {
    return (
      <div className="flex h-full items-center justify-center text-muted">
        Cannot reach the server: {(error as Error | null)?.message ?? 'no answer'}
      </div>
    );
  }
  if (config.authRequired && !config.authenticated) return <Login />;

  const page = (el: React.ReactNode) => <Shell authRequired={config.authRequired}>{el}</Shell>;

  return (
    <Routes>
      <Route path="/" element={<Navigate to="/library" replace />} />
      <Route path="/library" element={page(<Library />)} />
      <Route path="/search" element={page(<SearchResults />)} />
      <Route path="/import" element={page(<ImportPage />)} />
      <Route path="/import/:batchId" element={page(<ImportBatch />)} />
      <Route path="/settings" element={page(<Settings version={config.version} />)} />

      {/* The viewer is a full-screen overlay route, so Escape returns to
          whatever grid was underneath rather than to a fixed page. */}
      <Route path="/m/:id" element={<Viewer />} />

      {/* Where fiffly sends you to connect: a page of its own, no app chrome. */}
      <Route path="/oauth/authorize" element={<OAuthConsent />} />

      <Route path="*" element={<Navigate to="/library" replace />} />
    </Routes>
  );
}
