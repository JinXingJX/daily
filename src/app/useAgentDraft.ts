import { useEffect, useMemo, useState } from "react";
import {
  type AgentFormState,
  agentToFormState,
  emptyHookConfig,
  normalizeHookConfig,
} from "@/features/agents/agentHooks";
import { coreRoute, type AppRoute } from "@/useAppStore";
import {
  DEFAULT_CODEX_AUTH_CONFIG_ID,
  DEFAULT_CODEX_MODEL,
} from "@/app/modelDefaults";
import type { AgentAppConfig, AppSnapshot, AuthConfigView, PluginManifest, SettingsView } from "@/types";
import type { DesktopCommand } from "@/platform/desktop";
import { t } from "@/i18n";

type RunSnapshotCommand = <T,>(command: DesktopCommand, args?: T) => Promise<AppSnapshot>;

export function useAgentDraft({
  agentApps,
  pluginManifests,
  settings,
  selectedAgentId,
  selectedAuthConfig,
  runSnapshotCommand,
  setActiveRoute,
  setSelectedAgentId,
}: {
  agentApps: AgentAppConfig[];
  pluginManifests: PluginManifest[];
  settings: SettingsView | null;
  selectedAgentId: number | null;
  selectedAuthConfig: AuthConfigView | null;
  runSnapshotCommand: RunSnapshotCommand;
  setActiveRoute: (route: AppRoute) => void;
  setSelectedAgentId: (id: number | null) => void;
}) {
  const [agentDraft, setAgentDraft] = useState<AgentFormState | null>(null);
  const selectedAgent = useMemo(
    () => agentApps.find((agent) => agent.id === selectedAgentId) ?? null,
    [agentApps, selectedAgentId],
  );
  const agentSyncKey = useMemo(
    () => agentApps.map((agent) => JSON.stringify(agent)).join("|"),
    [agentApps],
  );

  useEffect(() => {
    if (!selectedAgent) {
      setAgentDraft(null);
      return;
    }
    setAgentDraft(
      agentToFormState(
        selectedAgent,
        settings?.effective.model.authConfigId || DEFAULT_CODEX_AUTH_CONFIG_ID,
      ),
    );
  }, [agentSyncKey, selectedAgent, settings?.effective.model.authConfigId]);

  async function handleCreateAgent() {
    const authConfig = selectedAuthConfig ?? settings?.effective.model.authConfigs[0] ?? null;
    const nextSnapshot = await runSnapshotCommand("create_agent_app", {
      input: {
        name: "New Agent",
        systemPrompt: settings?.effective.agent.systemPrompt ?? "",
        providerKind: authConfig?.providerKind ?? "codex_login",
        authConfigId: authConfig?.id ?? settings?.effective.model.authConfigId ?? DEFAULT_CODEX_AUTH_CONFIG_ID,
        model: authConfig?.defaultModel ?? settings?.effective.model.model ?? DEFAULT_CODEX_MODEL,
        temperature: settings?.effective.model.temperature ?? 0.2,
        mountedPluginIds: pluginManifests.map((manifest) => manifest.identifier),
        contextRoots: [],
        onStartHook: emptyHookConfig(),
        onFinishHook: emptyHookConfig(),
      },
    });
    const newestAgent = [...nextSnapshot.agentApps].sort((left, right) => right.id - left.id)[0];
    setSelectedAgentId(newestAgent?.id ?? null);
    setActiveRoute(coreRoute("settings"));
  }

  async function handleSaveAgent() {
    if (!agentDraft?.id) {
      return;
    }
    await runSnapshotCommand("update_agent_app", {
      input: {
        id: agentDraft.id,
        name: agentDraft.name,
        systemPrompt: agentDraft.systemPrompt,
        providerKind: agentDraft.providerKind,
        authConfigId: agentDraft.authConfigId,
        model: agentDraft.model,
        temperature: agentDraft.temperature,
        mountedPluginIds: agentDraft.mountedPluginIds,
        contextRoots: agentDraft.contextRoots,
        onStartHook: normalizeHookConfig(agentDraft.onStartHook),
        onFinishHook: normalizeHookConfig(agentDraft.onFinishHook),
      },
    });
  }

  async function handleDeleteAgent() {
    if (!agentDraft?.id || !window.confirm(t("Delete agent “{name}”? Tasks using it switch to the first agent in the list, and other agents' hooks stop referencing it.", { name: agentDraft.name }))) {
      return;
    }
    await runSnapshotCommand("delete_agent_app", {
      input: { id: agentDraft.id },
    });
  }

  return {
    agentDraft,
    setAgentDraft,
    handleCreateAgent,
    handleSaveAgent,
    handleDeleteAgent,
  };
}
