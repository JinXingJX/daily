import { describe, expect, it } from "vitest";
import type { AgentEventView, AgentTurnView } from "@/types";
import { formatElapsed, formatStepDuration, sessionRunState, turnProgress } from "./turnProgress";

function view(id: string, kind: AgentEventView["kind"], status: AgentEventView["status"], createdAt: number, updatedAt = createdAt): AgentEventView {
  return {
    id, parserId: "builtin.default", kind, status, role: null, title: id, content: "",
    ply: null, toolId: null, toolName: null, metadata: null, createdAt, updatedAt,
  };
}

function turn(status: AgentTurnView["status"], eventViews: AgentEventView[]): AgentTurnView {
  return {
    id: 1, prompt: "", response: "", strategy: "", status, partialResponse: "", steps: [], eventViews,
    agentAppId: null, agentName: null, pendingAsyncTool: null, usage: null, errorCode: null,
  };
}

describe("turnProgress", () => {
  it("counts only work steps and reports the step in flight", () => {
    const progress = turnProgress(
      turn("running", [
        view("prompt", "user_input", "completed", 100),
        view("think", "llm_thought", "completed", 100, 104),
        view("read", "tool_call", "completed", 104, 105),
        view("test", "tool_call", "in_progress", 105),
        view("reply", "assistant_text", "in_progress", 106),
      ]),
      160,
    );
    expect(progress.steps.map((step) => step.id)).toEqual(["think", "read", "test"]);
    expect(progress.completed).toBe(2);
    expect(progress.currentStep).toBe(3);
    expect(progress.elapsedSec).toBe(60);
  });

  it("merges a tool call with its result and hides runtime markers", () => {
    const call = { ...view("call", "tool_call", "completed", 10, 11), toolId: "t1", toolName: "read_file" };
    const result = { ...view("result", "tool_result", "completed", 11, 13), toolId: "t1", content: "ok" };
    const progress = turnProgress(
      turn("completed", [view("start", "runtime", "in_progress", 9), call, result]),
      99,
    );
    expect(progress.steps).toHaveLength(1);
    expect(progress.steps[0]).toMatchObject({ id: "call", updatedAt: 13, result: { content: "ok" } });
    expect(progress.currentStep).toBeNull();
  });

  it("freezes elapsed time at the last event once the turn has failed", () => {
    const progress = turnProgress(
      turn("failed", [view("think", "llm_thought", "completed", 10, 12), view("call", "tool_call", "failed", 12, 40)]),
      999,
    );
    expect(progress.state).toBe("failed");
    expect(progress.failedStep?.id).toBe("call");
    expect(progress.currentStep).toBeNull();
    expect(progress.elapsedSec).toBe(30);
  });

  it("has no elapsed time before the first event", () => {
    expect(turnProgress(turn("queued", []), 50)).toMatchObject({ state: "running", elapsedSec: null, currentStep: 1 });
  });
});

describe("sessionRunState", () => {
  const runtime = { isRunning: true, runningSessionIds: [7], status: "Running" };

  it("marks the session the runtime is busy with, even when it is not open", () => {
    expect(sessionRunState(7, runtime, 2, null)).toBe("running");
    expect(sessionRunState(8, runtime, 2, null)).toBe("idle");
  });

  it("marks every concurrently running session", () => {
    const concurrent = { isRunning: true, runningSessionIds: [2, 7], status: "Running" };
    expect(sessionRunState(2, concurrent, 2, turn("running", []))).toBe("running");
    expect(sessionRunState(7, concurrent, 2, null)).toBe("running");
    expect(sessionRunState(8, concurrent, 2, null)).toBe("idle");
  });

  it("surfaces a failed latest turn only for the open session", () => {
    const failed = turn("failed", []);
    expect(sessionRunState(2, null, 2, failed)).toBe("failed");
    expect(sessionRunState(2, null, 2, turn("completed", []))).toBe("idle");
  });
});

describe("formatting", () => {
  it("formats elapsed and step durations", () => {
    expect(formatElapsed(102)).toBe("01:42");
    expect(formatElapsed(3723)).toBe("1:02:03");
    expect(formatStepDuration(0)).toBe("<1s");
    expect(formatStepDuration(12)).toBe("12s");
    expect(formatStepDuration(75)).toBe("01:15");
  });
});
