import { RotateCcw, ShieldAlert, Sparkles } from "lucide-react";
import { bridgeScript } from "@daily/plugin-sdk/bridge";
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Kicker } from "@/components/app/Section";
import { Button } from "@/components/ui/button";
import { DesktopCommandError, invoke, onAppEvent } from "@/platform/desktop";
import type { PluginManifest, PluginViewContribution } from "@/types";
import { t } from "@/i18n";

type PluginViewHostProps = {
  plugin: PluginManifest;
  view: PluginViewContribution;
  theme: "light" | "dark";
  onDirtyChange?: (dirty: boolean) => void;
  background?: boolean;
};

type PluginViewAsset = {
  source: string;
};

const PLUGIN_AGENT_PROGRESS_EVENT = "plugin://agent-progress";

type PluginFrameRequest = {
  source: "daily-plugin";
  id: string;
  method: string;
  payload?: unknown;
};

export function PluginViewHost({ plugin, view, theme, onDirtyChange, background = false }: PluginViewHostProps) {
  if (view.runtime !== "iframe") {
    return (
      <PluginHostState
        plugin={plugin}
        view={view}
        title={t("Unsupported plugin view")}
        detail={t("The host has no runtime named “{runtime}”.", { runtime: view.runtime })}
        unsupported
      />
    );
  }

  return <IframePluginView plugin={plugin} view={view} theme={theme} onDirtyChange={onDirtyChange} background={background} />;
}

function IframePluginView({ plugin, view, theme, onDirtyChange, background = false }: PluginViewHostProps) {
  const iframeRef = useRef<HTMLIFrameElement | null>(null);
  const [assetSource, setAssetSource] = useState<string | null>(null);
  const [loadKey, setLoadKey] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [loadingAsset, setLoadingAsset] = useState(false);
  const inFlightByFrame = useRef(new WeakMap<Window, Set<string>>());

  const scriptUrl = useMemo(() => buildScriptUrl(view), [view]);

  useEffect(() => {
    let disposed = false;
    if (scriptUrl) {
      setAssetSource(null);
      setError(null);
      setLoadingAsset(false);
      return;
    }
    setLoadingAsset(true);
    setError(null);
    invoke<PluginViewAsset>("plugin_view_asset", {
      input: {
        pluginId: plugin.identifier,
        viewId: view.id,
      },
    })
      .then((asset) => {
        if (!disposed) {
          setAssetSource(asset.source);
        }
      })
      .catch((nextError) => {
        if (!disposed) {
          setError(errorMessage(nextError, "Failed to load plugin view asset."));
        }
      })
      .finally(() => {
        if (!disposed) {
          setLoadingAsset(false);
        }
      });
    return () => {
      disposed = true;
    };
  }, [plugin.identifier, scriptUrl, view.id, loadKey]);

  const handleMessage = useCallback(
    (event: MessageEvent) => {
      if (event.source !== iframeRef.current?.contentWindow) {
        return;
      }
      const sourceWindow = event.source as Window;
      const status = event.data as { source?: unknown; type?: unknown; message?: unknown } | null;
      if (status?.source === "daily-plugin-status" && status.type === "load-error") {
        const message = typeof status.message === "string" ? status.message.slice(0, 500) : t("The plugin failed to load.");
        setError(
          scriptUrl
            ? `${message}\n${t("Make sure the dev server {origin} is running (npm run dev in the plugin folder), or reinstall the plugin without a dev URL to load its built files.", { origin: new URL(scriptUrl).origin })}`
            : message,
        );
        return;
      }
      const data = event.data as Partial<PluginFrameRequest>;
      if (!data || data.source !== "daily-plugin" || typeof data.id !== "string" || typeof data.method !== "string") {
        return;
      }
      if (data.id.length > 128 || data.method.length > 128 || serializedSize(data) > 16 * 1024 * 1024) {
        sourceWindow.postMessage(
          { source: "daily-host", id: data.id.slice(0, 128), ok: false, error: "Plugin request is invalid or too large." },
          "*",
        );
        return;
      }
      if (data.method === "host.setDirty") {
        const dirty = pluginDirtyValue(data.payload);
        if (dirty === null) {
          sourceWindow.postMessage(
            { source: "daily-host", id: data.id, ok: false, error: "Plugin dirty state must be a boolean." },
            "*",
          );
          return;
        }
        onDirtyChange?.(dirty);
        sourceWindow.postMessage({ source: "daily-host", id: data.id, ok: true, payload: {} }, "*");
        return;
      }
      let requestIds = inFlightByFrame.current.get(sourceWindow);
      if (!requestIds) {
        requestIds = new Set();
        inFlightByFrame.current.set(sourceWindow, requestIds);
      }
      if (requestIds.has(data.id) || requestIds.size >= 32) {
        sourceWindow.postMessage(
          { source: "daily-host", id: data.id, ok: false, error: "Plugin has too many concurrent or duplicate requests." },
          "*",
        );
        return;
      }
      requestIds.add(data.id);

      void invoke("plugin_host_rpc", {
        input: {
          pluginId: plugin.identifier,
          viewId: view.id,
          method: data.method,
          payload: data.payload ?? {},
        },
      })
        .then((payload) => {
          sourceWindow.postMessage(
            {
              source: "daily-host",
              id: data.id,
              ok: true,
              payload,
            },
            "*",
          );
        })
        .catch((nextError) => {
          sourceWindow.postMessage(
            {
              source: "daily-host",
              id: data.id,
              ok: false,
              error: errorMessage(nextError, "Plugin host request failed."),
              code: nextError instanceof DesktopCommandError ? nextError.code : "host_error",
            },
            "*",
          );
        })
        .finally(() => requestIds.delete(data.id as string));
    },
    [onDirtyChange, plugin.identifier, scriptUrl, view.id],
  );

  useEffect(() => {
    window.addEventListener("message", handleMessage);
    return () => window.removeEventListener("message", handleMessage);
  }, [handleMessage]);

  useEffect(() => () => onDirtyChange?.(false), [onDirtyChange, plugin.identifier, view.id]);

  // Forwards live agent progress for this view; requests carry a streamId the plugin listens on.
  useEffect(
    () =>
      onAppEvent((message) => {
        if (message.event !== PLUGIN_AGENT_PROGRESS_EVENT) return;
        const payload = message.payload as { pluginId?: unknown; viewId?: unknown } | null;
        if (payload?.pluginId !== plugin.identifier || payload.viewId !== view.id) return;
        iframeRef.current?.contentWindow?.postMessage(
          { source: "daily-host", type: "event", event: "agent.progress", payload },
          "*",
        );
      }),
    [plugin.identifier, view.id],
  );

  const srcDoc = useMemo(() => {
    if (!scriptUrl && !assetSource) {
      return "";
    }
    return buildSrcDoc(plugin, view, theme, scriptUrl, assetSource);
  }, [assetSource, plugin, scriptUrl, theme, view]);

  if (error) {
    if (background) {
      return null;
    }
    return (
      <PluginHostState
        plugin={plugin}
        view={view}
        title={t("Plugin view failed to load")}
        detail={error}
        action={
          <Button size="sm" variant="secondary" onClick={() => setLoadKey((value) => value + 1)}>
            <RotateCcw className="size-4" />
            {t("Retry")}
          </Button>
        }
      />
    );
  }

  if (loadingAsset || !srcDoc) {
    if (background) {
      return null;
    }
    return (
      <PluginHostState
        plugin={plugin}
        view={view}
        title={t("Loading plugin")}
        detail={t("Preparing the sandbox and host bridge.")}
      />
    );
  }

  if (background) {
    return (
      <iframe
        key={loadKey}
        ref={iframeRef}
        className="h-px w-px border-0"
        sandbox="allow-scripts"
        srcDoc={srcDoc}
        tabIndex={-1}
        title={`${plugin.name}: ${view.title} background worker`}
      />
    );
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
      <iframe
        key={loadKey}
        ref={iframeRef}
        className="min-h-0 flex-1 border-0 bg-[var(--surface-inset)]"
        sandbox="allow-scripts"
        srcDoc={srcDoc}
        title={`${plugin.name}: ${view.title}`}
      />
    </div>
  );
}

function PluginHostState({
  plugin,
  view,
  title,
  detail,
  action,
  unsupported = false,
}: {
  plugin: PluginManifest;
  view: PluginViewContribution;
  title: string;
  detail: string;
  action?: ReactNode;
  unsupported?: boolean;
}) {
  return (
    <div className="grid min-h-0 flex-1 place-items-center p-6">
      <section className="w-full max-w-xl border-y border-[var(--line)] bg-transparent py-6">
        <div className="flex items-start gap-3">
          <div className="flex size-10 shrink-0 items-center justify-center rounded-[var(--radius-control)] bg-[var(--surface-muted)] text-[var(--text-soft)]">
            {unsupported ? <ShieldAlert className="size-5" /> : <Sparkles className="size-5" />}
          </div>
          <div className="min-w-0">
            <Kicker>{plugin.identifier} / {view.id}</Kicker>
            <h1 className="mt-2 text-lg font-semibold text-[var(--text-strong)]">{title}</h1>
            <p className="mt-2 whitespace-pre-line break-words text-sm leading-6 text-[var(--text-muted)]">{detail}</p>
            {action ? <div className="mt-4">{action}</div> : null}
          </div>
        </div>
      </section>
    </div>
  );
}

function buildScriptUrl(view: PluginViewContribution) {
  if (!view.devUrl) {
    return null;
  }
  try {
    const base = new URL(view.devUrl);
    if (!matchesHttpProtocol(base) || base.username || base.password) return null;
    const entry = view.devEntry ?? view.entry;
    const script = entry ? new URL(entry.replace(/^\//, ""), `${base.toString().replace(/\/$/, "")}/`) : base;
    return matchesHttpProtocol(script) && !script.username && !script.password ? script.toString() : null;
  } catch {
    return null;
  }
}

function buildSrcDoc(
  plugin: PluginManifest,
  view: PluginViewContribution,
  theme: "light" | "dark",
  scriptUrl: string | null,
  assetSource: string | null,
) {
  const nonce = crypto.randomUUID().replace(/-/g, "");
  const sdk = bridgeScript(plugin.identifier, view.id);
  const csp = buildFrameCsp(scriptUrl, nonce);
  const loader = scriptUrl
    ? `${buildDevPreamble(scriptUrl, nonce)}
    <script nonce="${nonce}" type="module" src="${escapeHtml(scriptUrl)}"></script>`
    : `<script nonce="${nonce}">${escapeScript(assetSource ?? "")}</script>`;
  return `<!doctype html>
<html data-daily-theme="${theme}">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <meta http-equiv="Content-Security-Policy" content="${escapeHtml(csp)}" />
    <style>
      :root { color-scheme: ${theme}; font-family: -apple-system, BlinkMacSystemFont, "PingFang SC", "Hiragino Sans GB", "Segoe UI", sans-serif; }
      html, body, #root { width: 100%; min-height: 100%; margin: 0; }
      body { background: ${theme === "dark" ? "#17171a" : "#ffffff"}; color: ${theme === "dark" ? "#e2e2e5" : "#16161a"}; overflow: hidden; }
      button, input, textarea, select { font: inherit; }
    </style>
  </head>
  <body>
    <div id="root"></div>
    <script nonce="${nonce}">${loadWatchdogScript()}</script>
    <script nonce="${nonce}">${sdk}</script>
    ${loader}
  </body>
</html>`;
}

// Reports script load failures (e.g. a stopped dev server) and views that never render,
// so the host can show an error instead of an empty frame.
function loadWatchdogScript() {
  const scriptFailed = JSON.stringify(t("Could not load the plugin script: "));
  const inlineScript = JSON.stringify(t("inline script"));
  const runtimeError = JSON.stringify(t("The plugin script threw an error."));
  const noRender = JSON.stringify(t("The plugin rendered nothing within 15 seconds."));
  return `
(function () {
  var reported = false;
  function rendered() {
    var root = document.getElementById("root");
    return Boolean(root && root.childElementCount > 0);
  }
  function report(message) {
    if (reported || rendered()) return;
    reported = true;
    parent.postMessage({ source: "daily-plugin-status", type: "load-error", message: String(message).slice(0, 500) }, "*");
  }
  window.addEventListener("error", function (event) {
    var target = event.target;
    if (target && target !== window && target.tagName === "SCRIPT") {
      report(${scriptFailed} + (target.src || ${inlineScript}));
    } else {
      report(event.message || ${runtimeError});
    }
  }, true);
  setTimeout(function () { report(${noRender}); }, 15000);
})();
`;
}

// Vite dev servers with @vitejs/plugin-react expect the Fast Refresh preamble that their
// index.html normally injects; without it every transformed component throws on load.
function buildDevPreamble(scriptUrl: string, nonce: string) {
  let origin: string;
  try {
    origin = new URL(scriptUrl).origin;
  } catch {
    return "";
  }
  const refreshUrl = JSON.stringify(`${origin}/@react-refresh`);
  return `<script nonce="${nonce}" type="module">
      import RefreshRuntime from ${escapeScript(refreshUrl)};
      RefreshRuntime.injectIntoGlobalHook(window);
      window.$RefreshReg$ = () => {};
      window.$RefreshSig$ = () => (type) => type;
      window.__vite_plugin_react_preamble_installed__ = true;
    </script>
    <script nonce="${nonce}" type="module" src="${escapeHtml(`${origin}/@vite/client`)}"></script>`;
}

function buildFrameCsp(scriptUrl: string | null, nonce: string) {
  const scriptPolicy = `'nonce-${nonce}'`;
  if (!scriptUrl) {
    return `default-src 'none'; script-src ${scriptPolicy}; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; object-src 'none'; base-uri 'none'; form-action 'none'; frame-src 'none'`;
  }
  try {
    const url = new URL(scriptUrl);
    const websocketOrigin = `${url.protocol === "https:" ? "wss:" : "ws:"}//${url.host}`;
    return `default-src 'none'; script-src ${scriptPolicy} ${url.origin}; style-src 'unsafe-inline' ${url.origin}; img-src data: blob: ${url.origin}; font-src data: ${url.origin}; connect-src ${url.origin} ${websocketOrigin}; object-src 'none'; base-uri 'none'; form-action 'none'; frame-src 'none'`;
  } catch {
    return `default-src 'none'; script-src ${scriptPolicy}; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; object-src 'none'; base-uri 'none'; form-action 'none'; frame-src 'none'`;
  }
}

function matchesHttpProtocol(url: URL) {
  return url.protocol === "http:" || url.protocol === "https:";
}

function escapeScript(value: string) {
  return value.replace(/<\/script/gi, "<\\/script");
}

function escapeHtml(value: string) {
  return value.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;");
}

function serializedSize(value: unknown) {
  try {
    return new TextEncoder().encode(JSON.stringify(value)).byteLength;
  } catch {
    return Number.POSITIVE_INFINITY;
  }
}

function pluginDirtyValue(value: unknown): boolean | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return null;
  return typeof (value as { dirty?: unknown }).dirty === "boolean"
    ? (value as { dirty: boolean }).dirty
    : null;
}

function errorMessage(error: unknown, fallback: string) {
  if (error instanceof Error) {
    return error.message;
  }
  if (typeof error === "string") {
    return error;
  }
  return fallback;
}
