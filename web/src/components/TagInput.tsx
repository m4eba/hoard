import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { suggestTags } from '../api';

/**
 * Chip-style tag input with typeahead.
 *
 * Enter or comma turns the text into a chip, Backspace on an empty field
 * removes the last one, and the arrows walk the suggestions. Suggestions come
 * from the tags this account already uses.
 */
export function TagInput({
  tags,
  onChange,
  placeholder = 'Add tags…',
  compact = false,
}: {
  tags: string[];
  onChange: (tags: string[]) => void;
  placeholder?: string;
  compact?: boolean;
}) {
  const [text, setText] = useState('');
  const [suggestions, setSuggestions] = useState<string[]>([]);
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(-1);
  const inputRef = useRef<HTMLInputElement>(null);
  const boxRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const q = text.trim().toLowerCase();
    if (q.length < 2) {
      setSuggestions([]);
      setOpen(false);
      return;
    }
    // Debounced and abortable: typing quickly should not leave a queue of
    // requests whose answers arrive out of order.
    const controller = new AbortController();
    const timer = setTimeout(() => {
      suggestTags(q, controller.signal)
        .then((found) => {
          const fresh = found.filter((s) => !tags.includes(s));
          setSuggestions(fresh);
          setOpen(fresh.length > 0);
          setActive(-1);
        })
        .catch(() => {});
    }, 150);
    return () => {
      controller.abort();
      clearTimeout(timer);
    };
  }, [text, tags]);

  useEffect(() => {
    const onDocClick = (e: MouseEvent) => {
      if (!boxRef.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', onDocClick);
    return () => document.removeEventListener('mousedown', onDocClick);
  }, []);

  function add(raw: string) {
    const tag = raw.trim().toLowerCase();
    if (tag && !tags.includes(tag)) onChange([...tags, tag]);
    setText('');
    setOpen(false);
    inputRef.current?.focus();
  }

  function onKeyDown(e: KeyboardEvent<HTMLInputElement>) {
    if (e.key === 'ArrowDown' && open) {
      e.preventDefault();
      setActive((i) => Math.min(i + 1, suggestions.length - 1));
    } else if (e.key === 'ArrowUp' && open) {
      e.preventDefault();
      setActive((i) => Math.max(i - 1, -1));
    } else if (e.key === 'Enter' || e.key === ',') {
      e.preventDefault();
      add(active >= 0 && suggestions[active] ? suggestions[active] : text);
    } else if (e.key === 'Backspace' && text === '' && tags.length > 0) {
      onChange(tags.slice(0, -1));
    } else if (e.key === 'Escape') {
      setOpen(false);
    }
  }

  return (
    <div ref={boxRef} className="relative">
      <div
        className={`flex flex-wrap items-center gap-1 rounded-md border border-edge bg-ink px-2 ${
          compact ? 'py-1' : 'py-1.5'
        }`}
      >
        {tags.map((tag) => (
          <button
            key={tag}
            onClick={() => onChange(tags.filter((t) => t !== tag))}
            className="group flex items-center gap-1 rounded-full bg-edge px-2 py-0.5 text-xs text-white/90 hover:bg-red-900/60"
            title="Remove"
          >
            {tag}
            <span className="text-muted group-hover:text-white">×</span>
          </button>
        ))}
        <input
          ref={inputRef}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={onKeyDown}
          placeholder={tags.length ? '' : placeholder}
          className="min-w-24 flex-1 bg-transparent py-0.5 text-sm outline-none placeholder:text-muted"
        />
      </div>

      {open && (
        <ul className="absolute z-30 mt-1 max-h-52 w-full overflow-auto rounded-md border border-edge bg-panel py-1 shadow-xl">
          {suggestions.map((s, i) => (
            <li key={s}>
              <button
                onMouseDown={(e) => {
                  // mousedown, not click: the input's blur would close the
                  // list before a click ever landed.
                  e.preventDefault();
                  add(s);
                }}
                onMouseEnter={() => setActive(i)}
                className={`block w-full px-3 py-1.5 text-left text-sm ${
                  i === active ? 'bg-edge text-white' : 'text-white/80'
                }`}
              >
                {s}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
