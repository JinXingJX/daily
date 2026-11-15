import { RotateCcw, Sparkles } from "lucide-react";
import type { PluginTheme, PluginViewContext } from "@daily/plugin-sdk";
import { useEffect, useRef, useState, useSyncExternalStore, type ReactNode } from "react";
import { Kicker } from "@/components/app/Section";
import { Button } from "@/components/ui/button";
import type { PluginMainView } from "@/features/workspace/Sidebar";
import { t } from "@/i18n";
import { pluginRuntime } from "./runtime";

type PluginViewsProps = {
  /** Views of enabled plugins. */
  views: PluginMainView[];
  /** The view on screen, or null while a core page is shown. */
  active: { pluginId: string; viewId: string } | null;
  theme: PluginTheme;
};

const viewKey = (pluginId: string, viewId: string) => `${pluginId}:${viewId}`;

/**
 * Hosts plugin views. A view mounts the first time it is opened and then stays mounted, hidden
 * while another page is shown, so switching back is instant and keeps its state.
 */
export function PluginViews({ views, active, theme }: PluginViewsProps) {
  const [opened, setOpened] = useState<string[]>([]);
  const activeKey = active ? viewKey(active.pluginId, active.viewId) : null;
  if (activeKey && !opened.includes(activeKey)) setOpened([...opened, activeKey]);

  const available = new Map(views.map((item) => [viewKey(item.plugin.identifier, item.view.id), item]));
  return opened.map((key) => {
    const item = available.get(key);
    return item ? <PluginViewSurface key={key} item={item} active={key === activeKey} theme={theme} /> : null;
  });
}

type SurfaceState = { status: "loading" } | { status: "ready" } | { status: "error"; message: string };

/** What the mounted view's context reads and the host updates. */
type LiveView = {
  context: PluginViewContext | null;
  container: HTMLElement | null;
  focused: HTMLElement | null;
  themeListeners: Set<(theme: PluginTheme) => void>;
  visibilityListeners: Set<(visible: boolean) => void>;
};

function PluginViewSurface({ item, active, theme }: { item: PluginMainView; active: boolean; theme: PluginTheme }) {
  const { plugin, view } = item;
  const hostRef = useRef<HTMLDivElement | null>(null);
  const [state, setState] = useState<SurfaceState>({ status: "loading" });
  const [attempt, setAttempt] = useState(0);
  const generation = useSyncExternalStore(pluginRuntime.subscribe, () => pluginRuntime.generation(plugin.identifier));
  const latest = useRef({ plugin, theme, active });
  latest.current = { plugin, theme, active };
  const live = useRef<LiveView>({ context: null, container: null, focused: null, themeListeners: new Set(), visibilityListeners: new Set() });

  useEffect(() => {
    const element = hostRef.current;
    if (!element) return;
    const root = element.shadowRoot ?? element.attachShadow({ mode: "open" });
    const container = document.createElement("div");
    container.id = "root";
    container.tabIndex = -1;
    root.adoptedStyleSheets = [baseSheet()];
    root.replaceChildren(container);
    const current = live.current;
    const rememberFocus = (event: Event) => {
      if (event.target instanceof HTMLElement) current.focused = event.target;
    };
    root.addEventListener("focusin", rememberFocus);
    let disposed = false;
    const controller = new AbortController();
    let unmount: void | (() => void);
    setState({ status: "loading" });

    pluginRuntime
      .load(latest.current.plugin)
      .then(async (instance) => {
        if (disposed) return;
        const definition = instance.views[view.id];
        if (!definition) throw new Error(t("The plugin does not define the view “{view}”.", { view: view.id }));
        const context: PluginViewContext = {
          viewId: view.id,
          root,
          theme: latest.current.theme,
          visible: latest.current.active,
          signal: controller.signal,
          addStyles(css) {
            let sheet = instance.styles.get(css);
            if (!sheet) {
              sheet = new CSSStyleSheet();
              sheet.replaceSync(css);
              instance.styles.set(css, sheet);
            }
            if (!root.adoptedStyleSheets.includes(sheet)) root.adoptedStyleSheets = [...root.adoptedStyleSheets, sheet];
          },
          onThemeChange: (listener) => subscribe(current.themeListeners, listener),
          onVisibilityChange: (listener) => subscribe(current.visibilityListeners, listener),
        };
        current.context = context;
        current.container = container;
        unmount = await definition.mount(container, context);
        if (disposed) {
          if (typeof unmount === "function") unmount();
          return;
        }
        setState({ status: "ready" });
        if (latest.current.active) container.focus({ preventScroll: true });
      })
      .catch((error) => {
        if (!disposed) setState({ status: "error", message: error instanceof Error ? error.message : String(error) });
      });

    return () => {
      disposed = true;
      controller.abort();
      try {
        if (typeof unmount === "function") unmount();
      } catch (error) {
        console.error(`Plugin view ${view.id} failed to unmount cleanly`, error);
      }
      current.context = null;
      current.container = null;
      current.focused = null;
      root.removeEventListener("focusin", rememberFocus);
      current.themeListeners.clear();
      current.visibilityListeners.clear();
      root.replaceChildren();
      root.adoptedStyleSheets = [];
    };
  }, [plugin.identifier, view.id, generation, attempt]);

  useEffect(() => {
    if (hostRef.current) hostRef.current.dataset.dailyTheme = theme;
    const { context, themeListeners } = live.current;
    if (!context || context.theme === theme) return;
    context.theme = theme;
    notify(themeListeners, theme);
  }, [theme]);

  useEffect(() => {
    const { context, container, focused, visibilityListeners } = live.current;
    if (!context || context.visible === active) return;
    context.visible = active;
    notify(visibilityListeners, active);
    // Keyboard events reach the view through its container, like a page that has focus.
    if (active && container && !container.contains(context.root.activeElement)) {
      (focused?.isConnected ? focused : container).focus({ preventScroll: true });
    }
  }, [active]);

  return (
    <div className={active ? "relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden" : "hidden"}>
      <div ref={hostRef} data-plugin-view={viewKey(plugin.identifier, view.id)} data-daily-theme={theme} className="relative block min-h-0 min-w-0 flex-1" />
      {state.status === "ready" ? null : (
        <div className="absolute inset-0 flex bg-[var(--surface)]">
          {state.status === "loading" ? (
            <PluginViewNotice item={item} title={t("Loading plugin")} detail={t("Loading the plugin module.")} />
          ) : (
            <PluginViewNotice
              item={item}
              title={t("Plugin view failed to load")}
              detail={state.message}
              action={
                <Button
                  size="sm"
                  variant="secondary"
                  onClick={() => {
                    // A failed import is cached with the plugin instance; start over.
                    pluginRuntime.unload(plugin.identifier);
                    setAttempt((value) => value + 1);
                  }}
                >
                  <RotateCcw className="size-4" />
                  {t("Retry")}
                </Button>
              }
            />
          )}
        </div>
      )}
    </div>
  );
}

function subscribe<T>(listeners: Set<T>, listener: T) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function notify<T>(listeners: Set<(value: T) => void>, value: T) {
  listeners.forEach((listener) => {
    try {
      listener(value);
    } catch (error) {
      console.error(error);
    }
  });
}

let sharedBaseSheet: CSSStyleSheet | null = null;

// The view starts from a clean slate like a new document: nothing inherits from Daily's page.
// `#root` is the view's viewport — the containing block of fixed elements and the
// `daily-view` size container, so `cqw`/`cqh` and `@container daily-view (…)` measure the view.
function baseSheet() {
  if (!sharedBaseSheet) {
    sharedBaseSheet = new CSSStyleSheet();
    sharedBaseSheet.replaceSync(`
      :host { all: initial; color-scheme: light; background: Canvas; color: CanvasText; font-family: -apple-system, BlinkMacSystemFont, "PingFang SC", "Hiragino Sans GB", "Segoe UI", sans-serif; }
      :host(:where([data-daily-theme="dark"])) { color-scheme: dark; }
      #root { position: absolute; inset: 0; overflow: hidden; outline: none; contain: layout paint style; container: daily-view / size; }
      button, input, textarea, select { font: inherit; }
    `);
  }
  return sharedBaseSheet;
}

function PluginViewNotice({ item, title, detail, action }: { item: PluginMainView; title: string; detail: string; action?: ReactNode }) {
  return (
    <div className="grid min-h-0 flex-1 place-items-center p-6">
      <section className="w-full max-w-xl border-y border-[var(--line)] bg-transparent py-6">
        <div className="flex items-start gap-3">
          <div className="flex size-10 shrink-0 items-center justify-center rounded-[var(--radius-control)] bg-[var(--surface-muted)] text-[var(--text-soft)]">
            <Sparkles className="size-5" />
          </div>
          <div className="min-w-0">
            <Kicker>{item.plugin.identifier} / {item.view.id}</Kicker>
            <h1 className="mt-2 text-lg font-semibold text-[var(--text-strong)]">{title}</h1>
            <p className="mt-2 whitespace-pre-line break-words text-sm leading-6 text-[var(--text-muted)]">{detail}</p>
            {action ? <div className="mt-4">{action}</div> : null}
          </div>
        </div>
      </section>
    </div>
  );
}
