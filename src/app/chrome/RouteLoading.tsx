
export function RouteLoading({ label }: { label: string }) {
  return (
    <div className="grid min-h-0 flex-1 place-items-center text-sm text-[var(--text-muted)]" role="status">
      {label}
    </div>
  );
}
