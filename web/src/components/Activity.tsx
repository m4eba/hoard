import { useEffect, useRef, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { getTasks } from '../api';

/**
 * Background work in progress — scans, moves, storyboards, yt-dlp updates —
 * as a small indicator with a dropdown. The list is kept current by the event
 * stream; the query only seeds it.
 */
export function Activity() {
  const { data } = useQuery({ queryKey: ['tasks'], queryFn: getTasks });
  const [open, setOpen] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  const tasks = data?.tasks ?? [];
  const running = tasks.filter((t) => t.state === 'running');

  useEffect(() => {
    if (!open) return;
    const onDown = (e: Event) => {
      if (!box.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('pointerdown', onDown);
    return () => document.removeEventListener('pointerdown', onDown);
  }, [open]);

  if (tasks.length === 0) return null;

  return (
    <div ref={box} className="relative">
      <button
        onClick={() => setOpen((o) => !o)}
        className="flex items-center gap-1.5 rounded-md border border-edge px-2 py-1 text-xs text-muted hover:bg-edge hover:text-white"
        aria-label="Background activity"
      >
        {running.length > 0 ? (
          <span className="size-2 animate-pulse rounded-full bg-accent" />
        ) : (
          <span className="size-2 rounded-full bg-green-500/70" />
        )}
        {running.length > 0 ? `${running.length} running` : 'Done'}
      </button>

      {open && (
        <div className="absolute right-0 z-50 mt-1 w-80 rounded-md border border-edge bg-panel p-2 shadow-xl">
          {tasks.map((t) => {
            const pct = t.total > 0 ? Math.min(100, (t.done / t.total) * 100) : null;
            return (
              <div key={t.id} className="rounded px-2 py-1.5">
                <div className="flex items-center gap-2 text-sm">
                  <span className="min-w-0 flex-1 truncate" title={t.label}>
                    {t.label}
                  </span>
                  <span
                    className={`shrink-0 text-[11px] ${
                      t.state === 'failed' ? 'text-red-400' : t.state === 'done' ? 'text-green-400' : 'text-muted'
                    }`}
                  >
                    {t.state === 'running' && pct !== null ? `${Math.round(pct)}%` : t.state}
                  </span>
                </div>
                {t.state === 'running' && (
                  <div className="mt-1 h-1 overflow-hidden rounded-full bg-edge">
                    <div
                      className={`h-full bg-accent ${pct === null ? 'w-1/3 animate-pulse' : ''}`}
                      style={pct === null ? undefined : { width: `${pct}%` }}
                    />
                  </div>
                )}
                {t.detail && (
                  <p
                    className={`mt-0.5 text-[11px] break-words ${t.state === 'failed' ? 'text-red-300' : 'text-muted'}`}
                  >
                    {t.detail}
                  </p>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
