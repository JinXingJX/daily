import { createElement, type ComponentProps } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it } from "vitest";
import { setLocaleForTests } from "@/i18n";
import type { AgentEventView, AgentTurnView } from "@/types";
import { ChatSurface } from "./ChatSurface";

function view(id: string, kind: AgentEventView["kind"], status: AgentEventView["status"], extra: Partial<AgentEventView> = {}): AgentEventView {
  return {
    id, parserId: "builtin.default", kind, status, role: null, title: id, content: "", ply: 1,
    toolId: null, toolName: null, metadata: null, createdAt: 100, updatedAt: 104, ...extra,
  };
}

const failedTurn: AgentTurnView = {
  id: 7, prompt: "审查插件桥", response: "", strategy: "", status: "failed", partialResponse: "", steps: [],
  agentAppId: 1, agentName: "Reviewer", pendingAsyncTool: null, usage: null, errorCode: null,
  eventViews: [
    view("prompt", "user_input", "completed"),
    view("think", "llm_thought", "completed", { content: "先搜索 IPC 入口" }),
    view("call", "tool_call", "completed", { toolId: "t1", toolName: "search", content: "ipcMain.handle" }),
    view("result", "tool_result", "completed", { toolId: "t1", toolName: "search", content: "14 matches" }),
    view("error", "error", "failed", { content: "sidecar exited while streaming" }),
  ],
};

function props(): ComponentProps<typeof ChatSurface> {
  const noOp = () => {};
  return {
    activeSessionId: 3, activeSessionTitle: "插件权限审查", activeSessionAgentAppId: 1, boundAgent: null,
    modelLabel: "Codex · gpt-5.5", runtimeIsRunning: false, modelReady: true,
    readiness: { modelLabel: "Codex · gpt-5.5", modelReady: true, agentCount: 1, pluginNames: [] },
    agents: [], turnsById: new Map([[7, failedTurn]]), latestTurn: failedTurn, runningMessage: null,
    messages: [
      { id: "m1", messageId: 1, turnId: 7, role: "user", content: "审查插件桥" },
      { id: "m2", messageId: 2, turnId: 7, role: "assistant", content: "" },
    ],
    draft: "", composerImages: [], canSend: false, retryBusy: false, retryMessageId: null,
    onDraftChange: noOp, onComposerImagesChange: noOp, onFiles: noOp, onBindAgent: noOp, onCreateSession: noOp,
    onOpenSettings: noOp, inspectorOpen: false, onCloseInspector: noOp, onSend: noOp, onRetry: noOp,
    continuableTurnId: 7, onContinue: noOp, canStop: false, onStop: noOp,
  };
}

describe("ChatSurface", () => {
  it("shows a failed turn as merged steps with checkpoint recovery", () => {
    const html = renderToStaticMarkup(createElement(ChatSurface, props()));
    expect(html).toContain("2 步完成后失败");
    expect(html).toContain("从断点继续");
    expect(html).toContain("保留 2 个已完成的步骤");
    expect(html).toContain("sidecar exited while streaming");
    // The search call and its result render as one step.
    expect(html.match(/data-status="completed"/g)).toHaveLength(2);
  });

  it("treats a task holding only the runtime greeting as a fresh start", () => {
    const fixture = props();
    fixture.messages = [{ id: "g", messageId: 1, turnId: null, role: "assistant", content: "Daily workspace is ready." }];
    fixture.continuableTurnId = null;
    const html = renderToStaticMarkup(createElement(ChatSurface, fixture));
    expect(html).toContain("从这里开始");
    expect(html).not.toContain("Daily workspace is ready.");
  });

  describe("in English", () => {
    afterEach(() => setLocaleForTests("zh-CN"));

    it("renders the recovery controls in English", () => {
      setLocaleForTests("en");
      const html = renderToStaticMarkup(createElement(ChatSurface, props()));
      expect(html).toContain("Failed after 2 steps");
      expect(html).toContain("Continue from checkpoint");
      expect(html).toContain("Continuing keeps the 2 completed steps");
      expect(html).not.toContain("从断点继续");
    });
  });
});
