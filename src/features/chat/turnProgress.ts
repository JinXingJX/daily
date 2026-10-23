import { localizeHostText } from "@daily/plugin-sdk/hostText";
import { currentLocale } from "@/i18n";
import type { AgentEventView, AgentTurnView, RuntimeView } from "@/types";

export type RunState = "running" | "waiting" | "failed" | "completed" | "idle";

/** One unit of agent work. A tool call carries its result, so a call and its output read as one step. */
export type TurnStep = AgentEventView & { result: AgentEventView | null };

export type TurnProgress = {
  state: RunState;
  steps: TurnStep[];
  completed: number;
  /** 1-based position of the step in flight, or null when nothing is in flight. */
  currentStep: number | null;
  failedStep: TurnStep | null;
  /** Seconds since the first event of the turn; null before any event arrives. */
  elapsedSec: number | null;
};

const HIDDEN_KINDS = new Set<AgentEventView["kind"]>(["user_input", "assistant_text", "runtime"]);

/** Event views carry the runtime's English titles; show them in the UI language. */
function localizeView(view: AgentEventView): AgentEventView {
  const locale = currentLocale();
  return { ...view, title: localizeHostText(view.title, locale), content: localizeHostText(view.content, locale) };
}

export function turnSteps(views: AgentEventView[] | undefined): TurnStep[] {
  const visible = (views ?? []).filter((view) => !HIDDEN_KINDS.has(view.kind)).map(localizeView);
  const resultsByTool = new Map<string, AgentEventView>();
  const calledTools = new Set<string>();
  for (const view of visible) {
    if (!view.toolId) continue;
    if (view.kind === "tool_result") resultsByTool.set(view.toolId, view);
    if (view.kind === "tool_call") calledTools.add(view.toolId);
  }
  return visible
    .filter((view) => !(view.kind === "tool_result" && view.toolId && calledTools.has(view.toolId)))
    .map((view) => {
      const result = view.kind === "tool_call" && view.toolId ? resultsByTool.get(view.toolId) ?? null : null;
      if (!result) return { ...view, result: null };
      return {
        ...view,
        status: view.status === "failed" || result.status === "failed" ? "failed" : result.status,
        updatedAt: Math.max(view.updatedAt, result.updatedAt),
        result,
      };
    });
}

export function turnRunState(turn: AgentTurnView | null | undefined): RunState {
  switch (turn?.status) {
    case "queued":
    case "running":
      return "running";
    case "waiting_async_tool":
      return "waiting";
    case "failed":
      return "failed";
    case "completed":
      return "completed";
    default:
      return "idle";
  }
}

export function turnProgress(turn: AgentTurnView | null | undefined, nowSec: number): TurnProgress {
  const steps = turnSteps(turn?.eventViews);
  const completed = steps.filter((view) => view.status === "completed").length;
  const inFlight = steps.findIndex((view) => view.status === "in_progress" || view.status === "waiting");
  const state = turnRunState(turn);
  const started = turn?.eventViews.reduce<number | null>(
    (earliest, view) => (earliest == null || view.createdAt < earliest ? view.createdAt : earliest),
    null,
  );
  const finished = turn?.eventViews.reduce((latest, view) => Math.max(latest, view.updatedAt), 0) ?? 0;
  const end = state === "running" || state === "waiting" ? nowSec : finished;
  return {
    state,
    steps,
    completed,
    currentStep: inFlight === -1 ? (state === "running" ? completed + 1 : null) : inFlight + 1,
    failedStep: steps.find((view) => view.status === "failed") ?? null,
    elapsedSec: started == null ? null : Math.max(0, end - started),
  };
}

export function stepDurationSec(view: Pick<AgentEventView, "status" | "createdAt" | "updatedAt">, nowSec: number) {
  const end = view.status === "in_progress" || view.status === "waiting" ? nowSec : view.updatedAt;
  return Math.max(0, end - view.createdAt);
}

export function formatElapsed(totalSec: number) {
  const sec = Math.max(0, Math.floor(totalSec));
  const hours = Math.floor(sec / 3600);
  const minutes = Math.floor((sec % 3600) / 60);
  const seconds = sec % 60;
  const pad = (value: number) => String(value).padStart(2, "0");
  return hours ? `${hours}:${pad(minutes)}:${pad(seconds)}` : `${pad(minutes)}:${pad(seconds)}`;
}

export function formatStepDuration(sec: number) {
  if (sec < 1) return "<1s";
  if (sec < 60) return `${sec}s`;
  return formatElapsed(sec);
}

/**
 * Sessions other than the open one only expose whether they are running, so their
 * state is "running" or "idle"; the open session also reports a failed or waiting latest turn.
 */
export function sessionRunState(
  sessionId: number,
  runtime: RuntimeView | null,
  activeSessionId: number | null,
  activeLatestTurn: AgentTurnView | null | undefined,
): RunState {
  if (runtime?.runningSessionIds.includes(sessionId)) return "running";
  if (sessionId !== activeSessionId) return "idle";
  const state = turnRunState(activeLatestTurn);
  return state === "completed" ? "idle" : state;
}
