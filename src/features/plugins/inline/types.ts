import { type ReactNode } from "react";

/** A renderer for one `daily-plugin` fenced block contributed by a plugin. */
export type PluginSpec<T> = {
  pluginId: string;
  renderer: string;
  legacyIdentifier?: string;
  label: string;
  icon: ReactNode;
  validate: (payload: unknown) => payload is T;
  render: (payload: T) => ReactNode;
  skeleton: ReactNode;
  chrome?: "shell" | "bare";
};

export type PluginRenderResult = {
  content: ReactNode;
  ok: boolean;
};

export type DailyPluginEnvelope = {
  pluginId: string;
  renderer: string;
  payload: unknown;
};

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}
