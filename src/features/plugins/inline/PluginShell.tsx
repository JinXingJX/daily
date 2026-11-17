import type { ReactNode } from "react";
import { AlertTriangle } from "lucide-react";
import { t } from "@/i18n";
import { Badge } from "@/components/ui/badge";

function pluginLabel(label: string) {
  if (label === "Chart") return t("Chart");
  if (label === "Weather") return t("Weather");
  if (label === "Plugin") return t("Plugin");
  return label;
}

export function PluginShell(props: {
  identifier: string;
  label: string;
  icon: ReactNode;
  partial?: boolean;
  source?: string;
  children: ReactNode;
}) {
  return (
    <section className="workbench-panel overflow-hidden" aria-busy={props.partial || undefined}>
      <header className="flex items-center justify-between gap-4 border-b border-[var(--line-soft)] px-4 py-3">
        <div className="flex items-center gap-2 text-sm font-medium text-[var(--text-strong)]">
          <span className="flex size-4 items-center justify-center text-[var(--text-soft)]">{props.icon}</span>
          {pluginLabel(props.label)}
        </div>
        <Badge variant="secondary">{props.partial ? t("Streaming") : t("Interactive")}</Badge>
      </header>
      <div className="space-y-4 p-4">{props.children}</div>
    </section>
  );
}

export function PluginFallback(props: { identifier: string; source: string; reason: string }) {
  return (
    <section className="workbench-panel overflow-hidden">
      <div className="flex items-start gap-3 px-4 py-4" role="alert">
        <AlertTriangle className="mt-0.5 size-4 shrink-0 text-[var(--danger)]" />
        <div className="space-y-2">
          <p className="text-sm font-medium text-[var(--text-strong)]">{t("Could not display this result")}</p>
          <p className="text-sm text-[var(--text-soft)]">{t("The original result is preserved below.")}</p>
        </div>
      </div>
      <details className="border-t border-[var(--line)] px-4 py-3">
        <summary className="min-h-8 cursor-pointer rounded-[var(--radius-control)] text-sm text-[var(--text-soft)]">{t("Original result")}</summary>
        <p className="mt-2 text-xs text-[var(--text-muted)]">{props.identifier}: {props.reason}</p>
        <pre className="mt-3 overflow-x-auto whitespace-pre-wrap break-words rounded-[var(--radius-control)] bg-[var(--surface)] p-3 font-mono text-xs leading-6 text-[var(--text)]">{props.source}</pre>
      </details>
    </section>
  );
}
