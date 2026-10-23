import { t } from "@/i18n";

/** Title the runtime gives a session until its first message names it (`DEFAULT_SESSION_TITLE`). */
export const DEFAULT_SESSION_TITLE = "New task";

/** A session title for display: the untouched default is shown in the current language. */
export function displaySessionTitle(title: string | null | undefined): string {
  return !title || title === DEFAULT_SESSION_TITLE ? t("New task") : title;
}

export function isDefaultSessionTitle(title: string | null | undefined): boolean {
  return !title || title === DEFAULT_SESSION_TITLE;
}
