import { formatBytes } from '../api';

/**
 * The card shared by URL imports and file uploads.
 *
 * Both are "a thing arriving that you can tag while it arrives", so they look
 * and behave the same; only the source differs. Keeping one component means a
 * change to how progress reads applies to both.
 */
export type CardState =
  | 'queued'
  | 'extracting'
  | 'downloading'
  | 'uploading'
  | 'processing'
  | 'done'
  | 'failed'
  | 'cancelled';

export function MediaCard({
  thumbUrl,
  title,
  subtitle,
  state,
  percent,
  bytesDone,
  bytesTotal,
  error,
  controls,
  action,
}: {
  thumbUrl?: string;
  title: string;
  subtitle?: string;
  state: CardState;
  percent: number;
  bytesDone: number;
  bytesTotal: number;
  error?: string;
  /** Tag and album inputs, rendered under the header. */
  controls?: React.ReactNode;
  /** The trailing button: view, retry, cancel. */
  action?: React.ReactNode;
}) {
  const terminal = state === 'done' || state === 'failed' || state === 'cancelled';

  return (
    <div className="flex flex-col gap-2 rounded-lg border border-edge bg-panel p-3">
      <div className="flex items-start gap-2">
        <div className="h-12 w-16 shrink-0 overflow-hidden rounded bg-black/40">
          {thumbUrl && <img src={thumbUrl} alt="" className="h-full w-full object-cover" />}
        </div>
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-medium" title={title}>
            {title}
          </p>
          {subtitle && (
            <p className="truncate text-xs text-muted" title={subtitle}>
              {subtitle}
            </p>
          )}
        </div>
        <StateChip state={state} />
      </div>

      {!terminal && (
        <Progress state={state} percent={percent} done={bytesDone} total={bytesTotal} />
      )}

      {error && (
        <p className="rounded bg-red-950/40 px-2 py-1 text-xs break-words text-red-300">{error}</p>
      )}

      {controls}
      {action && <div className="flex gap-2">{action}</div>}
    </div>
  );
}

function Progress({
  state,
  percent,
  done,
  total,
}: {
  state: CardState;
  percent: number;
  done: number;
  total: number;
}) {
  // A source that never declared a size cannot have a percentage, so show the
  // bytes and an indeterminate bar rather than inventing a number.
  const known = total > 0;
  return (
    <div>
      <div className="h-1 overflow-hidden rounded-full bg-edge">
        <div
          className={`h-full bg-accent transition-[width] ${known ? '' : 'w-1/3 animate-pulse'}`}
          style={known ? { width: `${percent}%` } : undefined}
        />
      </div>
      <p className="mt-1 text-[11px] text-muted tabular-nums">
        {known
          ? `${formatBytes(done)} / ${formatBytes(total)}`
          : done > 0
            ? formatBytes(done)
            : state === 'queued'
              ? 'waiting…'
              : 'starting…'}
      </p>
    </div>
  );
}

const TONE: Record<CardState, string> = {
  queued: 'bg-edge text-muted',
  extracting: 'bg-edge text-white/80',
  downloading: 'bg-blue-900/50 text-blue-200',
  uploading: 'bg-blue-900/50 text-blue-200',
  processing: 'bg-blue-900/50 text-blue-200',
  done: 'bg-green-900/40 text-green-300',
  failed: 'bg-red-900/40 text-red-300',
  cancelled: 'bg-edge text-muted',
};

function StateChip({ state }: { state: CardState }) {
  return (
    <span className={`shrink-0 rounded px-1.5 py-0.5 text-[11px] ${TONE[state]}`}>{state}</span>
  );
}
