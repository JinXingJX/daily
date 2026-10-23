import type { AgentAppConfig, AgentHookConfig } from "@/types";

export type AgentFormState = {
  id: number | null;
  name: string;
  systemPrompt: string;
  providerKind: AgentAppConfig["providerKind"];
  authConfigId: string;
  model: string;
  temperature: number;
  mountedPluginIds: string[];
  contextRoots: string[];
  onStartHook: AgentHookConfig;
  onFinishHook: AgentHookConfig;
};

export type HookField = "onStartHook" | "onFinishHook";

export function agentToFormState(agent: AgentAppConfig, fallbackAuthConfigId: string): AgentFormState {
  return {
    id: agent.id,
    name: agent.name,
    systemPrompt: agent.systemPrompt,
    providerKind: agent.providerKind,
    authConfigId: agent.authConfigId || fallbackAuthConfigId,
    model: agent.model,
    temperature: agent.temperature,
    mountedPluginIds: [...agent.mountedPluginIds],
    contextRoots: [...agent.contextRoots],
    onStartHook: { ...agent.onStartHook, agentAppIds: [...agent.onStartHook.agentAppIds] },
    onFinishHook: { ...agent.onFinishHook, agentAppIds: [...agent.onFinishHook.agentAppIds] },
  };
}

export function emptyHookConfig(): AgentHookConfig {
  return {
    mode: null,
    appendPreviousOutput: false,
    agentAppIds: [],
  };
}

export function normalizeHookConfig(config: AgentHookConfig): AgentHookConfig {
  if (!config.mode || !config.agentAppIds.length) {
    return emptyHookConfig();
  }
  return {
    mode: config.mode,
    appendPreviousOutput: config.mode === "serial" ? config.appendPreviousOutput : false,
    agentAppIds: [...config.agentAppIds],
  };
}

export function toggleHookAgentSelection(config: AgentHookConfig, agentId: number): AgentHookConfig {
  const exists = config.agentAppIds.includes(agentId);
  return {
    ...config,
    agentAppIds: exists
      ? config.agentAppIds.filter((id) => id !== agentId)
      : [...config.agentAppIds, agentId],
  };
}

export function moveHookAgent(config: AgentHookConfig, agentId: number, direction: -1 | 1): AgentHookConfig {
  const currentIndex = config.agentAppIds.indexOf(agentId);
  if (currentIndex === -1) {
    return config;
  }
  const nextIndex = currentIndex + direction;
  if (nextIndex < 0 || nextIndex >= config.agentAppIds.length) {
    return config;
  }
  const nextIds = [...config.agentAppIds];
  [nextIds[currentIndex], nextIds[nextIndex]] = [nextIds[nextIndex], nextIds[currentIndex]];
  return {
    ...config,
    agentAppIds: nextIds,
  };
}

function uniqueAgentIds(ids: number[]) {
  return Array.from(new Set(ids));
}

function buildHookAdjacency(
  agents: AgentAppConfig[],
  draft: AgentFormState | null,
  overrides?: Partial<Pick<AgentFormState, HookField>>,
) {
  const graph = new Map<number, number[]>();
  for (const agent of agents) {
    const startHook =
      draft && agent.id === draft.id ? overrides?.onStartHook ?? draft.onStartHook : agent.onStartHook;
    const finishHook =
      draft && agent.id === draft.id ? overrides?.onFinishHook ?? draft.onFinishHook : agent.onFinishHook;
    graph.set(agent.id, uniqueAgentIds([...startHook.agentAppIds, ...finishHook.agentAppIds]));
  }
  return graph;
}

function hasHookPath(
  graph: Map<number, number[]>,
  startId: number,
  targetId: number,
  visited = new Set<number>(),
): boolean {
  if (startId === targetId) {
    return true;
  }
  if (visited.has(startId)) {
    return false;
  }
  visited.add(startId);
  const targets = graph.get(startId) ?? [];
  return targets.some((nextId) => hasHookPath(graph, nextId, targetId, visited));
}

export function wouldIntroduceHookCycle(
  agents: AgentAppConfig[],
  draft: AgentFormState | null,
  hookField: HookField,
  nextHook: AgentHookConfig,
  candidateAgentId: number,
) {
  if (!draft?.id) {
    return false;
  }
  const graph = buildHookAdjacency(agents, draft, {
    [hookField]: nextHook,
  });
  return hasHookPath(graph, candidateAgentId, draft.id);
}
