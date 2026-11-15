import { invoke, onAppEvent, pluginsDesktop } from "@/platform/desktop";
import { importPlugin } from "./loadPluginModule";
import { PluginRuntime } from "./pluginRuntime";

const AGENT_PROGRESS_EVENT = "plugin://agent-progress";

/** The plugins running in this window. */
export const pluginRuntime = new PluginRuntime({
  importPlugin,
  builtModuleUrl: (pluginId) => pluginsDesktop.moduleUrl(pluginId),
  request: (pluginId, method, payload) => invoke("plugin_host_rpc", { input: { pluginId, method, payload } }),
});

let listening = false;

/** Forwards live agent progress to the plugin that started the run. */
export function forwardPluginEvents(): void {
  if (listening) return;
  listening = true;
  onAppEvent((message) => {
    if (message.event !== AGENT_PROGRESS_EVENT) return;
    const payload = message.payload as { pluginId?: unknown } | null;
    if (typeof payload?.pluginId === "string") pluginRuntime.dispatch(payload.pluginId, "agent.progress", payload);
  });
}
