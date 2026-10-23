import { describe, expect, it } from "vitest";
import { applyTurnDeltaToSession } from "@/app/turnDelta";
import type { AgentEventView, AgentTurnView, ChatSessionView, TurnDelta } from "@/types";

const view = (overrides: Partial<AgentEventView> = {}): AgentEventView => ({
  id: "v1",
  parserId: "core",
  kind: "assistant_text",
  status: "in_progress",
  role: "assistant",
  title: "Answer",
  content: "Hel",
  ply: 0,
  toolId: null,
  toolName: null,
  metadata: null,
  createdAt: 1,
  updatedAt: 1,
  ...overrides,
});

const turn: AgentTurnView = {
  id: 7,
  prompt: "hi",
  response: "",
  strategy: "",
  status: "running",
  partialResponse: "He",
  steps: [],
  eventViews: [view()],
  agentAppId: 1,
  agentName: "Agent",
  pendingAsyncTool: null,
};

const session: ChatSessionView = {
  id: 3,
  title: "Chat",
  agentAppId: 1,
  messages: [{ id: 1, turnId: 7, role: "user", content: "hi" }],
  turns: [turn],
  updatedAt: 10,
  hasRunningTurn: true,
};

const delta = (overrides: Partial<TurnDelta>): TurnDelta => ({
  sessionId: 3,
  turnId: 7,
  activeSessionId: 3,
  runtime: { isRunning: true, runningSessionIds: [3], status: "Running" },
  messageAppends: [],
  turnPatch: {},
  ...overrides,
});

describe("applyTurnDeltaToSession", () => {
  it("keeps identity when a delta targets another session or changes nothing", () => {
    expect(applyTurnDeltaToSession(session, delta({ sessionId: 99 }))).toBe(session);
    expect(applyTurnDeltaToSession(session, delta({ sessionUpdatedAt: 10 }))).toBe(session);
  });

  it("appends streamed text, dedupes messages and merges event views", () => {
    const next = applyTurnDeltaToSession(
      session,
      delta({
        sessionUpdatedAt: 11,
        messageAppends: [session.messages[0], { id: 2, turnId: 7, role: "assistant", content: "Hello" }],
        turnPatch: {
          partialResponseAppend: "llo",
          eventViewPatches: [{ view: view({ content: "lo", updatedAt: 2 }), merge: "append_content" }],
        },
      }),
    )!;
    expect(next.messages.map((message) => message.id)).toEqual([1, 2]);
    expect(next.turns[0].partialResponse).toBe("Hello");
    expect(next.turns[0].eventViews[0]).toMatchObject({ content: "Hello", createdAt: 1, updatedAt: 2 });
    expect(next.updatedAt).toBe(11);
  });

  it("recomputes running state when a turn completes", () => {
    const next = applyTurnDeltaToSession(
      session,
      delta({ turnPatch: { status: "completed", response: "Done", clearPartialResponse: true } }),
    )!;
    expect(next.hasRunningTurn).toBe(false);
    expect(next.turns[0]).toMatchObject({ status: "completed", response: "Done", partialResponse: "" });
  });
});
