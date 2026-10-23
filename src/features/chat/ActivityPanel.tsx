import { useNowSec } from "@/app/useNowSec";
import { Button } from "@/components/ui/button";
import {
  formatElapsed,
  formatStepDuration,
  stepDurationSec,
  turnProgress
} from "@/features/chat/turnProgress";
import { cn } from "@/lib/utils";
import type { AgentAppConfig, AgentTurnView } from "@/types";
import {
  Bot,
  Plug,
  X
} from "lucide-react";
import { useState } from "react";
import { hookAgentNames, runStateLabel, runStateTone } from "./chatHelpers";
import { SettingsTarget } from "./chatTypes";
import { stepDetail, stepKindLabel } from "./MessageFeed";
import { t } from "@/i18n";

export type InspectorTab = "run" | "context" | "hooks";

export function ActivityPanel({
  latestTurn,
  agent,
  agents,
  modelLabel,
  onClose,
  onOpenSettings,
}: {
  latestTurn: AgentTurnView | null;
  agent: AgentAppConfig | null;
  agents: AgentAppConfig[];
  modelLabel: string | null;
  onClose: () => void;
  onOpenSettings: (target: SettingsTarget) => void;
}) {
  const [tab, setTab] = useState<InspectorTab>("run");
  const live = latestTurn?.status === "running" || latestTurn?.status === "queued";
  const now = useNowSec(live);
  const progress = turnProgress(latestTurn, now);
  const tabs: Array<{ id: InspectorTab; label: string }> = [
    { id: "run", label: t("Run") },
    { id: "context", label: t("Context") },
    { id: "hooks", label: t("Hooks") },
  ];

  return (
    <aside className="activity-panel" aria-label={t("Inspector")}>
      <div className="tab-list items-center px-2.5 pt-1.5">
        <div role="tablist" aria-label={t("Inspector views")} className="flex flex-1 gap-0.5">
          {tabs.map((item) => (
            <button
              key={item.id}
              type="button"
              role="tab"
              id={`inspector-tab-${item.id}`}
              aria-selected={tab === item.id}
              aria-controls={`inspector-panel-${item.id}`}
              className="tab"
              onClick={() => setTab(item.id)}
            >
              {item.label}
            </button>
          ))}
        </div>
        <button className="titlebar-icon mb-1" type="button" aria-label={t("Close inspector")} title={`${t("Close inspector")} (⌘I)`} onClick={onClose}>
          <X className="size-4" />
        </button>
      </div>

      <div
        role="tabpanel"
        id={`inspector-panel-${tab}`}
        aria-labelledby={`inspector-tab-${tab}`}
        className="flex min-h-0 flex-1 flex-col gap-5 overflow-y-auto p-4"
      >
        {tab === "run" ? (
          latestTurn ? (
            <>
              <div className="stat-grid grid-cols-2">
                <div>
                  <div className="stat-label">{t("Status")}</div>
                  <div className={cn("stat-value", runStateTone(progress.state))}>{runStateLabel(progress.state)}</div>
                </div>
                <div>
                  <div className="stat-label">{live ? t("Elapsed") : t("Duration")}</div>
                  <div className="stat-value mono">{progress.elapsedSec != null ? formatElapsed(progress.elapsedSec) : "—"}</div>
                </div>
                <div>
                  <div className="stat-label">{t("Agent")}</div>
                  <div className="stat-value">{latestTurn.agentName ?? t("Not bound")}</div>
                </div>
                <div>
                  <div className="stat-label">{t("Steps")}</div>
                  <div className="stat-value mono">
                    {progress.completed} / {progress.steps.length}
                  </div>
                </div>
              </div>

              <section className="flex flex-col gap-2">
                <h3 className="panel-label">{t("Steps")}</h3>
                {progress.steps.length ? (
                  <ol className="timeline">
                    {progress.steps.map((step) => (
                      <li key={step.id} className="timeline-item">
                        <span className="timeline-dot" data-status={step.status} aria-hidden="true" />
                        <span className="flex min-w-0 flex-1 flex-col">
                          <span
                            className={cn(
                              "truncate",
                              step.status === "in_progress" || step.status === "waiting"
                                ? "font-medium text-[var(--accent-text)]"
                                : step.status === "failed"
                                  ? "font-medium text-[var(--danger)]"
                                  : "text-[var(--text-strong)]",
                            )}
                            title={stepDetail(step)}
                          >
                            {step.toolName ? `${stepKindLabel(step)} · ${step.toolName}` : stepKindLabel(step)}
                          </span>
                          <span className="text-[11px] text-[var(--text-muted)]">
                            {formatStepDuration(stepDurationSec(step, now))}
                          </span>
                        </span>
                      </li>
                    ))}
                  </ol>
                ) : (
                  <p className="m-0 text-[12px] text-[var(--text-muted)]">{t("Tool calls and run events appear here.")}</p>
                )}
              </section>

              <section className="flex flex-col gap-2">
                <h3 className="panel-label">{t("Instruction")}</h3>
                <p className="line-clamp-3 m-0 whitespace-pre-wrap break-words rounded-[var(--radius-panel)] bg-[var(--surface-muted)] px-3 py-2.5 text-[12px] leading-5 text-[var(--text-soft)]">
                  {latestTurn.prompt}
                </p>
              </section>
            </>
          ) : (
            <EmptyPanel title={t("Nothing has run yet")} detail={t("Send an instruction to follow the agent's progress here.")} />
          )
        ) : null}

        {tab === "context" ? (
          agent ? (
            <>
              <InspectorList title={t("Model")} items={[modelLabel ?? agent.model]} />
              <InspectorList
                title={t("Context folders")}
                mono
                items={agent.contextRoots}
                empty={t("Unrestricted: the agent can read the whole workspace.")}
              />
              <InspectorList title={t("Callable plugins")} mono items={agent.mountedPluginIds} empty={t("No plugins mounted.")} />
              <Button variant="secondary" size="sm" className="self-start text-xs" onClick={() => onOpenSettings("agents")}>
                {t("Edit {name}", { name: agent.name })}
              </Button>
            </>
          ) : (
            <EmptyPanel title={t("No agent bound")} detail={t("Pick an agent next to the input to see what it can access.")} />
          )
        ) : null}

        {tab === "hooks" ? (
          agent ? (
            <>
              <HookSummary title={t("Before")} config={agent.onStartHook} agents={agents} />
              <HookSummary title={t("After")} config={agent.onFinishHook} agents={agents} />
              <Button variant="secondary" size="sm" className="self-start text-xs" onClick={() => onOpenSettings("agents")}>
                {t("Edit hooks")}
              </Button>
            </>
          ) : (
            <EmptyPanel title={t("No agent bound")} detail={t("Hooks are part of an agent's settings. Bind an agent to see them here.")} />
          )
        ) : null}
      </div>
    </aside>
  );
}

export function InspectorList({ title, items, mono, empty }: { title: string; items: string[]; mono?: boolean; empty?: string }) {
  return (
    <section className="flex flex-col gap-2">
      <h3 className="panel-label">{title}</h3>
      {items.length ? (
        <ul className="m-0 flex list-none flex-col gap-1 p-0">
          {items.map((item) => (
            <li
              key={item}
              className={cn(
                "truncate rounded-[var(--radius-control)] border border-[var(--line)] bg-[var(--surface-strong)] px-2.5 py-1.5 text-[12px] text-[var(--text)]",
                mono && "mono",
              )}
              title={item}
            >
              {item}
            </li>
          ))}
        </ul>
      ) : (
        <p className="m-0 text-[12px] text-[var(--text-muted)]">{empty}</p>
      )}
    </section>
  );
}

export function HookSummary({
  title,
  config,
  agents,
}: {
  title: string;
  config: AgentAppConfig["onStartHook"];
  agents: AgentAppConfig[];
}) {
  const names = hookAgentNames(config.agentAppIds, agents);
  const enabled = Boolean(config.mode && names.length);
  return (
    <section className="hook-card" data-active={enabled}>
      <div className="flex items-center justify-between">
        <span className="font-medium text-[var(--text-strong)]">{title}</span>
        <span className="text-[11px] text-[var(--text-muted)]">
          {enabled ? (config.mode === "parallel" ? t("Parallel") : t("Serial")) : t("Off")}
        </span>
      </div>
      {enabled ? (
        <>
          <div className="flex flex-wrap gap-1.5">
            {names.map((name) => (
              <span key={name} className="meta-chip">
                <Bot className="size-3 shrink-0" />
                <span>{name}</span>
              </span>
            ))}
          </div>
          {config.mode === "serial" && config.appendPreviousOutput ? (
            <span className="text-[11px] text-[var(--text-muted)]">{t("Includes this turn's output")}</span>
          ) : null}
        </>
      ) : null}
    </section>
  );
}

export function EmptyPanel({ title, detail }: { title: string; detail: string }) {
  return (
    <div className="px-2 py-10 text-center">
      <Plug className="mx-auto size-5 text-[var(--text-muted)]" aria-hidden="true" />
      <h3 className="mt-3 text-[13px] font-medium text-[var(--text-strong)]">{title}</h3>
      <p className="mt-1 text-[12px] leading-5 text-[var(--text-muted)]">{detail}</p>
    </div>
  );
}
