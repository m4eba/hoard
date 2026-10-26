import { useCallback, useEffect, useState } from 'react';

/**
 * How the library is laid out. A file explorer remembers this, so it is stored
 * per browser rather than living in a URL or resetting on every visit.
 */
export type ViewMode = 'list' | 'grid' | 'large';

const KEY = 'hoard.view';
const MODES: ViewMode[] = ['list', 'grid', 'large'];

function read(): ViewMode {
  try {
    const stored = localStorage.getItem(KEY) as ViewMode | null;
    if (stored && MODES.includes(stored)) return stored;
  } catch {
    // Private-mode browsers can throw; the default is fine.
  }
  return 'grid';
}

export function useViewMode(): [ViewMode, (next: ViewMode) => void] {
  const [mode, setMode] = useState<ViewMode>(read);

  // Keep other tabs in step: switching view in one and finding the other
  // disagreeing is the kind of small wrongness that erodes trust in a UI.
  useEffect(() => {
    const onStorage = (e: StorageEvent) => {
      if (e.key === KEY) setMode(read());
    };
    window.addEventListener('storage', onStorage);
    return () => window.removeEventListener('storage', onStorage);
  }, []);

  const update = useCallback((next: ViewMode) => {
    setMode(next);
    try {
      localStorage.setItem(KEY, next);
    } catch {
      // Not being able to remember the choice is not a reason to refuse it.
    }
  }, []);

  return [mode, update];
}
