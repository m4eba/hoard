import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import type { ListItem } from '../api';
import { useFifflyLink } from '../fiffly';

/**
 * The "⋯" button on a tile or row and the menu it opens. The menu is portalled
 * to the page, since tiles clip their contents to the rounded corners.
 */
export function MediaMenu({ item, className = '' }: { item: ListItem; className?: string }) {
  const [at, setAt] = useState<{ top: number; right: number } | null>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const fiffly = useFifflyLink(item.id, at !== null);

  useEffect(() => {
    if (!at) return;
    const close = (e: Event) => {
      if (e instanceof KeyboardEvent ? e.key === 'Escape' : !menuRef.current?.contains(e.target as Node)) setAt(null);
    };
    // Scrolling would leave the menu floating away from its tile.
    const scrolled = () => setAt(null);
    window.addEventListener('pointerdown', close, true);
    window.addEventListener('keydown', close);
    window.addEventListener('scroll', scrolled, true);
    return () => {
      window.removeEventListener('pointerdown', close, true);
      window.removeEventListener('keydown', close);
      window.removeEventListener('scroll', scrolled, true);
    };
  }, [at]);

  const toggle = (e: React.MouseEvent) => {
    // Inside a tile, which opens the viewer on click.
    e.preventDefault();
    e.stopPropagation();
    if (at) return setAt(null);
    const r = buttonRef.current!.getBoundingClientRect();
    setAt({ top: r.bottom + 4, right: window.innerWidth - r.right });
  };

  const entry = 'block w-full px-3 py-1.5 text-left text-sm hover:bg-edge';
  return (
    <>
      <button
        ref={buttonRef}
        type="button"
        onClick={toggle}
        // pointerdown is where the outside-click check runs; this button
        // toggles on its own.
        onPointerDown={(e) => e.stopPropagation()}
        aria-label="More actions"
        aria-haspopup="menu"
        aria-expanded={at !== null}
        title="More actions"
        className={`flex size-[22px] items-center justify-center rounded bg-black/70 text-white/80 hover:bg-black/90 hover:text-white ${
          at ? 'opacity-100' : ''
        } ${className}`}
      >
        <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor" aria-hidden>
          <path d="M6 10a2 2 0 1 0 0 4 2 2 0 0 0 0-4zm6 0a2 2 0 1 0 0 4 2 2 0 0 0 0-4zm6 0a2 2 0 1 0 0 4 2 2 0 0 0 0-4z" />
        </svg>
      </button>
      {at &&
        createPortal(
          <div
            ref={menuRef}
            role="menu"
            style={{ top: at.top, right: at.right }}
            className="fixed z-50 min-w-44 overflow-hidden rounded-md border border-edge bg-panel py-1 shadow-xl"
            onClick={() => setAt(null)}
          >
            <a
              role="menuitem"
              href={fiffly}
              target="_blank"
              rel="noreferrer"
              aria-disabled={!fiffly}
              className={`${entry} ${fiffly ? '' : 'pointer-events-none text-muted'}`}
            >
              Open in fiffly
            </a>
            <a role="menuitem" href={`/f/${item.id}`} download={item.filename} className={entry}>
              Download
            </a>
          </div>,
          document.body,
        )}
    </>
  );
}
