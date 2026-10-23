import { GitBranchPlus, LoaderCircle } from "lucide-react";
import { isRecord, type PluginSpec } from "@/features/plugins/inline/types";

export type HookAgentResult = {
  agentId: number;
  agentName: string;
  status: "running" | "completed" | "failed";
  model: string;
  output?: string | null;
  error?: string | null;
};

export type AgentHooksPayload = {
  title: string;
  phase: "on_start" | "on_finish";
  mode: "parallel" | "serial";
  appendPreviousOutput: boolean;
  agents: HookAgentResult[];
};

function isAgentHooksPayload(value: unknown): value is AgentHooksPayload {
  if (
    !isRecord(value) ||
    typeof value.title !== "string" ||
    typeof value.phase !== "string" ||
    typeof value.mode !== "string" ||
    typeof value.appendPreviousOutput !== "boolean" ||
    !Array.isArray(value.agents)
  ) {
    return false;
  }
  return value.agents.every(
    (item) =>
      isRecord(item) &&
      typeof item.agentId === "number" &&
      typeof item.agentName === "string" &&
      typeof item.status === "string" &&
      typeof item.model === "string" &&
      (item.output === undefined || item.output === null || typeof item.output === "string") &&
      (item.error === undefined || item.error === null || typeof item.error === "string"),
  );
}

function normalizeHookText(value: string) {
  return value.replace(/\r\n/g, "\n").trim();
}

function hookPhaseLabel(phase: AgentHooksPayload["phase"]) {
  return phase === "on_start" ? "On Start" : "On Finish";
}

function buildHookMeta(payload: AgentHooksPayload) {
  const parts = [`${hookPhaseLabel(payload.phase)} Hook`];
  if (payload.mode === "parallel") {
    parts.push("Parallel");
  } else if (payload.appendPreviousOutput) {
    parts.push("Serial With Context");
  } else {
    parts.push("Serial");
  }
  return parts.join(" · ");
}


function buildHookOutput(payload: AgentHooksPayload) {
  const grouped = new Map<string, { agentNames: string[]; text: string }>();

  for (const agent of payload.agents) {
    const rawValue = normalizeHookText(agent.output ?? agent.error ?? "");
    if (!rawValue) {
      continue;
    }

    const text = agent.error ? `ERROR: ${rawValue}` : rawValue;
    const group = grouped.get(text);
    if (group) {
      group.agentNames.push(agent.agentName);
      continue;
    }

    grouped.set(text, {
      agentNames: [agent.agentName],
      text,
    });
  }

  if (grouped.size === 0) {
    return null;
  }

  return Array.from(grouped.values())
    .map(({ agentNames, text }) => {
      if (payload.agents.length === 1) {
        return text;
      }
      return `[${agentNames.join(", ")}] ${text}`;
    })
    .join("\n\n");
}

function AgentHooksCard({ payload }: { payload: AgentHooksPayload }) {
  const summary = buildHookMeta(payload);
  const output = buildHookOutput(payload);

  return (
    <div className="space-y-2 border-y border-[var(--line)] px-1 py-2">
      <div className="text-xs text-[var(--text-muted)]">{summary}</div>
      {output ? (
        <div className="whitespace-pre-wrap break-words text-sm leading-6 text-[var(--text-strong)]">{output}</div>
      ) : (
        <div className="flex items-center gap-2 text-sm text-[var(--text-muted)]">
          <LoaderCircle className="size-4 animate-spin" />
          Waiting for Hook Output…
        </div>
      )}
    </div>
  );
}

function AgentHooksSkeleton() {
  return (
    <div className="space-y-2 border-y border-[var(--line)] px-1 py-2">
      <div className="text-xs text-[var(--text-muted)]">Hook Lifecycle · Pending</div>
      <div className="flex items-center gap-2 text-sm text-[var(--text-muted)]">
        <LoaderCircle className="size-4 animate-spin" />
        Waiting for Hook Output…
      </div>
    </div>
  );
}

export const agentHooksSpec: PluginSpec<AgentHooksPayload> = {
  pluginId: "builtin.agent_hooks",
  renderer: "agent_hooks",
  legacyIdentifier: "plugin:agent_hooks",
  label: "Agent Hooks",
  icon: <GitBranchPlus className="size-4" />,
  validate: isAgentHooksPayload,
  render: (payload: AgentHooksPayload) => <AgentHooksCard payload={payload} />,
  skeleton: <AgentHooksSkeleton />,
  chrome: "bare",
};
