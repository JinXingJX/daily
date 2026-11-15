import type {
  AgentEventView,
  AgentEventViewPatch,
  AgentTurnView,
  ChatSessionView,
  DialogueMessage,
  TurnDelta,
  TurnPatch,
} from "@/types";

// Pure reducers that fold streaming TurnDelta events into the active session.
// They return the original object when nothing changed so React selectors stay stable.


export function applyTurnDeltaToSession(session: ChatSessionView | null, delta: TurnDelta) {
  if (!session || session.id !== delta.sessionId) {
    return session;
  }

  const messages = appendMessages(session.messages, delta.messageAppends);
  const turns = applyTurnDeltaToTurns(session.turns, delta.turnId, delta.turnPatch);
  const hasRunningTurn = turns.some((turn) => turn.status === "running" || turn.status === "waiting_async_tool");

  if (messages === session.messages && turns === session.turns && session.updatedAt === delta.sessionUpdatedAt) {
    return session;
  }

  return {
    ...session,
    messages,
    turns,
    updatedAt: delta.sessionUpdatedAt ?? session.updatedAt,
    hasRunningTurn,
  };
}

function appendMessages(current: DialogueMessage[], appends: DialogueMessage[]) {
  if (!appends.length) {
    return current;
  }
  const existingIds = new Set(current.map((message) => message.id));
  const nextMessages = appends.filter((message) => !existingIds.has(message.id));
  if (!nextMessages.length) {
    return current;
  }
  return [...current, ...nextMessages];
}

function applyTurnDeltaToTurns(turns: AgentTurnView[], turnId: number, patch: TurnPatch) {
  const index = turns.findIndex((turn) => turn.id === turnId);
  if (index === -1) {
    return turns;
  }
  const nextTurn = applyTurnPatch(turns[index], patch);
  if (nextTurn === turns[index]) {
    return turns;
  }
  const nextTurns = turns.slice();
  nextTurns[index] = nextTurn;
  return nextTurns;
}

function applyTurnPatch(turn: AgentTurnView, patch: TurnPatch) {
  let next = turn;
  const update = () => {
    if (next === turn) {
      next = { ...turn };
    }
    return next;
  };

  if (patch.status !== undefined) {
    update().status = patch.status;
  }
  if (patch.strategy !== undefined) {
    update().strategy = patch.strategy;
  }
  if (patch.response !== undefined) {
    update().response = patch.response;
  }
  if (patch.clearPartialResponse) {
    update().partialResponse = "";
  }
  if (patch.partialResponse !== undefined) {
    update().partialResponse = patch.partialResponse;
  }
  if (patch.partialResponseAppend !== undefined) {
    update().partialResponse = `${next.partialResponse}${patch.partialResponseAppend}`;
  }
  if (patch.clearPendingAsyncTool) {
    update().pendingAsyncTool = null;
  }
  if ("pendingAsyncTool" in patch) {
    update().pendingAsyncTool = patch.pendingAsyncTool ?? null;
  }
  if (patch.steps !== undefined) {
    update().steps = patch.steps;
  }
  if (patch.stepsAppend?.length) {
    update().steps = [...next.steps, ...patch.stepsAppend];
  }
  if (patch.eventViewPatches?.length) {
    update().eventViews = applyEventViewPatches(next.eventViews, patch.eventViewPatches);
  }
  if (patch.usage !== undefined) {
    update().usage = patch.usage;
  }
  if (patch.errorCode !== undefined) {
    update().errorCode = patch.errorCode;
  }

  return next;
}

function applyEventViewPatches(current: AgentEventView[], patches: AgentEventViewPatch[]) {
  let next = current;
  for (const patch of patches) {
    const index = next.findIndex((view) => view.id === patch.view.id);
    if (index === -1) {
      next = [...next, patch.view];
      continue;
    }

    const merged = mergeEventView(next[index], patch);
    if (merged === next[index]) {
      continue;
    }
    const copy = next.slice();
    copy[index] = merged;
    next = copy;
  }
  return next;
}

function mergeEventView(existing: AgentEventView, patch: AgentEventViewPatch) {
  const next = patch.view;
  switch (patch.merge) {
    case "replace":
      return { ...next, createdAt: existing.createdAt };
    case "append_content":
      return updateEventViewFields(existing, next, `${existing.content}${next.content}`);
    case "update":
      return updateEventViewFields(
        existing,
        next,
        existing.content || !next.content ? existing.content : next.content,
      );
  }
}

function updateEventViewFields(existing: AgentEventView, next: AgentEventView, content: string) {
  return {
    ...existing,
    parserId: next.parserId,
    kind: next.kind,
    status: next.status,
    role: next.role,
    title: next.title,
    content,
    ply: next.ply,
    toolId: next.toolId,
    toolName: next.toolName,
    metadata: next.metadata,
    updatedAt: next.updatedAt,
  };
}
