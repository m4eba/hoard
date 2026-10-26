import { useState } from 'react';
import type { Place } from '../api';
import { CardGrid } from './CardGrid';
import { useUploads } from './useUploads';

/**
 * Drop files anywhere on the library to upload them.
 *
 * Drag-and-drop only: the import page owns the deliberate path, with the file
 * picker and the URL field together. What lands here still shows the same
 * cards, so a drop never looks like nothing happened. Files land in `target`,
 * the folder being looked at, unless a card says otherwise.
 */
export function UploadDropzone({
  children,
  actions,
  heading,
  target = null,
}: {
  children: React.ReactNode;
  /** Shown at the left of the toolbar: where you are. */
  heading?: React.ReactNode;
  target?: Place | null;
  // Rendered at the right of the toolbar, so the view switcher shares this row
  // rather than adding a second one above the grid.
  actions?: React.ReactNode;
}) {
  const [over, setOver] = useState(false);
  const { items, accept, retag, clear, shared, setShared } = useUploads(target);
  const active = items.filter((i) => i.state !== 'done').length;

  return (
    <div
      onDragOver={(e) => {
        // Only files from outside; a tile being dragged onto a folder is a
        // move, not an upload.
        if (!e.dataTransfer.types.includes('Files')) return;
        e.preventDefault();
        setOver(true);
      }}
      onDragLeave={() => setOver(false)}
      onDrop={(e) => {
        if (!e.dataTransfer.types.includes('Files')) return;
        e.preventDefault();
        setOver(false);
        accept(e.dataTransfer.files);
      }}
      className="relative min-h-full"
    >
      {/* No upload button here: choosing files is what the import page is
          for. Dropping onto the library stays, because it costs nothing and
          is where you already are when you decide to add something. */}
      <div className="mb-3 flex flex-wrap items-center gap-3">
        {heading}
        {active > 0 && <span className="text-sm text-muted">Uploading {active}…</span>}
        {actions && <div className="ml-auto">{actions}</div>}
      </div>

      <CardGrid
        heading="Uploads"
        items={items}
        shared={shared}
        onRetag={retag}
        onShared={setShared}
        onClear={clear}
      />

      {children}

      {over && (
        <div className="pointer-events-none absolute inset-0 z-40 flex items-center justify-center rounded-lg border-2 border-dashed border-accent bg-ink/80">
          <p className="text-lg text-accent">Drop to upload</p>
        </div>
      )}
    </div>
  );
}
