import React from "react";
import ReactDOM from "react-dom/client";
import "./index.css";
import App from "./App";
import { useLocale } from "./i18n";

type ErrorBoundaryState = {
  error: Error | null;
};

class AppErrorBoundary extends React.Component<React.PropsWithChildren, ErrorBoundaryState> {
  state: ErrorBoundaryState = {
    error: null,
  };

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { error };
  }

  componentDidCatch(error: Error, errorInfo: React.ErrorInfo) {
    console.error("Daily frontend crashed", error, errorInfo);
  }

  render() {
    if (this.state.error) {
      return (
        <div className="grid min-h-[100dvh] place-items-center px-6">
          <div className="w-full max-w-2xl rounded-[var(--radius-panel)] border border-[var(--danger-line)] bg-[var(--danger-soft)] p-6 shadow-[var(--shadow-card)]">
            <div className="text-sm font-medium text-[var(--danger)]">Frontend Crash</div>
            <h1 className="mt-3 text-2xl font-semibold text-[var(--text-strong)]">The UI Hit a Runtime Error</h1>
            <p className="mt-3 text-sm leading-6 text-[var(--text)]">
              The screen did not render because React threw while mounting or updating the app.
            </p>
            <div className="mt-5 rounded-[var(--radius-control)] border border-[var(--line)] bg-[var(--surface)] px-3 py-3 text-sm font-medium leading-6 text-[var(--text-strong)]">
              {this.state.error.message}
            </div>
            <pre className="mt-5 overflow-x-auto rounded-[var(--radius-control)] border border-[var(--line)] bg-[var(--surface-inset)] px-3 py-3 text-sm leading-6 text-[var(--text-strong)]">
              {this.state.error.stack || this.state.error.message}
            </pre>
          </div>
        </div>
      );
    }

    return this.props.children;
  }
}

/** Remounts the app when the language changes so every `t()` call re-runs. */
function LocaleRoot() {
  const locale = useLocale();
  React.useEffect(() => {
    document.documentElement.lang = locale;
  }, [locale]);
  return <App key={locale} />;
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <AppErrorBoundary>
      <LocaleRoot />
    </AppErrorBoundary>
  </React.StrictMode>,
);
