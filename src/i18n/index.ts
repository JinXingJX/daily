// UI strings are written in English and used as message keys: `t("Retry")`. Other languages
// map those keys in their catalog (see zh-CN.ts), so a key missing from a catalog fails to
// type-check. `{name}` placeholders are filled from `vars`.
//
// The renderer remounts its tree when the language changes (see LocaleRoot), so `t` can be
// called anywhere, including module-level helpers, without subscribing to a store.
import { useSyncExternalStore } from "react";
import { zhCN } from "./zh-CN";

export type Locale = "en" | "zh-CN";
export type LocalePreference = "system" | Locale;
export type MessageKey = keyof typeof zhCN;

const STORAGE_KEY = "daily-locale";
const catalogs: Record<Exclude<Locale, "en">, Record<MessageKey, string>> = { "zh-CN": zhCN };

function systemLocale(): Locale {
  const languages = typeof navigator === "undefined" ? [] : navigator.languages ?? [navigator.language];
  return languages.some((language) => language?.toLowerCase().startsWith("zh")) ? "zh-CN" : "en";
}

function readPreference(): LocalePreference {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored === "en" || stored === "zh-CN" || stored === "system") return stored;
  } catch {
    // Storage can be unavailable; fall back to the system language.
  }
  return "system";
}

let preference: LocalePreference = readPreference();
let locale: Locale = preference === "system" ? systemLocale() : preference;
const listeners = new Set<() => void>();

export function currentLocale(): Locale {
  return locale;
}

export function localePreference(): LocalePreference {
  return preference;
}

export function setLocalePreference(next: LocalePreference) {
  preference = next;
  locale = next === "system" ? systemLocale() : next;
  try {
    localStorage.setItem(STORAGE_KEY, next);
  } catch {
    // Not persisted; the choice still applies to this window.
  }
  document.documentElement.lang = locale;
  listeners.forEach((listener) => listener());
}

/** Test hook: switch language without touching storage. */
export function setLocaleForTests(next: Locale) {
  locale = next;
}

export function t(key: MessageKey, vars?: Record<string, string | number>): string {
  const template = locale === "en" ? key : catalogs[locale][key] ?? key;
  if (!vars) return template;
  return template.replace(/\{(\w+)\}/g, (whole, name: string) => (name in vars ? String(vars[name]) : whole));
}

/** Re-renders when the language changes; LocaleRoot keys the app tree with it. */
export function useLocale(): Locale {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => locale,
  );
}
