import { cn } from "@/lib/utils";
import type {
  PluginManifest,
  ProviderKind
} from "@/types";
import { type ReactNode } from "react";
import { t } from "@/i18n";

export const PLUGIN_SOURCE_LABELS: Record<PluginManifest["source"], string> = {
  get built_in() { return t("Built-in"); },
  get marketplace() { return t("Marketplace"); },
  get local_dev() { return t("Local development"); },
};

export const PROVIDER_LABELS: Record<ProviderKind, string> = {
  open_ai_compatible: "OpenAI compatible",
  anthropic_messages: "Anthropic messages",
  gemini_api: "Gemini API",
  codex_api_key: "Codex API (Responses)",
  codex_login: "Codex Login",
  antigravity_login: "Antigravity Login",
};

export const MAX_SYSTEM_PROMPT_BYTES = 256 * 1024;

export function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  return `${(bytes / 1024).toFixed(bytes < 10 * 1024 ? 1 : 0)} KB`;
}

export function SettingsHeader({
  title,
  detail,
  action,
}: {
  title: string;
  detail: string;
  action?: ReactNode;
}) {
  return (
    <div className="mb-6 flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
      <div className="min-w-0">
        <h1 className="m-0 text-[20px] font-semibold leading-7 tracking-[-0.01em] text-[var(--text-strong)]">{title}</h1>
        <p className="mb-0 mt-0.5 max-w-2xl text-[13px] leading-5 text-[var(--text-soft)]">{detail}</p>
      </div>
      {action ? <div className="shrink-0">{action}</div> : null}
    </div>
  );
}

export function SettingRow({
  title,
  detail,
  children,
}: {
  title: string;
  detail: string;
  children: ReactNode;
}) {
  return (
    <div className="settings-row">
      <div className="min-w-0">
        <div className="text-[13px] font-medium text-[var(--text-strong)]">{title}</div>
        <div className="mt-0.5 break-all text-[12px] leading-5 text-[var(--text-muted)]">{detail}</div>
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  );
}

export function Field({
  label,
  className,
  children,
}: {
  label: string;
  className?: string;
  children: ReactNode;
}) {
  return (
    <label className={cn("grid gap-2", className)}>
      <span className="text-[13px] font-medium text-[var(--text-soft)]">{label}</span>
      {children}
    </label>
  );
}
