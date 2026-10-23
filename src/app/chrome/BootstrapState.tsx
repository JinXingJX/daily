import { type BackendStatus } from "@/platform/desktop";
import { t } from "@/i18n";

export function BootstrapState({
  error,
  backendStatus,
  onRetry,
}: {
  error: string | null;
  backendStatus: BackendStatus;
  onRetry: () => void;
}) {
  return (
    <div className="min-h-[100dvh] bg-[var(--app-shell-bg)]">
      <div className="app-drag h-12 border-b border-[var(--line)] bg-[var(--chrome)]" />
      {error ? (
        <section className="mx-auto mt-28 w-full max-w-xl border-y border-[var(--line)] bg-transparent py-6" role="alert">
          <h1 className="text-lg font-semibold text-[var(--text-strong)]">{t("Workspace failed to start")}</h1>
          <p className="mt-2 text-sm leading-6 text-[var(--text-muted)]">
            {t("The workspace could not load its initial state.")}
          </p>
          <div className="mt-4 border-y border-[var(--line)] bg-[var(--surface)] px-1 py-4 font-mono text-sm leading-6 text-[var(--text)]">
            {error}
          </div>
          <div className="mt-4 text-sm text-[var(--text-muted)]">
            {backendStatus.canRetry ? (
              <button className="rounded-[var(--radius-control)] border border-[var(--line-strong)] px-3 py-2 font-semibold text-[var(--text-strong)] hover:bg-[var(--surface-muted)]" type="button" onClick={onRetry}>
                {t("Restart local runtime")}
              </button>
            ) : t("The local runtime is still starting; Daily reconnects automatically.")}
          </div>
        </section>
      ) : (
        <div className="grid h-[calc(100dvh-3rem)] grid-cols-[288px_minmax(0,1fr)]">
          <aside className="border-r border-[var(--line)] bg-[var(--app-sidebar-bg)] p-4">
            <div className="skeleton h-9 w-32 rounded-[var(--radius-control)]" />
            <div className="skeleton mt-5 h-8 w-full rounded-[var(--radius-control)]" />
            <div className="mt-6 space-y-2">
              <div className="skeleton h-7 w-[82%] rounded-[var(--radius-control)]" />
              <div className="skeleton h-7 w-[64%] rounded-[var(--radius-control)]" />
              <div className="skeleton h-7 w-[74%] rounded-[var(--radius-control)]" />
            </div>
          </aside>
          <main className="p-5">
            <div className="skeleton h-10 w-full rounded-[var(--radius-control)]" />
            <div className="mx-auto mt-24 max-w-3xl space-y-4">
              <div className="skeleton h-4 w-40 rounded-full" />
              <div className="skeleton h-20 w-full rounded-[var(--radius-panel)]" />
              <div className="skeleton ml-auto h-16 w-3/4 rounded-[var(--radius-panel)]" />
            </div>
          </main>
        </div>
      )}
    </div>
  );
}
