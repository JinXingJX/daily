import type { AgentAppConfig, AgentTurnView, ComposerImage } from "@/types";
import { ImagePlus } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { ActivityPanel } from "./ActivityPanel";
import { ChatReadiness, ComposerMode, RenderMessage, SettingsTarget } from "./chatTypes";
import { Composer } from "./Composer";
import { MessageFeed } from "./MessageFeed";
import { StartCanvas } from "./StartCanvas";
import { t } from "@/i18n";

type ChatSurfaceProps = {
  activeSessionId: number | null;
  activeSessionTitle: string | null;
  activeSessionAgentAppId: number | null;
  boundAgent: AgentAppConfig | null;
  modelLabel: string | null;
  runtimeIsRunning: boolean;
  modelReady: boolean;
  /** The model's sign-in provider is known to be signed out. */
  modelSignedOut: boolean;
  readiness: ChatReadiness;
  agents: AgentAppConfig[];
  messages: RenderMessage[];
  runningMessage: RenderMessage | null;
  turnsById: Map<number, AgentTurnView>;
  latestTurn: AgentTurnView | null;
  draft: string;
  composerImages: ComposerImage[];
  canSend: boolean;
  retryBusy: boolean;
  retryMessageId: number | null;
  onDraftChange: (value: string) => void;
  onComposerImagesChange: (updater: (current: ComposerImage[]) => ComposerImage[]) => void;
  onFiles: (files: File[]) => void;
  onBindAgent: (agentAppId: number | null) => void;
  onCreateSession: () => void;
  onOpenSettings: (target: SettingsTarget) => void;
  inspectorOpen: boolean;
  onCloseInspector: () => void;
  onSend: (mode: ComposerMode | null) => void;
  onRetry: (messageId: number) => void;
  /** The failed latest turn, which can continue from its checkpoint. */
  continuableTurnId: number | null;
  onContinue: (turnId: number) => void;
  canStop: boolean;
  onStop: () => void;
};

export function ChatSurface(props: ChatSurfaceProps) {
  const [dragActive, setDragActive] = useState(false);
  const [mode, setMode] = useState<ComposerMode | null>(null);
  const composerRef = useRef<HTMLTextAreaElement | null>(null);
  // A new task only holds the runtime's greeting; treat it as empty until the user has written.
  const feedEmpty =
    props.activeSessionId == null ||
    (!props.runningMessage && !props.messages.some((message) => message.role === "user" || message.turnId != null));

  useEffect(() => {
    setMode(null);
  }, [props.activeSessionId]);

  const fillDraft = (value: string) => {
    props.onDraftChange(value);
    requestAnimationFrame(() => composerRef.current?.focus());
  };

  const composer = (
    <Composer
      textareaRef={composerRef}
      activeSessionId={props.activeSessionId}
      activeSessionTitle={props.activeSessionTitle}
      activeSessionAgentAppId={props.activeSessionAgentAppId}
      modelLabel={props.modelLabel}
      modelSignedOut={props.modelSignedOut}
      runtimeIsRunning={props.runtimeIsRunning}
      agents={props.agents}
      canSend={props.canSend}
      turnCommandPending={props.retryBusy}
      draft={props.draft}
      composerImages={props.composerImages}
      mode={mode}
      large={feedEmpty}
      onModeChange={setMode}
      onDraftChange={props.onDraftChange}
      onComposerImagesChange={props.onComposerImagesChange}
      onFiles={props.onFiles}
      onBindAgent={props.onBindAgent}
      onCreateSession={props.onCreateSession}
      onSend={() => props.onSend(mode)}
      canStop={props.canStop}
      onStop={props.onStop}
    />
  );

  return (
    <div className="chat-workspace relative flex min-h-0 flex-1 overflow-hidden">
      <section
        className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden"
        aria-label={t("Chat")}
        onDragOver={(event) => {
          event.preventDefault();
          setDragActive(true);
        }}
        onDragLeave={(event) => {
          if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setDragActive(false);
        }}
        onDrop={(event) => {
          event.preventDefault();
          setDragActive(false);
          props.onFiles(Array.from(event.dataTransfer.files));
        }}
      >
        {dragActive ? (
          <div className="chat-drop-active pointer-events-none absolute inset-0 z-20 grid place-items-center border-2 border-dashed border-[var(--accent-line)]">
            <div className="flex items-center gap-3 rounded-[var(--radius-panel)] border border-[var(--accent-line)] bg-[var(--surface-strong)] px-4 py-2.5 text-sm font-medium text-[var(--accent-text)] shadow-[var(--shadow-popover)]">
              <ImagePlus className="size-4" />
              {t("Drop to attach images")}
            </div>
          </div>
        ) : null}

        {feedEmpty ? (
          <StartCanvas
            sessionTitle={props.activeSessionId != null ? props.activeSessionTitle : null}
            readiness={props.readiness}
            composer={composer}
            onPrompt={fillDraft}
            onOpenSettings={props.onOpenSettings}
          />
        ) : (
          <>
            <MessageFeed
              messages={props.messages}
              runningMessage={props.runningMessage}
              turnsById={props.turnsById}
              retryBusy={props.retryBusy}
              retryMessageId={props.retryMessageId}
              onRetry={props.onRetry}
              continuableTurnId={props.continuableTurnId}
              onContinue={props.onContinue}
            />
            <div className="composer-dock">
              <div className="mx-auto w-full max-w-[45rem]">{composer}</div>
            </div>
          </>
        )}
      </section>
      {props.inspectorOpen ? (
        <ActivityPanel
          latestTurn={props.latestTurn}
          agent={props.boundAgent}
          agents={props.agents}
          modelLabel={props.modelLabel}
          onClose={props.onCloseInspector}
          onOpenSettings={props.onOpenSettings}
        />
      ) : null}
    </div>
  );
}
