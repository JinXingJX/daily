import { normalizeHookConfig, type AgentFormState } from "@/features/agents/agentHooks";
import type { AgentAppConfig, AgentHookConfig } from "@/types";
import { t } from "@/i18n";

function sameList<T>(left: readonly T[], right: readonly T[]) {
  return left.length === right.length && left.every((value, index) => value === right[index]);
}

function sameSet(left: readonly string[], right: readonly string[]) {
  return sameList([...left].sort(), [...right].sort());
}

function sameHook(left: AgentHookConfig, right: AgentHookConfig) {
  const a = normalizeHookConfig(left);
  const b = normalizeHookConfig(right);
  return a.mode === b.mode && a.appendPreviousOutput === b.appendPreviousOutput && sameList(a.agentAppIds, b.agentAppIds);
}

/** Labels of the fields where the draft differs from the saved agent, in form order. */
export function changedAgentFields(draft: AgentFormState | null, saved: AgentAppConfig | null | undefined): string[] {
  if (!draft || !saved) return [];
  const changes: Array<[string, boolean]> = [
    [t("Name"), draft.name !== saved.name],
    [t("Provider"), draft.authConfigId !== saved.authConfigId],
    [t("Model"), draft.model !== saved.model],
    [t("Temperature"), draft.temperature !== saved.temperature],
    [t("System prompt"), draft.systemPrompt !== saved.systemPrompt],
    [t("Context folders"), !sameSet(draft.contextRoots, saved.contextRoots)],
    [t("Callable plugins"), !sameSet(draft.mountedPluginIds, saved.mountedPluginIds)],
    [t("Start hooks"), !sameHook(draft.onStartHook, saved.onStartHook)],
    [t("Finish hooks"), !sameHook(draft.onFinishHook, saved.onFinishHook)],
  ];
  return changes.filter(([, changed]) => changed).map(([label]) => label);
}
