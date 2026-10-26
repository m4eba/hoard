import { useEffect, useMemo, useState } from 'react';
import { Link, useParams } from 'react-router';
import { useQueryClient } from '@tanstack/react-query';
import { cancelJob, getBatch, jobIsTerminal, patchBatch, patchJob, retryJob, type ImportJob } from '../api';
import { CardGrid, type CardItem, type Shared } from '../components/CardGrid';

/**
 * A batch of URL downloads, updated live.
 *
 * Progress arrives over server-sent events rather than polling. The stream
 * opens with a snapshot of every job, so a page opened late — or reopened —
 * shows the truth immediately instead of waiting for the next change.
 *
 * The cards are the same component uploads use: a download and an upload
 * differ only in where the bytes come from.
 */
export function ImportBatch() {
  const { batchId = '' } = useParams();
  const queryClient = useQueryClient();
  const [jobs, setJobs] = useState<Record<number, ImportJob>>({});
  const [connected, setConnected] = useState(false);
  const [shared, setShared] = useState<Shared>({ tags: [], place: null });
  // Bumped by a retry: the stream closes once everything has finished, and a
  // retried job needs it open again.
  const [epoch, setEpoch] = useState(0);

  useEffect(() => {
    let cancelled = false;
    // Seed from the REST endpoint too: if the stream cannot be opened at all
    // the cards should still render rather than showing an empty screen.
    getBatch(batchId)
      .then((res) => {
        if (cancelled) return;
        setJobs((prev) => ({ ...prev, ...byId(res.jobs) }));
      })
      .catch(() => {});

    const source = new EventSource(`/api/import/${batchId}/events`);
    source.onopen = () => setConnected(true);
    source.onmessage = (e) => {
      try {
        const job = JSON.parse(e.data) as ImportJob;
        if (job.id) setJobs((prev) => ({ ...prev, [job.id]: job }));
      } catch {
        // A frame we cannot parse is not worth tearing the stream down for.
      }
    };
    source.addEventListener('done', () => {
      source.close();
      setConnected(false);
      // Everything landed: the library and folder counts are stale now.
      void queryClient.invalidateQueries();
    });
    source.onerror = () => setConnected(false);

    return () => {
      cancelled = true;
      source.close();
    };
  }, [batchId, queryClient, epoch]);

  const list = useMemo(() => Object.values(jobs).sort((a, b) => a.id - b.id), [jobs]);
  const running = list.filter((j) => !jobIsTerminal(j.state)).length;

  const items: CardItem[] = list.map((job) => ({
    id: String(job.id),
    title: job.title || job.url,
    subtitle: job.url,
    thumbUrl: job.thumbUrl,
    state: job.state,
    percent: job.percent,
    bytesDone: job.bytesDone,
    bytesTotal: job.bytesTotal,
    error: job.state === 'failed' ? job.error : undefined,
    tags: job.tags ?? [],
    place: job.rootId !== undefined && job.rootId !== null ? { rootId: job.rootId, folder: job.folder } : null,
    mediaId: job.mediaId,
  }));

  const refresh = () =>
    getBatch(batchId)
      .then((res) => {
        setJobs((prev) => ({ ...prev, ...byId(res.jobs) }));
        return queryClient.invalidateQueries();
      })
      .catch(() => {});

  return (
    <div className="mx-auto max-w-6xl">
      <div className="mb-4 flex flex-wrap items-baseline gap-3">
        <h1 className="text-xl font-semibold">Downloading</h1>
        {running > 0 && connected && <span className="text-sm text-muted">live</span>}
        <Link to="/import" className="ml-auto text-sm text-accent hover:underline">
          ← More links
        </Link>
        <Link to="/library" className="text-sm text-accent hover:underline">
          Go to library →
        </Link>
      </div>

      <CardGrid
        heading="Links"
        items={items}
        shared={shared}
        onRetag={(id, next) => {
          const jobId = Number(id);
          if (next.tags) {
            // Optimistic, so typing neither waits for a round trip nor gets
            // undone by the next progress frame.
            setJobs((prev) => ({ ...prev, [jobId]: { ...prev[jobId], tags: next.tags! } }));
          }
          if (next.place) {
            const { rootId, folder } = next.place;
            setJobs((prev) => ({ ...prev, [jobId]: { ...prev[jobId], rootId, folder } }));
          }
          void patchJob(batchId, jobId, { tags: next.tags, ...next.place })
            // A finished card writes straight to its media, so the library is
            // stale the moment it succeeds.
            .then(() => queryClient.invalidateQueries())
            .catch((err) => alert((err as Error).message));
        }}
        onShared={(next) => {
          setShared((prev) => ({ ...prev, ...next }));
          void patchBatch(batchId, { tags: next.tags, ...next.place })
            .then(refresh)
            .catch((err) => alert((err as Error).message));
        }}
        onCancel={(id) => void cancelJob(batchId, Number(id)).catch(() => {})}
        onRetry={(id) =>
          void retryJob(batchId, Number(id))
            .then(() => setEpoch((n) => n + 1))
            .catch((err) => alert((err as Error).message))
        }
      />
    </div>
  );
}

const byId = (jobs: ImportJob[]) => Object.fromEntries(jobs.map((j) => [j.id, j]));
