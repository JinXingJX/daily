import { CircleAlert, CirclePause, LoaderCircle, PanelLeft, PanelRight } from "lucide-react";
import type { ReactNode } from "react";
import { useNowSec } from "@/app/useNowSec";
import { formatElapsed, turnProgress, turnRunState } from "@/features/chat/turnProgress";
import type { AgentTurnView } from "@/types";
import { t } from "@/i18n";

export function WindowTitlebar({
  leadWidth,
  panelVisible,
  activeLabel,
  runTurn,
  actions,
  inspectorAvailable,
  inspectorOpen,
  onToggleInspector,
  onTogglePanel,
}: {
  /** Width of the segment above the sidebar, so the two columns line up. */
  leadWidth: number;
  panelVisible: boolean;
  activeLabel: string;
  /** Latest turn of the open session; its run state is shown when not completed. */
  runTurn: AgentTurnView | null;
  /** Controls placed before the inspector toggle. */
  actions?: ReactNode;
  inspectorAvailable: boolean;
  inspectorOpen: boolean;
  onToggleInspector: () => void;
  onTogglePanel: () => void;
}) {
  const toggleLabel = panelVisible ? t("Hide task panel") : t("Show task panel");
  return (
    <header className="window-titlebar app-drag relative z-30 flex shrink-0">
      <div className="titlebar-lead" data-panel={panelVisible ? "open" : "closed"} style={{ width: leadWidth }}>
        {panelVisible ? <span className="titlebar-brand">Daily</span> : <span />}
        <button
          className="titlebar-icon"
          type="button"
          aria-label={toggleLabel}
          aria-expanded={panelVisible}
          title={toggleLabel}
          onClick={onTogglePanel}
        >
          <PanelLeft className="size-4" />
        </button>
      </div>
      <div className="titlebar-main">
        <h1 className="titlebar-title">{activeLabel}</h1>
        {runTurn ? <RunPill turn={runTurn} /> : null}
        {actions}
        {inspectorAvailable ? (
          <button
            className="titlebar-icon"
            type="button"
            aria-label={inspectorOpen ? t("Close inspector") : t("Open inspector")}
            aria-pressed={inspectorOpen}
            title={`${inspectorOpen ? t("Close inspector") : t("Open inspector")} (⌘I)`}
            onClick={onToggleInspector}
          >
            <PanelRight className="size-4" />
          </button>
        ) : null}
      </div>
    </header>
  );
}

function RunPill({ turn }: { turn: AgentTurnView }) {
  const live = turnRunState(turn) === "running";
  const now = useNowSec(live);
  const run = turnProgress(turn, now);
  if (run.state === "running") {
    return (
      <div className="run-pill" data-state="running" role="status">
        <LoaderCircle className="size-3.5 animate-spin motion-reduce:animate-none" aria-hidden="true" />
        <span>{t("Running")}</span>
        {run.currentStep ? (
          <>
            <span className="run-pill-sep" aria-hidden="true">·</span>
            <span>{t("Step {step}", { step: run.currentStep })}</span>
          </>
        ) : null}
        {run.elapsedSec != null ? (
          <span className="mono font-normal text-[var(--text-muted)]">{formatElapsed(run.elapsedSec)}</span>
        ) : null}
      </div>
    );
  }
  if (run.state === "waiting") {
    return (
      <div className="run-pill" data-state="waiting" role="status">
        <CirclePause className="size-3.5" aria-hidden="true" />
        {t("Waiting for plugin confirmation")}
      </div>
    );
  }
  if (run.state === "failed") {
    const failedAt = run.failedStep ? run.steps.indexOf(run.failedStep) + 1 : null;
    return (
      <div className="run-pill" data-state="failed" role="status">
        <CircleAlert className="size-3.5" aria-hidden="true" />
        {failedAt ? t("Failed at step {step}", { step: failedAt }) : t("Run failed")}
      </div>
    );
  }
  return null;
}
