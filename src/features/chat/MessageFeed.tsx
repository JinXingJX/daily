import { useNowSec } from "@/app/useNowSec";
import { Button } from "@/components/ui/button";
import {
  formatElapsed,
  formatStepDuration,
  stepDurationSec,
  turnProgress,
  turnSteps,
  type TurnProgress,
  type TurnStep,
} from "@/features/chat/turnProgress";
import { DailyMark } from "@/components/app/DailyMark";
import { cn } from "@/lib/utils";
import type { AgentEventView, AgentTurnView } from "@/types";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  Check,
  ChevronDown,
  CircleAlert,
  CircleDashed,
  CirclePause,
  LoaderCircle,
  RotateCcw,
  StepForward
} from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { Suspense, lazy, memo, useEffect, useRef, useState, type ReactNode } from "react";
import { RenderMessage } from "./chatTypes";
import { t } from "@/i18n";

export const ChatRenderer = lazy(() =>
  import("@/chatRenderer").then((module) => ({ default: module.ChatRenderer })),
);

export const MessageFeed = memo(function MessageFeed({
  messages,
  runningMessage,
  turnsById,
  onRetry,
  retryBusy,
  retryMessageId,
  continuableTurnId,
  onContinue,
}: {
  messages: RenderMessage[];
  runningMessage: RenderMessage | null;
  turnsById: Map<number, AgentTurnView>;
  onRetry: (messageId: number) => void;
  retryBusy: boolean;
  retryMessageId: number | null;
  continuableTurnId: number | null;
  onContinue: (turnId: number) => void;
}) {
  // The continue action sits on the turn's last assistant message (its failure reply).
  const continueMessageId =
    continuableTurnId == null
      ? null
      : [...messages].reverse().find((message) => message.role === "assistant" && message.turnId === continuableTurnId)?.id ?? null;
  const parentRef = useRef<HTMLDivElement | null>(null);
  const itemCount = messages.length + (runningMessage ? 1 : 0);
  const useVirtualization = itemCount > 40;
  const getMessageAt = (index: number) => (index < messages.length ? messages[index] : runningMessage);
  const virtualizer = useVirtualizer({
    count: itemCount,
    getScrollElement: () => parentRef.current,
    getItemKey: (index) => getMessageAt(index)?.id ?? index,
    estimateSize: () => 220,
    overscan: 6,
  });

  // The feed follows new output only while the reader is at the bottom; scrolling up to read
  // earlier steps is never interrupted. A new message always brings the reader back down.
  const followRef = useRef(true);
  const contentRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    followRef.current = true;
  }, [itemCount]);

  useEffect(() => {
    const element = parentRef.current;
    if (!element || !followRef.current) {
      return;
    }
    if (useVirtualization && itemCount > 0) {
      virtualizer.scrollToIndex(itemCount - 1, { align: "end" });
      return;
    }
    element.scrollTop = element.scrollHeight;
  }, [itemCount, messages, runningMessage, useVirtualization, virtualizer]);

  // Markdown, code highlighting and step output finish laying out after the message arrives.
  useEffect(() => {
    const element = parentRef.current;
    const content = contentRef.current;
    if (!element || !content || useVirtualization || typeof ResizeObserver === "undefined") {
      return;
    }
    const observer = new ResizeObserver(() => {
      if (followRef.current) element.scrollTop = element.scrollHeight;
    });
    observer.observe(content);
    return () => observer.disconnect();
  }, [useVirtualization]);

  const card = (message: RenderMessage, animated: boolean) => (
    <MessageCard
      key={message.id}
      message={message}
      turn={message.turnId != null ? turnsById.get(message.turnId) ?? null : null}
      eventViews={eventViewsForMessage(message, turnsById)}
      onRetry={onRetry}
      retryBusy={retryBusy}
      retryPending={retryMessageId === message.messageId}
      onContinue={message.id === continueMessageId ? onContinue : undefined}
      animated={animated}
    />
  );

  return (
    <div
      className="min-h-0 flex-1 overflow-auto px-6 pb-6 pt-5 sm:px-8"
      ref={parentRef}
      onScroll={(event) => {
        const element = event.currentTarget;
        followRef.current = element.scrollHeight - element.scrollTop - element.clientHeight < 80;
      }}
    >
      {useVirtualization ? (
        <div className="relative mx-auto w-full max-w-[45rem]" style={{ height: virtualizer.getTotalSize() }}>
          {virtualizer.getVirtualItems().map((item) => {
            const message = getMessageAt(item.index);
            if (!message) {
              return null;
            }
            return (
              <div
                key={item.key}
                ref={virtualizer.measureElement}
                data-index={item.index}
                className="absolute left-0 top-0 w-full pb-6 [contain:layout_paint]"
                style={{ transform: `translateY(${item.start}px)` }}
              >
                {card(message, false)}
              </div>
            );
          })}
        </div>
      ) : (
        <div ref={contentRef} className="mx-auto flex w-full max-w-[45rem] flex-col gap-[22px] pb-4">
          <AnimatePresence initial={false}>
            {messages.map((message) => card(message, true))}
            {runningMessage ? card(runningMessage, true) : null}
          </AnimatePresence>
        </div>
      )}
    </div>
  );
});

export function eventViewsForMessage(message: RenderMessage, turnsById: Map<number, AgentTurnView>) {
  if (message.eventViews) {
    return message.eventViews;
  }
  if (message.role !== "assistant" || message.turnId == null) {
    return undefined;
  }
  return turnsById.get(message.turnId)?.eventViews;
}

export const MessageCard = memo(function MessageCard(props: {
  message: RenderMessage;
  turn: AgentTurnView | null;
  eventViews?: AgentEventView[];
  onRetry: (messageId: number) => void;
  retryBusy: boolean;
  retryPending: boolean;
  onContinue?: (turnId: number) => void;
  animated: boolean;
}) {
  const { message, turn, eventViews, onRetry, retryBusy, retryPending, onContinue, animated } = props;
  const continuable = Boolean(onContinue) && message.turnId != null;
  const isAssistant = message.role === "assistant";
  const retryable = isAssistant && !message.partial && message.turnId != null && message.messageId != null;
  const live = Boolean(message.partial);
  const now = useNowSec(live);
  const progress = turnProgress(turn ? { ...turn, eventViews: eventViews ?? turn.eventViews } : null, now);
  const steps = turn ? progress.steps : turnSteps(eventViews);

  return (
    <motion.article
      layout={animated}
      initial={animated ? { opacity: 0, y: 10 } : false}
      animate={animated ? { opacity: 1, y: 0 } : undefined}
      exit={animated ? { opacity: 0, y: -6 } : undefined}
      transition={animated ? { duration: 0.2, ease: [0.25, 1, 0.5, 1] } : undefined}
      className={cn("group flex w-full flex-col gap-2.5", !isAssistant && "items-end")}
      aria-label={isAssistant ? (live ? t("Agent reply (streaming)") : t("Agent reply")) : t("Your message")}
    >
      {isAssistant ? (
        <>
          <TurnHeader agentName={turn?.agentName ?? null} progress={progress} live={live} hasTurn={Boolean(turn)} />
          {steps.length ? <StepCard steps={steps} live={live} progress={progress} /> : null}
          {message.content.trim() ? (
            <Suspense fallback={<div className="text-sm text-[var(--text-muted)]">{t("Rendering…")}</div>}>
              <ChatRenderer content={message.content} isStreaming={message.partial} />
            </Suspense>
          ) : null}
          {continuable ? (
            <div className="recovery-panel">
              <span className="min-w-0 flex-[1_1_16rem] text-[13px] leading-5 text-[var(--text-soft)]">
                {t("Continuing keeps the {count} completed steps and reruns only what failed. Starting over discards every step of this turn.", { count: progress.completed })}
              </span>
              <Button
                size="sm"
                variant="secondary"
                className="h-8 text-xs"
                disabled={retryBusy}
                onClick={() => onRetry(message.messageId as number)}
              >
                <RotateCcw className="size-3.5" />
                {retryPending ? t("Starting over…") : t("Start over")}
              </Button>
              <Button
                size="sm"
                className="h-8 text-xs"
                disabled={retryBusy}
                onClick={() => onContinue?.(message.turnId as number)}
              >
                <StepForward className="size-3.5" />
                {t("Continue from checkpoint")}
              </Button>
            </div>
          ) : retryable ? (
            <div className="flex justify-end opacity-0 transition-opacity focus-within:opacity-100 group-hover:opacity-100">
              <Button
                size="sm"
                variant="ghost"
                className="h-7 text-xs text-[var(--text-muted)]"
                disabled={retryBusy}
                onClick={() => onRetry(message.messageId as number)}
              >
                <RotateCcw className="size-3.5" />
                {retryPending ? t("Retrying…") : t("Retry")}
              </Button>
            </div>
          ) : null}
        </>
      ) : (
        <div className="user-message">{message.content}</div>
      )}
    </motion.article>
  );
});

export function TurnHeader({
  agentName,
  progress,
  live,
  hasTurn,
}: {
  agentName: string | null;
  progress: TurnProgress;
  live: boolean;
  hasTurn: boolean;
}) {
  let status: ReactNode = null;
  if (live || progress.state === "running") {
    status = <span className="text-[var(--text-soft)]">{t("Working · {count} steps done", { count: progress.completed })}</span>;
  } else if (progress.state === "waiting") {
    status = <span className="font-medium text-[var(--warning)]">{t("Waiting for plugin confirmation")}</span>;
  } else if (progress.state === "failed") {
    status = <span className="font-medium text-[var(--danger)]">{t("Failed after {count} steps", { count: progress.completed })}</span>;
  } else if (hasTurn && progress.steps.length) {
    status = (
      <span className="text-[var(--text-muted)]">
        {t("{count} steps", { count: progress.steps.length })}{progress.elapsedSec != null ? ` · ${t("took {time}", { time: formatElapsed(progress.elapsedSec) })}` : ""}
      </span>
    );
  }
  return (
    <div className="flex items-center gap-2 text-[13px]">
      <DailyMark className="agent-mark size-[18px]" />
      <span className="font-medium text-[var(--text-strong)]">{agentName ?? t("Agent")}</span>
      {live && progress.elapsedSec != null ? (
        <span className="mono text-[12px] text-[var(--text-muted)]">{formatElapsed(progress.elapsedSec)}</span>
      ) : null}
      <span className="flex-1" />
      <span className="text-[12px]">{status}</span>
    </div>
  );
}

export function StepCard({ steps, live, progress }: { steps: TurnStep[]; live: boolean; progress: TurnProgress }) {
  const failed = steps.some((step) => step.status === "failed");
  const [expanded, setExpanded] = useState(live || failed);
  const now = useNowSec(live);
  useEffect(() => {
    if (live || failed) setExpanded(true);
  }, [live, failed]);
  const collapsible = !live;

  return (
    <div className="step-card">
      {collapsible ? (
        <button
          type="button"
          className="step-card-summary"
          aria-expanded={expanded}
          onClick={() => setExpanded((value) => !value)}
        >
          {failed ? (
            <CircleAlert className="size-3.5 shrink-0 text-[var(--danger)]" aria-hidden="true" />
          ) : (
            <Check className="size-3.5 shrink-0 text-[var(--success)]" aria-hidden="true" />
          )}
          <span className="min-w-0 flex-1 truncate">
            {failed
              ? t("{count} steps · {failed} failed", { count: steps.length, failed: steps.filter((step) => step.status === "failed").length })
              : t("{count} steps completed", { count: steps.length })}
            {progress.elapsedSec != null ? ` · ${formatElapsed(progress.elapsedSec)}` : ""}
          </span>
          <ChevronDown className={cn("size-3.5 shrink-0 transition-transform", expanded && "rotate-180")} aria-hidden="true" />
        </button>
      ) : null}
      {expanded ? steps.map((step) => <StepRow key={step.id} step={step} now={now} />) : null}
      {live && !steps.some((step) => step.status === "in_progress" || step.status === "waiting") ? (
        <div className="step-row flex items-center gap-2.5 px-3 py-[9px] text-[var(--text-muted)]">
          <CircleDashed className="size-3.5 shrink-0" aria-hidden="true" />
          {t("Preparing the next step…")}
        </div>
      ) : null}
    </div>
  );
}

export function StepRow({ step, now }: { step: TurnStep; now: number }) {
  const output = stepOutput(step);
  const [open, setOpen] = useState(step.status === "failed");
  useEffect(() => {
    if (step.status === "failed") setOpen(true);
  }, [step.status]);
  const expandable = output.length > 0;
  const duration = formatStepDuration(stepDurationSec(step, now));

  return (
    <section className="step-row" data-status={step.status}>
      <button
        type="button"
        className="step-row-header"
        aria-expanded={expandable ? open : undefined}
        disabled={!expandable}
        onClick={() => setOpen((value) => !value)}
      >
        <StepStatusIcon status={step.status} />
        <span className="step-kind">{stepKindLabel(step)}</span>
        <span className="step-detail" title={stepDetail(step)}>
          {stepDetail(step)}
        </span>
        <span className="step-time">{duration}</span>
        {expandable ? (
          <ChevronDown
            className={cn("size-3.5 shrink-0 text-[var(--text-muted)] transition-transform", open && "rotate-180")}
            aria-hidden="true"
          />
        ) : (
          <span className="size-3.5 shrink-0" aria-hidden="true" />
        )}
      </button>
      {open && output ? <pre className="step-output">{output}</pre> : null}
    </section>
  );
}

export function StepStatusIcon({ status }: { status: AgentEventView["status"] }) {
  switch (status) {
    case "completed":
      return <Check className="size-3.5 shrink-0 text-[var(--success)]" role="img" aria-label={t("Done")} />;
    case "failed":
      return <CircleAlert className="size-3.5 shrink-0 text-[var(--danger)]" role="img" aria-label={t("Failed")} />;
    case "waiting":
      return <CirclePause className="size-3.5 shrink-0 text-[var(--warning)]" role="img" aria-label={t("Waiting")} />;
    default:
      return (
        <LoaderCircle
          className="size-3.5 shrink-0 animate-spin text-[var(--accent)] motion-reduce:animate-none"
          role="img"
          aria-label={t("In progress")}
        />
      );
  }
}

export function stepKindLabel(step: TurnStep) {
  switch (step.kind) {
    case "llm_thought":
      return t("Thinking");
    case "tool_call":
      return step.status === "waiting" ? t("Waiting for tool") : t("Tool call");
    case "tool_result":
      return t("Tool result");
    case "error":
      return t("Run failed");
    default:
      return t("Event");
  }
}

export function firstLine(text: string) {
  return text.trim().split("\n").find((line) => line.trim())?.trim() ?? "";
}

export function stepDetail(step: TurnStep) {
  if (step.kind === "tool_call" || step.kind === "tool_result") {
    const input = firstLine(step.content);
    return [step.toolName ?? step.title, input.length > 120 ? `${input.slice(0, 120)}…` : input].filter(Boolean).join("  ");
  }
  return firstLine(step.content) || step.title;
}

export function clip(text: string) {
  const trimmed = text.trim();
  return trimmed.length <= 4000 ? trimmed : `${trimmed.slice(0, 4000)}…`;
}

export function stepOutput(step: TurnStep) {
  const rationale = rationaleOf(step);
  const parts: string[] = [];
  if (rationale) parts.push(t("Reason: {text}", { text: rationale }));
  if (step.kind === "tool_call") {
    if (step.content.trim()) parts.push(step.result ? `${t("Input")}\n${clip(step.content)}` : clip(step.content));
    if (step.result?.content.trim()) parts.push(`${t("Output")}\n${clip(step.result.content)}`);
  } else if (step.content.trim()) {
    parts.push(clip(step.content));
  }
  return parts.join("\n\n");
}

export function rationaleOf(view: AgentEventView) {
  const metadata = view.metadata as { rationale?: unknown } | null;
  return view.kind === "tool_call" && typeof metadata?.rationale === "string" ? metadata.rationale.trim() : "";
}
