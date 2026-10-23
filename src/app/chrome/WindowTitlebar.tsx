import { CircleAlert, CirclePause, LoaderCircle, PanelLeft, PanelRight, Search } from "lucide-react";
import { useNowSec } from "@/app/useNowSec";
import { formatElapsed, turnProgress, turnRunState } from "@/features/chat/turnProgress";
import type { AgentTurnView } from "@/types";
import { t } from "@/i18n";

export function WindowTitlebar({
  leadWidth,
  panelVisible,
  panelToggleAvailable,
  contextLabel,
  activeLabel,
  runTurn,
  inspectorAvailable,
  inspectorOpen,
  onCommandOpen,
  onToggleInspector,
  onTogglePanel,
}: {
  /** Width of the segment above the rail and task panel, so the two columns line up. */
  leadWidth: number;
  panelVisible: boolean;
  panelToggleAvailable: boolean;
  contextLabel: string | null;
  activeLabel: string;
  /** Latest turn of the open session; its run state is shown when not completed. */
  runTurn: AgentTurnView | null;
  inspectorAvailable: boolean;
  inspectorOpen: boolean;
  onCommandOpen: () => void;
  onToggleInspector: () => void;
  onTogglePanel: () => void;
}) {
  return (
    <header className="window-titlebar app-drag relative z-30 flex shrink-0">
      <div className="titlebar-lead" style={{ width: leadWidth }}>
        {panelToggleAvailable ? (
          <button
            className="titlebar-icon"
            type="button"
            aria-label={panelVisible ? t("Hide task panel") : t("Show task panel")}
            aria-pressed={!panelVisible}
            title={panelVisible ? t("Hide task panel") : t("Show task panel")}
            onClick={onTogglePanel}
          >
            <PanelLeft className="size-4" />
          </button>
        ) : null}
      </div>
      <div className="titlebar-main">
        <nav aria-label={t("Location")} className="flex min-w-0 flex-1 items-center gap-2">
          {contextLabel ? (
            <>
              <span className="shrink-0 text-[var(--text-muted)]">{contextLabel}</span>
              <span className="shrink-0 text-[var(--line-active)]" aria-hidden="true">/</span>
            </>
          ) : null}
          <span className="truncate font-medium text-[var(--text-strong)]">{activeLabel}</span>
        </nav>
        {runTurn ? <RunPill turn={runTurn} /> : null}
        <button className="titlebar-command" type="button" title={t("Search tasks and commands")} onClick={onCommandOpen}>
          <Search className="size-3.5" />
          <span className="hidden md:inline">{t("Search tasks and commands")}</span>
          <kbd className="kbd">⌘K</kbd>
        </button>
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
        {run.elapsedSec != null ? <span className="mono font-normal">{formatElapsed(run.elapsedSec)}</span> : null}
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
