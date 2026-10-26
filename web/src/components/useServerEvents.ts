import { useEffect, useRef } from 'react';
import type { ServerEvent } from '../api';

/**
 * One EventSource for the whole tab, shared by every component that listens.
 *
 * Browsers cap concurrent connections per host, and an SSE stream holds one
 * open for good — a stream per component would starve image loading on a
 * busy page.
 */
const listeners = new Set<(e: ServerEvent) => void>();
let source: EventSource | null = null;

function open() {
  if (source) return;
  source = new EventSource('/api/events');
  source.onmessage = (e) => {
    let event: ServerEvent;
    try {
      event = JSON.parse(e.data) as ServerEvent;
    } catch {
      // A frame we cannot parse is not worth tearing the stream down for.
      return;
    }
    listeners.forEach((l) => l(event));
  };
  // EventSource reconnects on its own, so an error needs no handling beyond
  // not crashing.
}

function close() {
  source?.close();
  source = null;
}

export function useServerEvents(onEvent: (e: ServerEvent) => void, enabled = true) {
  // A ref, so a new callback identity on every render does not resubscribe.
  const handler = useRef(onEvent);
  handler.current = onEvent;

  useEffect(() => {
    if (!enabled) return;
    const listener = (e: ServerEvent) => handler.current(e);
    listeners.add(listener);
    open();
    return () => {
      listeners.delete(listener);
      if (listeners.size === 0) close();
    };
  }, [enabled]);
}
