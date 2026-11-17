/** The Daily mark: a sun on the horizon. It inherits the current text color. */
export function DailyMark({ className }: { className?: string }) {
  return (
    <svg
      className={className}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2.25}
      strokeLinecap="round"
      aria-hidden="true"
    >
      <path d="M5 16a7 7 0 0 1 14 0" />
      <path d="M2 20h20" />
      <path d="M12 3v2" />
      <path d="m4.9 6.9 1.4 1.4" />
      <path d="m19.1 6.9-1.4 1.4" />
    </svg>
  );
}
