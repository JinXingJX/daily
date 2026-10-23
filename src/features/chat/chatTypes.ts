import type { AgentEventView, DialogueMessage } from "@/types";
import { t } from "@/i18n";

export type RenderMessage = {
  id: string;
  messageId?: number;
  turnId?: number | null;
  role: DialogueMessage["role"];
  content: string;
  partial?: boolean;
  eventViews?: AgentEventView[];
};

export type ComposerMode = "plan" | "build" | "review";

export const COMPOSER_MODES: Array<{ id: ComposerMode; label: string; prompt: string }> = [
  // Getters so labels and prompts follow the current language.
  {
    id: "plan",
    get label() { return t("Plan"); },
    get prompt() { return t("Start with a plan; if anything is risky, wait for my confirmation before acting."); },
  },
  {
    id: "build",
    get label() { return t("Build"); },
    get prompt() { return t("Implement this change end to end, keep the scope tight, and verify it."); },
  },
  {
    id: "review",
    get label() { return t("Review"); },
    get prompt() { return t("Review the recent changes for defects, regressions and missing interaction states."); },
  },
];

export type ChatReadiness = {
  modelLabel: string;
  modelReady: boolean;
  agentCount: number;
  pluginNames: string[];
};

export type SettingsTarget = "auth" | "agents" | "plugins";
