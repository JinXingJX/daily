import type { BackendStatus } from "@/platform/desktop";
import { currentLocale, t } from "@/i18n";
import { localizeHostText } from "@daily/plugin-sdk/hostText";
import { TaskCenter } from "./TaskCenter";

export type StatusTone = "success" | "warning" | "danger" | "accent" | "neutral";

/** Always-visible runtime facts: sidecar health, model auth, and plugin count. */
export function StatusBar({
  backendStatus,
  runtimeStatus,
  runtimeIsRunning,
  authLabel,
  authTone,
  enabledPluginCount,
}: {
  backendStatus: BackendStatus;
  runtimeStatus: string | null;
  runtimeIsRunning: boolean;
  authLabel: string;
  authTone: StatusTone;
  enabledPluginCount: number;
}) {
  const backendTone: StatusTone =
    backendStatus.state === "ready" ? (runtimeIsRunning ? "accent" : "success") : backendStatus.state === "failed" ? "danger" : "warning";
  const backendLabel =
    backendStatus.state === "ready"
      ? runtimeIsRunning
        ? t("Runtime busy")
        : t("Runtime ready")
      : backendStatus.state === "failed"
        ? t("Runtime offline")
        : backendStatus.state === "restarting"
          ? t("Runtime restarting")
          : t("Connecting to runtime");

  return (
    <footer className="status-bar" aria-label={t("Status bar")}>
      <span className="status-bar-item" role="status" aria-live="polite" title={backendStatus.message}>
        <span className="status-dot" data-tone={backendTone} aria-hidden="true" />
        {backendLabel}
      </span>
      {runtimeStatus && backendStatus.state === "ready" ? (
        <span className="mono min-w-0 truncate text-[var(--text-muted)]" title={runtimeStatus}>
          {localizeHostText(runtimeStatus, currentLocale())}
        </span>
      ) : null}
      <span className="flex-1" />
      <TaskCenter ready={backendStatus.state === "ready"} />
      <span className="status-bar-item">
        <span className="status-dot" data-tone={authTone} aria-hidden="true" />
        {authLabel}
      </span>
      <span>{enabledPluginCount ? t("{count} plugins enabled", { count: enabledPluginCount }) : t("No plugins enabled")}</span>
    </footer>
  );
}
