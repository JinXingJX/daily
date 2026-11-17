import { useMemo, type Dispatch, type SetStateAction } from "react";
import { ChatSurface } from "@/features/chat/ChatSurface";
import {
  COMPOSER_MODES,
  type ChatReadiness,
  type ComposerMode,
  type RenderMessage,
  type SettingsTarget,
} from "@/features/chat/chatTypes";
import { type DesktopCommand } from "@/platform/desktop";
import { useAppStore } from "@/useAppStore";
import type { AgentTurnView, AppSnapshot, ComposerImage, DialogueMessage, ProviderKind } from "@/types";
import { errorMessage } from "@/lib/errors";
import { readFileAsBase64 } from "@/lib/files";
import { displaySessionTitle } from "@/features/workspace/sessionTitle";
import { t } from "@/i18n";

export type RunSnapshotCommand = <T,>(command: DesktopCommand, args?: T) => Promise<AppSnapshot>;
const EMPTY_MESSAGES: DialogueMessage[] = [];
const EMPTY_TURNS: AgentTurnView[] = [];

export function withModePrompt(text: string, mode: ComposerMode | null) {
  const prompt = COMPOSER_MODES.find((item) => item.id === mode)?.prompt;
  if (!prompt) return text;
  return text.trim() ? `${text.trimEnd()}\n\n${prompt}` : prompt;
}

export function ChatRoute({
  draft,
  composerImages,
  busyCommand,
  retryMessageId,
  setDraft,
  setComposerImages,
  setRetryMessageId,
  runSnapshotCommand,
  onError,
  onCreateSession,
  onOpenSettings,
  inspectorOpen,
  onCloseInspector,
  signedOutProviders,
}: {
  draft: string;
  composerImages: ComposerImage[];
  busyCommand: string | null;
  retryMessageId: number | null;
  setDraft: Dispatch<SetStateAction<string>>;
  setComposerImages: Dispatch<SetStateAction<ComposerImage[]>>;
  setRetryMessageId: Dispatch<SetStateAction<number | null>>;
  runSnapshotCommand: RunSnapshotCommand;
  onError: (message: string) => void;
  /** Creates and opens a task, resolving to its session id. */
  onCreateSession: () => Promise<number | null>;
  onOpenSettings: (target: SettingsTarget) => void;
  inspectorOpen: boolean;
  onCloseInspector: () => void;
  /** Sign-in providers known to be signed out. */
  signedOutProviders: ProviderKind[];
}) {
  const activeSessionId = useAppStore((state) => state.activeSession?.id ?? null);
  const activeSessionTitle = useAppStore((state) => state.activeSession?.title ?? null);
  const activeSessionAgentAppId = useAppStore((state) => state.activeSession?.agentAppId ?? null);
  const sessionMessages = useAppStore((state) => state.activeSession?.messages ?? EMPTY_MESSAGES);
  const sessionTurns = useAppStore((state) => state.activeSession?.turns ?? EMPTY_TURNS);
  const agentApps = useAppStore((state) => state.agentApps);
  const runtime = useAppStore((state) => state.runtime);
  const modelSettings = useAppStore((state) => state.settings?.effective.model);
  const installedPlugins = useAppStore((state) => state.plugins?.installed);

  const boundAgent = activeSessionAgentAppId
    ? agentApps.find((agent) => agent.id === activeSessionAgentAppId) ?? null
    : null;
  const authConfig = modelSettings?.authConfigs.find((config) => config.id === (boundAgent?.authConfigId ?? modelSettings.authConfigId));
  const modelReady = authConfig?.providerKind !== "antigravity_login" || authConfig.models.length > 0;
  const modelId = boundAgent?.model ?? authConfig?.defaultModel ?? modelSettings?.model ?? null;
  const modelLabel = authConfig ? [authConfig.name, modelId].filter(Boolean).join(" · ") : modelId;
  const modelSignedOut = authConfig ? signedOutProviders.includes(authConfig.providerKind) : false;
  const readiness = useMemo<ChatReadiness>(
    () => ({
      modelLabel: modelLabel ?? t("No model configured"),
      modelReady,
      agentCount: agentApps.length,
      pluginNames: (installedPlugins ?? []).filter((plugin) => plugin.enabled).map((plugin) => plugin.name),
    }),
    [agentApps.length, installedPlugins, modelLabel, modelReady],
  );
  const renderMessages = useMemo(
    () =>
      sessionMessages.map<RenderMessage>((message) => ({
        id: `message-${message.id}`,
        messageId: message.id,
        turnId: message.turnId,
        role: message.role,
        content: message.content,
      })),
    [sessionMessages],
  );
  const turnsById = useMemo(() => new Map(sessionTurns.map((turn) => [turn.id, turn])), [sessionTurns]);
  // Other sessions may run at the same time; only this session's run state gates its controls.
  const sessionRunning = activeSessionId != null && (runtime?.runningSessionIds.includes(activeSessionId) ?? false);
  const runningMessage = useMemo<RenderMessage | null>(() => {
    const runningTurn = [...sessionTurns]
      .reverse()
      .find((turn) => turn.status === "queued" || turn.status === "running" || turn.status === "waiting_async_tool");
    const runningTurnAlreadyRendered =
      runningTurn?.id != null &&
      sessionMessages.some((message) => message.role === "assistant" && message.turnId === runningTurn.id);
    if (!runningTurn || runningTurnAlreadyRendered) {
      return null;
    }
    return {
      id: `events-${runningTurn.id}`,
      turnId: runningTurn.id,
      role: "assistant",
      content: runningTurn.partialResponse || runningTurn.response,
      partial: true,
      eventViews: runningTurn.eventViews,
    };
  }, [sessionMessages, sessionTurns]);

  const hasPendingAsyncTurn = sessionTurns.some((turn) => turn.status === "waiting_async_tool");
  const turnCommandPending =
    busyCommand === "run_session_turn" ||
    busyCommand === "retry_session_turn" ||
    busyCommand === "continue_session_turn" ||
    busyCommand === "create_workspace_node";
  // A task can start from the welcome screen; sending then creates the task first.
  const canSend =
    modelReady &&
    Boolean(draft.trim() || composerImages.length) &&
    !sessionRunning &&
    !hasPendingAsyncTurn &&
    !turnCommandPending;
  // Only the latest turn can continue from its checkpoint; earlier failures can still be retried.
  const latestTurn = sessionTurns[sessionTurns.length - 1] ?? null;
  const continuableTurnId = latestTurn?.status === "failed" && !sessionRunning ? latestTurn.id : null;
  const canStop = sessionRunning;

  async function handleBindAgent(agentAppId: number | null) {
    if (activeSessionId == null) {
      return;
    }
    await runSnapshotCommand("bind_session_agent", {
      input: {
        sessionId: activeSessionId,
        agentAppId,
      },
    });
  }

  async function handleSend(mode: ComposerMode | null) {
    if (!canSend) {
      return;
    }
    const sessionId = activeSessionId ?? (await onCreateSession());
    if (sessionId == null) {
      return;
    }
    await runSnapshotCommand("run_session_turn", {
      input: {
        sessionId,
        text: withModePrompt(draft, mode),
        images: composerImages,
      },
    });
    setDraft("");
    setComposerImages([]);
  }

  async function handleRetry(messageId: number) {
    if (activeSessionId == null || sessionRunning) {
      return;
    }
    setRetryMessageId(messageId);
    try {
      await runSnapshotCommand("retry_session_turn", {
        input: {
          sessionId: activeSessionId,
          messageId,
        },
      });
    } finally {
      setRetryMessageId(null);
    }
  }

  async function handleContinue(turnId: number) {
    if (activeSessionId == null || sessionRunning) {
      return;
    }
    await runSnapshotCommand("continue_session_turn", { input: { sessionId: activeSessionId, turnId } });
  }

  async function handleStop() {
    if (activeSessionId == null || !canStop) {
      return;
    }
    await runSnapshotCommand("stop_session_turn", { input: { sessionId: activeSessionId } });
  }

  async function handleComposerFileChange(files: File[]) {
    if (!files.length) {
      return;
    }
    const allowedTypes = new Set(["image/gif", "image/jpeg", "image/png", "image/webp"]);
    const candidates = files.filter((file) => allowedTypes.has(file.type));
    if (candidates.length !== files.length) {
      throw new Error(t("Only GIF, JPEG, PNG and WebP images are supported."));
    }
    if (composerImages.length + candidates.length > 4) {
      throw new Error(t("A message can include at most 4 images."));
    }
    const oversized = candidates.find((file) => file.size > 5 * 1024 * 1024);
    if (oversized) {
      throw new Error(t("{name} exceeds the 5 MiB image size limit.", { name: oversized.name }));
    }
    const images = await Promise.all(
      candidates.map(async (file) => ({
          name: file.name,
          mimeType: file.type,
          dataBase64: await readFileAsBase64(file),
        })),
    );
    setComposerImages((current) => [...current, ...images]);
  }

  function runChatAction(action: Promise<unknown>) {
    void action.catch((nextError) => onError(errorMessage(nextError)));
  }

  return (
    <ChatSurface
      activeSessionId={activeSessionId}
      activeSessionTitle={activeSessionTitle == null ? null : displaySessionTitle(activeSessionTitle)}
      activeSessionAgentAppId={activeSessionAgentAppId}
      boundAgent={boundAgent}
      modelLabel={modelLabel}
      runtimeIsRunning={sessionRunning}
      modelReady={modelReady}
      modelSignedOut={modelSignedOut}
      readiness={readiness}
      agents={agentApps}
      messages={renderMessages}
      runningMessage={runningMessage}
      turnsById={turnsById}
      latestTurn={latestTurn}
      draft={draft}
      composerImages={composerImages}
      canSend={canSend}
      retryBusy={turnCommandPending}
      retryMessageId={retryMessageId}
      onDraftChange={setDraft}
      onComposerImagesChange={setComposerImages}
      onFiles={(files) => runChatAction(handleComposerFileChange(files))}
      onBindAgent={(agentAppId) => runChatAction(handleBindAgent(agentAppId))}
      onCreateSession={() => runChatAction(onCreateSession())}
      onOpenSettings={onOpenSettings}
      inspectorOpen={inspectorOpen}
      onCloseInspector={onCloseInspector}
      onSend={(mode) => runChatAction(handleSend(mode))}
      onRetry={(messageId) => runChatAction(handleRetry(messageId))}
      continuableTurnId={continuableTurnId}
      onContinue={(turnId) => runChatAction(handleContinue(turnId))}
      canStop={canStop}
      onStop={() => runChatAction(handleStop())}
    />
  );
}
