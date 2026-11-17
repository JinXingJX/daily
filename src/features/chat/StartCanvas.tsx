import {
  Check,
  CircleAlert,
  FileText,
  FolderOpen,
  ShieldCheck
} from "lucide-react";
import { motion } from "motion/react";
import { type ReactNode } from "react";
import { DailyMark } from "@/components/app/DailyMark";
import { ChatReadiness, SettingsTarget } from "./chatTypes";
import { t } from "@/i18n";

export const STARTER_PROMPTS = [
  {
    get label() { return t("Map the project"); },
    get detail() { return t("Outline the structure, find the active screens, and suggest next steps"); },
    get prompt() { return t("Outline this project's structure, find the frontend screens that are in active use, and suggest concrete next implementation steps."); },
    icon: FolderOpen,
  },
  {
    get label() { return t("Plan a change"); },
    get detail() { return t("Write a short implementation plan with the files to change and how to verify"); },
    get prompt() { return t("Write a short implementation plan for the current change, listing the files to modify and the verification steps."); },
    icon: FileText,
  },
  {
    get label() { return t("Review the current state"); },
    get detail() { return t("Review the workspace and point out the UI or interaction issues to fix first"); },
    get prompt() { return t("Review the current state of the workspace and point out the UI or interaction issues that most need fixing."); },
    icon: ShieldCheck,
  },
];

export function StartCanvas({
  sessionTitle,
  readiness,
  composer,
  onPrompt,
  onOpenSettings,
}: {
  sessionTitle: string | null;
  readiness: ChatReadiness;
  composer: ReactNode;
  onPrompt: (prompt: string) => void;
  onOpenSettings: (target: SettingsTarget) => void;
}) {
  return (
    <div className="min-h-0 flex-1 overflow-auto px-6 sm:px-8">
      <motion.div
        initial={{ opacity: 0, y: 8 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.2, ease: [0.25, 1, 0.5, 1] }}
        className="mx-auto flex min-h-full w-full max-w-[42rem] flex-col justify-center gap-7 pb-24 pt-8"
      >
        <div className="flex flex-col items-center gap-2.5 text-center">
          <div className="flex items-center gap-3.5">
            <DailyMark className="size-8 shrink-0 text-[var(--accent)]" />
            <h2 className="display-title display-title-lg">
              {sessionTitle && sessionTitle !== t("New task") ? sessionTitle : t("What should the agent do?")}
            </h2>
          </div>
          <p className="m-0 text-[14px] leading-[22px] text-[var(--text-muted)]">
            {t("Tasks run locally. Every model request and tool call stays here for you to inspect.")}
          </p>
        </div>

        {composer}

        <section className="flex flex-wrap justify-center gap-2" aria-labelledby="starter-prompts">
          <h3 id="starter-prompts" className="sr-only">
            {t("Start here")}
          </h3>
          {STARTER_PROMPTS.map((item) => (
            <button
              key={item.label}
              className="chip-button chip-button-lg"
              type="button"
              title={item.detail}
              onClick={() => onPrompt(item.prompt)}
            >
              <item.icon className="size-[15px]" />
              {item.label}
            </button>
          ))}
        </section>

        <section aria-label={t("Readiness")} className="readiness-line">
          <ReadinessItem
            label={t("Model")}
            ready={readiness.modelReady}
            value={readiness.modelReady ? readiness.modelLabel : t("Sign in and refresh models")}
            action={readiness.modelReady ? null : { label: t("Go to sign-in"), onClick: () => onOpenSettings("auth") }}
          />
          <ReadinessItem
            label={t("Agents")}
            ready={readiness.agentCount > 0}
            value={readiness.agentCount ? t("{count} available", { count: readiness.agentCount }) : t("No agents yet")}
            action={{ label: readiness.agentCount ? t("Manage") : t("Create"), onClick: () => onOpenSettings("agents") }}
          />
          <ReadinessItem
            label={t("Plugins")}
            ready={null}
            value={readiness.pluginNames.length ? readiness.pluginNames.join(", ") : t("No plugins enabled")}
            action={{ label: t("Manage"), onClick: () => onOpenSettings("plugins") }}
          />
        </section>
      </motion.div>
    </div>
  );
}

/** One fact about what a new task can use, with the place to change it. */
export function ReadinessItem({
  label,
  ready,
  value,
  action,
}: {
  label: string;
  ready: boolean | null;
  value: string;
  action: { label: string; onClick: () => void } | null;
}) {
  return (
    <span className="readiness-item">
      {ready === true ? <Check className="size-[13px] shrink-0 text-[var(--success)]" strokeWidth={2.25} role="img" aria-label={t("Ready")} /> : null}
      {ready === false ? <CircleAlert className="size-[13px] shrink-0 text-[var(--warning)]" role="img" aria-label={t("Not ready")} /> : null}
      <span>{label}</span>
      <span className="max-w-[16rem] truncate text-[var(--text-soft)]" title={value}>{value}</span>
      {action ? (
        <button type="button" className="text-link focus-ring" onClick={action.onClick}>
          {action.label}
        </button>
      ) : null}
    </span>
  );
}
