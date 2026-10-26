import type { ViewMode } from './viewMode';

/** Segmented control for the library layout, like a file manager's. */
export function ViewToggle({
  value,
  onChange,
}: {
  value: ViewMode;
  onChange: (next: ViewMode) => void;
}) {
  const options: { mode: ViewMode; label: string; icon: React.ReactNode }[] = [
    { mode: 'list', label: 'List', icon: <ListIcon /> },
    { mode: 'grid', label: 'Grid', icon: <GridIcon /> },
    { mode: 'large', label: 'Large grid', icon: <LargeIcon /> },
  ];

  return (
    <div className="flex overflow-hidden rounded-md border border-edge" role="group">
      {options.map(({ mode, label, icon }) => (
        <button
          key={mode}
          onClick={() => onChange(mode)}
          // aria-pressed rather than a visual cue alone: the buttons are
          // icon-only, so the state has to be readable without seeing them.
          aria-pressed={value === mode}
          aria-label={label}
          title={label}
          className={`px-2.5 py-1.5 transition ${
            value === mode ? 'bg-edge text-white' : 'text-muted hover:bg-edge/50 hover:text-white'
          }`}
        >
          {icon}
        </button>
      ))}
    </div>
  );
}

const svg = {
  width: 16,
  height: 16,
  viewBox: '0 0 16 16',
  fill: 'currentColor',
  'aria-hidden': true,
} as const;

const ListIcon = () => (
  <svg {...svg}>
    <rect x="1" y="2.5" width="4" height="3" rx="0.5" />
    <rect x="6.5" y="3.5" width="8.5" height="1" rx="0.5" />
    <rect x="1" y="6.5" width="4" height="3" rx="0.5" />
    <rect x="6.5" y="7.5" width="8.5" height="1" rx="0.5" />
    <rect x="1" y="10.5" width="4" height="3" rx="0.5" />
    <rect x="6.5" y="11.5" width="8.5" height="1" rx="0.5" />
  </svg>
);

const GridIcon = () => (
  <svg {...svg}>
    {[1, 6, 11].map((y) =>
      [1, 6, 11].map((x) => <rect key={`${x}-${y}`} x={x} y={y} width="4" height="4" rx="0.5" />),
    )}
  </svg>
);

const LargeIcon = () => (
  <svg {...svg}>
    <rect x="1" y="1" width="6.5" height="6.5" rx="0.8" />
    <rect x="8.5" y="1" width="6.5" height="6.5" rx="0.8" />
    <rect x="1" y="8.5" width="6.5" height="6.5" rx="0.8" />
    <rect x="8.5" y="8.5" width="6.5" height="6.5" rx="0.8" />
  </svg>
);
