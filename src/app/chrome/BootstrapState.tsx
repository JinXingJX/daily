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
    <div className="flex min-h-[100dvh] bg-[var(--app-shell-bg)]">
      {error ? (
        <div className="flex min-w-0 flex-1 flex-col">
          <div className="app-drag h-[var(--titlebar-height)] shrink-0" />
          <section className="mx-auto mt-24 w-full max-w-xl px-6" role="alert">
            <h1 className="display-title">{t("Workspace failed to start")}</h1>
            <p className="mb-0 mt-2 text-[14px] leading-[22px] text-[var(--text-soft)]">
              {t("The workspace could not load its initial state.")}
            </p>
            <div className="mono mt-5 rounded-[var(--radius-panel)] border border-[var(--line)] bg-[var(--surface)] px-4 py-3 text-[13px] leading-[21px] text-[var(--text)]">
              {error}
            </div>
            <div className="mt-5 text-[14px] text-[var(--text-muted)]">
              {backendStatus.canRetry ? (
                <button
                  className="focus-ring h-9 rounded-[var(--radius-control)] border border-[var(--primary)] bg-[var(--primary)] px-3.5 font-medium text-[var(--primary-contrast)] hover:bg-[var(--primary-hover)]"
                  type="button"
                  onClick={onRetry}
                >
                  {t("Restart local runtime")}
                </button>
              ) : t("The local runtime is still starting; Daily reconnects automatically.")}
            </div>
          </section>
        </div>
      ) : (
        <>
          <aside className="flex w-[264px] shrink-0 flex-col border-r border-[var(--line)] bg-[var(--app-sidebar-bg)]">
            <div className="app-drag h-[var(--titlebar-height)] shrink-0" />
            <div className="flex flex-col gap-2 px-4 pt-2">
              <div className="skeleton h-6 w-28 rounded-[var(--radius-control)]" />
              <div className="skeleton h-6 w-40 rounded-[var(--radius-control)]" />
              <div className="skeleton mt-6 h-5 w-[82%] rounded-[var(--radius-control)]" />
              <div className="skeleton h-5 w-[64%] rounded-[var(--radius-control)]" />
              <div className="skeleton h-5 w-[74%] rounded-[var(--radius-control)]" />
            </div>
          </aside>
          <main className="flex min-w-0 flex-1 flex-col">
            <div className="app-drag h-[var(--titlebar-height)] shrink-0" />
            <div className="mx-auto flex w-full max-w-[42rem] flex-1 flex-col justify-center gap-6 px-6 pb-24">
              <div className="skeleton mx-auto h-9 w-72 rounded-[var(--radius-control)]" />
              <div className="skeleton h-36 w-full rounded-[var(--radius-composer)]" />
              <div className="flex justify-center gap-2">
                <div className="skeleton h-[34px] w-24 rounded-[var(--radius-chip)]" />
                <div className="skeleton h-[34px] w-24 rounded-[var(--radius-chip)]" />
                <div className="skeleton h-[34px] w-24 rounded-[var(--radius-chip)]" />
              </div>
            </div>
          </main>
        </>
      )}
    </div>
  );
}
