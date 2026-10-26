import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { suggestTags } from '../api';

/**
 * The library search box, with tag completion.
 *
 * Completion applies to the token being typed rather than the whole query, so
 * suggestions still work halfway through `folder:Holiday bea…`. Accepting one
 * rewrites that token as `tag:<name>` — the strict form — because a bare word
 * is matched fuzzily and someone who picked a tag from a list meant that exact
 * tag.
 */
export function SearchBox({
  value,
  onChange,
  onSubmit,
}: {
  value: string;
  onChange: (next: string) => void;
  onSubmit: (query: string) => void;
}) {
  const [suggestions, setSuggestions] = useState<string[]>([]);
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(-1);
  const inputRef = useRef<HTMLInputElement>(null);
  const boxRef = useRef<HTMLDivElement>(null);
  // Set while a suggestion is being applied, so the resulting value change does
  // not immediately reopen the list on the token we just completed.
  const justAccepted = useRef(false);

  const token = lastToken(value);

  useEffect(() => {
    if (justAccepted.current) {
      justAccepted.current = false;
      return;
    }
    const prefix = token.text.toLowerCase();
    if (prefix.length < 2) {
      setSuggestions([]);
      setOpen(false);
      return;
    }
    const controller = new AbortController();
    const timer = setTimeout(() => {
      suggestTags(prefix, controller.signal)
        .then((found) => {
          setSuggestions(found);
          // Only when the box actually has focus. A query can arrive from
          // elsewhere — clicking a tag on a card navigates here with the search
          // filled in — and a dropdown nobody opened has no blur coming to
          // close it, so it sits over the results forever.
          setOpen(found.length > 0 && document.activeElement === inputRef.current);
          setActive(-1);
        })
        .catch(() => {});
    }, 150);
    return () => {
      controller.abort();
      clearTimeout(timer);
    };
  }, [token.text]);

  // Blur is not enough on its own: a click that lands on something which does
  // not take focus never blurs the input, and the list would stay up.
  useEffect(() => {
    if (!open) return;
    const onDown = (e: Event) => {
      if (!boxRef.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('pointerdown', onDown);
    return () => document.removeEventListener('pointerdown', onDown);
  }, [open]);

  function accept(tag: string) {
    justAccepted.current = true;
    // A trailing space so the next word starts a fresh token.
    onChange(`${value.slice(0, token.start)}${token.negated ? '-' : ''}tag:${tag} `);
    setOpen(false);
    setSuggestions([]);
    inputRef.current?.focus();
  }

  function clear() {
    setOpen(false);
    setSuggestions([]);
    onChange('');
    // Submitting the empty query is what actually leaves the results page;
    // just blanking the box would leave the old results on screen.
    onSubmit('');
    inputRef.current?.focus();
  }

  function onKeyDown(e: KeyboardEvent<HTMLInputElement>) {
    if (open && e.key === 'ArrowDown') {
      e.preventDefault();
      setActive((i) => Math.min(i + 1, suggestions.length - 1));
      return;
    }
    if (open && e.key === 'ArrowUp') {
      e.preventDefault();
      setActive((i) => Math.max(i - 1, -1));
      return;
    }
    if (e.key === 'Enter') {
      // Enter takes the highlighted suggestion if there is one, and otherwise
      // runs the query as typed — so the box never traps you in its dropdown.
      if (open && active >= 0 && suggestions[active]) {
        e.preventDefault();
        accept(suggestions[active]);
        return;
      }
      setOpen(false);
      onSubmit(value);
      return;
    }
    if (e.key === 'Escape') {
      // Escape backs out one step at a time: first the suggestions, then the
      // query itself. Clearing while a dropdown is open would throw away more
      // than the user asked to dismiss.
      if (open) setOpen(false);
      else if (value) clear();
    }
    if (e.key === 'Tab' && open && suggestions.length > 0) {
      e.preventDefault();
      accept(suggestions[active >= 0 ? active : 0]);
    }
  }

  return (
    <div ref={boxRef} className="relative w-full">
      <input
        ref={inputRef}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={onKeyDown}
        onBlur={() => setTimeout(() => setOpen(false), 120)}
        placeholder="Search titles, tags, file names…  (tag:beach -tag:blurry folder:Holiday kind:video)"
        // Extra padding on the right so long queries do not run under the
        // clear button.
        className="w-full rounded-md border border-edge bg-ink py-1.5 pr-8 pl-3 text-sm outline-none focus:border-accent"
      />

      {value && (
        <button
          type="button"
          // mousedown, not click: the input's blur handler closes the
          // suggestions first, and on a short field that can move focus before
          // the click lands.
          onMouseDown={(e) => {
            e.preventDefault();
            clear();
          }}
          aria-label="Clear search"
          title="Clear search"
          className="absolute top-1/2 right-1.5 -translate-y-1/2 rounded-full p-1 text-muted transition hover:bg-edge hover:text-white"
        >
          <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor" aria-hidden>
            <path d="M19 6.41 17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z" />
          </svg>
        </button>
      )}

      {open && (
        <ul className="absolute z-40 mt-1 max-h-64 w-full overflow-auto rounded-md border border-edge bg-panel py-1 shadow-xl">
          {suggestions.map((tag, i) => (
            <li key={tag}>
              <button
                type="button"
                onMouseDown={(e) => {
                  // mousedown, not click: the input's blur closes the list
                  // before a click would ever land.
                  e.preventDefault();
                  accept(tag);
                }}
                onMouseEnter={() => setActive(i)}
                className={`flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm ${
                  i === active ? 'bg-edge text-white' : 'text-white/80'
                }`}
              >
                <span className="text-muted">{token.negated ? '-tag:' : 'tag:'}</span>
                {tag}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/**
 * The token the caret is in: where it starts, its searchable text, and whether
 * it was negated.
 *
 * A `tag:` prefix is stripped so `tag:bea` completes on "bea" rather than on
 * the literal "tag:bea", which would match nothing.
 */
function lastToken(value: string): { start: number; text: string; negated: boolean } {
  const start = value.lastIndexOf(' ') + 1;
  let text = value.slice(start);
  let negated = false;
  if (text.startsWith('-')) {
    negated = true;
    text = text.slice(1);
  }
  for (const prefix of ['tag:', 'folder:', 'in:', 'root:', 'kind:', 'type:']) {
    if (text.startsWith(prefix)) {
      // Only tags are completed; the others are left alone.
      return { start, text: prefix === 'tag:' ? text.slice(prefix.length) : '', negated };
    }
  }
  return { start, text, negated };
}
