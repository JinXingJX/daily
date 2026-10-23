import {
  type TurnProgress
} from "@/features/chat/turnProgress";
import type { AgentAppConfig } from "@/types";
import { t } from "@/i18n";

export function hookAgentNames(ids: number[] | undefined, agents: AgentAppConfig[]) {
  return (ids ?? []).map((id) => agents.find((agent) => agent.id === id)?.name).filter((name): name is string => Boolean(name));
}

export function runStateLabel(state: TurnProgress["state"]) {
  switch (state) {
    case "running":
      return t("Running");
    case "waiting":
      return t("Waiting for confirmation");
    case "failed":
      return t("Failed");
    case "completed":
      return t("Done");
    default:
      return t("Ready");
  }
}

export function runStateTone(state: TurnProgress["state"]) {
  switch (state) {
    case "running":
      return "!text-[var(--accent-text)]";
    case "waiting":
      return "!text-[var(--warning)]";
    case "failed":
      return "!text-[var(--danger)]";
    default:
      return "";
  }
}
