import { describe, expect, it } from "vitest";
import type { AgentFormState } from "@/features/agents/agentHooks";
import type { AgentAppConfig } from "@/types";
import { changedAgentFields } from "./agentDraftDiff";

const saved: AgentAppConfig = {
  id: 1,
  name: "Builder",
  systemPrompt: "Be precise.",
  providerKind: "codex_login",
  authConfigId: "codex",
  model: "gpt-5.5",
  temperature: 0.2,
  mountedPluginIds: ["daily.writer", "daily.trading_timeline"],
  contextRoots: ["crates/daily-core"],
  onStartHook: { mode: null, appendPreviousOutput: false, agentAppIds: [] },
  onFinishHook: { mode: "serial", appendPreviousOutput: true, agentAppIds: [2] },
};

function draftOf(agent: AgentAppConfig): AgentFormState {
  return {
    id: agent.id,
    name: agent.name,
    systemPrompt: agent.systemPrompt,
    providerKind: agent.providerKind,
    authConfigId: agent.authConfigId,
    model: agent.model,
    temperature: agent.temperature,
    mountedPluginIds: [...agent.mountedPluginIds],
    contextRoots: [...agent.contextRoots],
    onStartHook: { ...agent.onStartHook },
    onFinishHook: { ...agent.onFinishHook },
  };
}

describe("changedAgentFields", () => {
  it("reports nothing for an untouched draft, ignoring list order of sets", () => {
    const draft = draftOf(saved);
    draft.mountedPluginIds.reverse();
    expect(changedAgentFields(draft, saved)).toEqual([]);
  });

  it("lists edited fields in form order", () => {
    const draft = draftOf(saved);
    draft.temperature = 0.7;
    draft.systemPrompt = "Be brief.";
    draft.contextRoots = [];
    expect(changedAgentFields(draft, saved)).toEqual(["温度", "系统提示词", "上下文目录"]);
  });

  it("treats a hook without agents as disabled, and order of hook agents as meaningful", () => {
    const draft = draftOf(saved);
    draft.onStartHook = { mode: "parallel", appendPreviousOutput: false, agentAppIds: [] };
    expect(changedAgentFields(draft, saved)).toEqual([]);
    draft.onFinishHook = { ...draft.onFinishHook, agentAppIds: [3, 2] };
    const reordered = { ...saved, onFinishHook: { ...saved.onFinishHook, agentAppIds: [2, 3] } };
    expect(changedAgentFields(draft, reordered)).toEqual(["完成后钩子"]);
  });
});
