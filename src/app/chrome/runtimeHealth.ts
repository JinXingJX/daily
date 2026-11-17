import type { BackendStatus } from "@/platform/desktop";
import { t } from "@/i18n";

export type StatusTone = "success" | "warning" | "danger" | "accent" | "neutral";

/** Health of the local runtime, as the dot on the workspace mark and its label. */
export function runtimeHealth(status: BackendStatus, running: boolean): { tone: StatusTone; label: string } {
  switch (status.state) {
    case "ready":
      return running ? { tone: "accent", label: t("Runtime busy") } : { tone: "success", label: t("Runtime ready") };
    case "failed":
      return { tone: "danger", label: t("Runtime offline") };
    case "restarting":
      return { tone: "warning", label: t("Runtime restarting") };
    default:
      return { tone: "warning", label: t("Connecting to runtime") };
  }
}
