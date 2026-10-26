// Building blocks shared by the Settings sections.

export function Section({ title, hint, children }: { title: string; hint?: string; children: React.ReactNode }) {
  return (
    <section>
      <h2 className="text-base font-semibold">{title}</h2>
      {hint && <p className="mt-0.5 text-sm text-muted">{hint}</p>}
      <div className="mt-3">{children}</div>
    </section>
  );
}

export const button = 'rounded-md border border-edge px-3 py-1.5 text-sm hover:bg-edge disabled:opacity-40';
export const input = 'w-full rounded-md border border-edge bg-ink px-2 py-1.5 text-sm outline-none focus:border-accent';
