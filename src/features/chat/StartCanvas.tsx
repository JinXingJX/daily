import {
  Check,
  ChevronRight,
  CircleAlert,
  FileText,
  FolderOpen,
  ShieldCheck
} from "lucide-react";
import { motion } from "motion/react";
import { type ReactNode } from "react";
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
        transition={{ duration: 0.18, ease: [0.22, 1, 0.36, 1] }}
        className="mx-auto flex min-h-full w-full max-w-[42.5rem] flex-col justify-center gap-7 py-12"
      >
        <div className="flex flex-col gap-1.5">
          <h1 className="m-0 text-[26px] font-semibold leading-[34px] tracking-[-0.015em] text-[var(--text-strong)]">
            {sessionTitle && sessionTitle !== t("New task") ? sessionTitle : t("What should the agent do?")}
          </h1>
          <p className="m-0 text-[14px] text-[var(--text-soft)]">
            {t("Tasks run locally. Every model request and tool call stays here for you to inspect.")}
          </p>
        </div>

        {composer}

        <section className="flex flex-col gap-1.5" aria-labelledby="starter-prompts">
          <h2 id="starter-prompts" className="panel-label mb-1">
            {t("Start here")}
          </h2>
          {STARTER_PROMPTS.map((item) => (
            <button key={item.label} className="prompt-row" type="button" onClick={() => onPrompt(item.prompt)}>
              <span className="icon-tile">
                <item.icon className="size-3.5" />
              </span>
              <span className="flex min-w-0 flex-1 flex-col">
                <span className="font-medium text-[var(--text-strong)]">{item.label}</span>
                <span className="truncate text-[12px] text-[var(--text-soft)]">{item.detail}</span>
              </span>
              <ChevronRight className="size-3.5 shrink-0 text-[var(--text-muted)]" />
            </button>
          ))}
        </section>

        <section aria-label={t("Readiness")} className="stat-grid grid-cols-1 sm:grid-cols-3">
          <ReadinessTile
            label={t("Model")}
            ready={readiness.modelReady}
            value={readiness.modelReady ? readiness.modelLabel : t("Sign in and refresh models")}
            action={readiness.modelReady ? null : { label: t("Go to sign-in"), onClick: () => onOpenSettings("auth") }}
          />
          <ReadinessTile
            label={t("Agents")}
            ready={readiness.agentCount > 0}
            value={readiness.agentCount ? t("{count} available", { count: readiness.agentCount }) : t("No agents yet")}
            action={{ label: readiness.agentCount ? t("Manage") : t("Create"), onClick: () => onOpenSettings("agents") }}
          />
          <ReadinessTile
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

export function ReadinessTile({
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
    <div className="flex flex-col gap-0.5 !bg-[var(--surface)]">
      <span className="stat-label">{label}</span>
      <span className="flex min-w-0 items-center gap-1.5 font-medium text-[var(--text-strong)]">
        {ready === true ? <Check className="size-[13px] shrink-0 text-[var(--success)]" aria-label={t("Ready")} /> : null}
        {ready === false ? <CircleAlert className="size-[13px] shrink-0 text-[var(--warning)]" aria-label={t("Not ready")} /> : null}
        <span className="min-w-0 truncate" title={value}>{value}</span>
        {action ? (
          <button
            type="button"
            className="focus-ring ml-auto shrink-0 rounded-[4px] px-1 text-[12px] font-normal text-[var(--accent-text)] hover:underline"
            onClick={action.onClick}
          >
            {action.label}
          </button>
        ) : null}
      </span>
    </div>
  );
}
