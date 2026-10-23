import { SLASH_COMMANDS } from "@/app/constants";
import { Button } from "@/components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";
import type { AgentAppConfig, ComposerImage } from "@/types";
import {
  ArrowUp,
  Bot,
  LoaderCircle,
  Paperclip,
  Square,
  X
} from "lucide-react";
import { useState, type RefObject } from "react";
import { COMPOSER_MODES, ComposerMode } from "./chatTypes";
import { t } from "@/i18n";

export function Composer({
  textareaRef,
  activeSessionId,
  activeSessionTitle,
  activeSessionAgentAppId,
  modelLabel,
  runtimeIsRunning,
  agents,
  canSend,
  turnCommandPending,
  draft,
  composerImages,
  mode,
  large,
  onModeChange,
  onDraftChange,
  onComposerImagesChange,
  onFiles,
  onBindAgent,
  onCreateSession,
  onSend,
  canStop,
  onStop,
}: {
  textareaRef: RefObject<HTMLTextAreaElement | null>;
  activeSessionId: number | null;
  activeSessionTitle: string | null;
  activeSessionAgentAppId: number | null;
  modelLabel: string | null;
  runtimeIsRunning: boolean;
  agents: AgentAppConfig[];
  canSend: boolean;
  turnCommandPending: boolean;
  draft: string;
  composerImages: ComposerImage[];
  mode: ComposerMode | null;
  large: boolean;
  onModeChange: (mode: ComposerMode | null) => void;
  onDraftChange: (value: string) => void;
  onComposerImagesChange: (updater: (current: ComposerImage[]) => ComposerImage[]) => void;
  onFiles: (files: File[]) => void;
  onBindAgent: (agentAppId: number | null) => void;
  onCreateSession: () => void;
  onSend: () => void;
  canStop: boolean;
  onStop: () => void;
}) {
  const [focused, setFocused] = useState(false);
  const placeholder =
    activeSessionId == null || large
      ? t("Describe a task and press ⌘⏎ to start. You can drop in screenshots.")
      : runtimeIsRunning
        ? t("The agent is working. Draft a follow-up and send it when it finishes.")
        : t("Add a follow-up, or type / for commands");
  const activeMode = COMPOSER_MODES.find((item) => item.id === mode) ?? null;

  return (
    <div>
      <div className={cn("composer-card", focused && "composer-card-focused")}>
        {draft.startsWith("/") ? (
          <div className="flex flex-wrap gap-1.5 border-b border-[var(--line)] px-2.5 py-2">
            {SLASH_COMMANDS.map((command) => (
              <button
                key={command.id}
                className="composer-chip"
                type="button"
                onClick={() => {
                  if (command.id === "/new-chat") {
                    onDraftChange("");
                    onCreateSession();
                  } else {
                    onDraftChange(command.apply());
                  }
                }}
              >
                <span className="mono font-medium text-[var(--text-strong)]">{command.id}</span>
                <span className="text-[var(--text-muted)]">{command.label}</span>
              </button>
            ))}
          </div>
        ) : null}

        <Textarea
          ref={textareaRef}
          value={draft}
          maxLength={65_536}
          aria-label={activeSessionTitle ? t("Message “{title}”", { title: activeSessionTitle }) : t("Describe a new task")}
          onChange={(event) => onDraftChange(event.currentTarget.value)}
          onKeyDown={(event) => {
            if ((event.metaKey || event.ctrlKey) && event.key === "Enter" && canSend) {
              event.preventDefault();
              onSend();
            }
          }}
          onFocus={() => setFocused(true)}
          onBlur={() => setFocused(false)}
          className={cn(
            "resize-none rounded-none border-0 bg-transparent px-4 pb-1.5 pt-3.5 leading-6 shadow-none hover:border-0 focus-visible:shadow-none",
            large ? "min-h-[6.5rem] text-[15px]" : "min-h-[4rem] text-[14px]",
          )}
          placeholder={placeholder}
        />

        {composerImages.length ? (
          <div className="flex flex-wrap gap-2 px-3 pb-1">
            {composerImages.map((image, index) => (
              <button
                key={`${image.name}-${index}`}
                className="composer-chip"
                type="button"
                aria-label={t("Remove image {name}", { name: image.name })}
                onClick={() =>
                  onComposerImagesChange((current) => current.filter((_, currentIndex) => currentIndex !== index))
                }
              >
                <Paperclip className="size-3.5 shrink-0" />
                <span className="max-w-[12rem] truncate">{image.name}</span>
                <X className="size-3.5 shrink-0 text-[var(--text-muted)]" />
              </button>
            ))}
          </div>
        ) : null}

        <div className="flex flex-wrap items-center gap-1.5 px-2.5 pb-2.5 pt-1">
          <Button size="icon" variant="ghost" className="size-8 text-[var(--text-soft)]" asChild aria-label={t("Attach images")} title={t("Attach images")}>
            <label>
              <input
                className="hidden"
                type="file"
                accept="image/gif,image/jpeg,image/png,image/webp"
                multiple
                onChange={(event) => {
                  const files = Array.from(event.currentTarget.files ?? []);
                  event.currentTarget.value = "";
                  onFiles(files);
                }}
              />
              <Paperclip className="size-4" />
            </label>
          </Button>
          <div className="segmented" role="radiogroup" aria-label={t("Working mode")}>
            {COMPOSER_MODES.map((item) => (
              <button
                key={item.id}
                type="button"
                role="radio"
                aria-checked={mode === item.id}
                className="segmented-item"
                title={`${item.prompt}${mode === item.id ? ` ${t("(click again to clear)")}` : ""}`}
                onClick={() => onModeChange(mode === item.id ? null : item.id)}
              >
                {item.label}
              </button>
            ))}
          </div>
          <div className="flex-1" />
          <Select
            value={activeSessionAgentAppId ? String(activeSessionAgentAppId) : "__none__"}
            disabled={activeSessionId == null || runtimeIsRunning}
            onValueChange={(value) => onBindAgent(value === "__none__" ? null : Number(value))}
          >
            <SelectTrigger
              aria-label={t("Choose agent")}
              className="composer-agent h-8 min-h-8 w-auto shadow-none focus:shadow-[var(--focus-ring)]"
              title={modelLabel ?? undefined}
            >
              <Bot className="size-[13px] shrink-0" />
              <SelectValue placeholder={t("Choose agent")} />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__none__">{t("No agent bound")}</SelectItem>
              {agents.map((agent) => (
                <SelectItem key={agent.id} value={String(agent.id)}>
                  {agent.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          {canStop ? (
            <Button size="sm" className="h-8 gap-1.5 px-3 text-xs" title={t("Stop; you can continue from the checkpoint later")} onClick={onStop}>
              <Square className="size-3 fill-current" />
              {t("Stop")}
            </Button>
          ) : (
            <Button
              size="icon"
              className="size-8"
              disabled={!canSend || turnCommandPending}
              aria-label={activeSessionId == null ? t("Start task") : t("Send")}
              title={`${t("Send")} (⌘⏎)`}
              onClick={onSend}
            >
              {turnCommandPending ? (
                <LoaderCircle className="size-4 animate-spin motion-reduce:animate-none" />
              ) : (
                <ArrowUp className="size-4" />
              )}
            </Button>
          )}
        </div>
      </div>
      <div className="mt-2 hidden justify-center gap-3.5 text-[11px] text-[var(--text-muted)] sm:flex">
        {canStop ? <span>{t("After stopping you can continue from the checkpoint")}</span> : null}
        {activeMode ? <span>{t("Adds the “{mode}” instructions", { mode: activeMode.label })}</span> : null}
        <span>
          <kbd>⌘⏎</kbd> {t("Send")}
        </span>
        <span>
          <kbd>/</kbd> {t("Commands")}
        </span>
        <span>{t("Drop images to attach")}</span>
      </div>
    </div>
  );
}
