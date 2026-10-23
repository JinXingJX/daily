import { AlertTriangle, RotateCcw } from "lucide-react";
import { cn } from "@/lib/utils";
import { type BackendStatus } from "@/platform/desktop";
import { t } from "@/i18n";

export function BackendBanner({ status, onRetry }: { status: BackendStatus; onRetry: () => void }) {
  return (
    <div role={status.state === "failed" ? "alert" : "status"} aria-live="polite" className={cn(
      "relative z-20 flex min-h-10 shrink-0 items-center gap-3 border-b px-4 text-xs",
      status.state === "failed"
        ? "border-[var(--danger-line)] bg-[var(--danger-soft)] text-[var(--danger)]"
        : "border-[var(--warning-line)] bg-[var(--warning-soft)] text-[var(--warning)]",
    )}>
      {status.state === "failed" ? <AlertTriangle className="size-4 shrink-0" /> : <RotateCcw className="size-4 shrink-0 animate-spin motion-reduce:animate-none" />}
      <span className="min-w-0 flex-1 truncate">{status.message}</span>
      {status.canRetry ? (
        <button className="rounded-[var(--radius-control)] border border-current px-2.5 py-1 font-semibold hover:bg-[var(--surface)] focus-visible:outline-none focus-visible:shadow-[var(--focus-ring)]" type="button" onClick={onRetry}>
          {t("Retry")}
        </button>
      ) : null}
    </div>
  );
}
