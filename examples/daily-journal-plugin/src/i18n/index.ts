import { zhCN } from "./zh-CN";
import { localizeHostText } from "@daily/plugin-sdk";
export function locale(): "en" | "zh-CN" {
  const lang = typeof document === "undefined" ? "zh-CN" : document.documentElement.lang || navigator.language;
  return lang.toLowerCase().startsWith("zh") ? "zh-CN" : "en";
}
export function message(error: unknown): string {
  const text = error instanceof Error ? error.message : String(error);
  return text in zhCN ? t(text as keyof typeof zhCN) : localizeHostText(text, locale());
}
export function t(key: keyof typeof zhCN, vars?: Record<string, string | number>): string {
  const text: string = locale() === "zh-CN" ? zhCN[key] : key;
  return text.replace(/\{(\w+)\}/g, (match, name: string) => String(vars?.[name] ?? match));
}
