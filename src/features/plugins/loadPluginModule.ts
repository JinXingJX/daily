import type { DailyPlugin } from "@daily/plugin-sdk";
import { pluginsDesktop } from "@/platform/desktop";
import type { PluginManifest } from "@/types";
import { t } from "@/i18n";

/** The module on the plugin's dev server, when it was installed with a dev URL. */
export function devModuleUrl(plugin: PluginManifest): string | null {
  if (!plugin.devUrl) return null;
  const entry = plugin.devEntry ?? plugin.entry;
  const base = `${plugin.devUrl.replace(/\/$/, "")}/`;
  return entry ? new URL(entry.replace(/^\//, ""), base).toString() : null;
}

const preambles = new Map<string, Promise<void>>();

// Modules transformed by @vitejs/plugin-react refuse to run without the Fast Refresh preamble
// that the dev server's index.html normally installs. Each dev server has its own runtime.
function installReactRefresh(origin: string): Promise<void> {
  let installed = preambles.get(origin);
  if (!installed) {
    installed = import(/* @vite-ignore */ `${origin}/@react-refresh`).then(
      (module: { default: { injectIntoGlobalHook(target: Window): void } }) => {
        const target = window as unknown as Record<string, unknown>;
        module.default.injectIntoGlobalHook(window);
        target.$RefreshReg$ ??= () => undefined;
        target.$RefreshSig$ ??= () => (type: unknown) => type;
        target.__vite_plugin_react_preamble_installed__ = true;
      },
      // Not a Vite React dev server; the entry module loads without it.
      () => undefined,
    );
    preambles.set(origin, installed);
  }
  return installed;
}

export async function importPlugin(plugin: PluginManifest): Promise<{ definition: DailyPlugin; url: string }> {
  const devUrl = devModuleUrl(plugin);
  if (!devUrl && !plugin.entry) throw new Error(t("The plugin has no entry module."));
  const url = devUrl ?? (await pluginsDesktop.moduleUrl(plugin.identifier));
  let module: { default?: Partial<DailyPlugin> };
  try {
    if (devUrl) await installReactRefresh(new URL(devUrl).origin);
    module = await import(/* @vite-ignore */ url);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    throw new Error(
      devUrl
        ? `${message}\n${t("Make sure the dev server {origin} is running (npm run dev in the plugin folder), or reinstall the plugin without a dev URL to load its built files.", { origin: new URL(devUrl).origin })}`
        : message,
    );
  }
  const definition = module.default;
  if (typeof definition?.activate !== "function" || typeof definition.views !== "object" || definition.views === null) {
    throw new Error(t("The plugin module must default-export definePlugin(…)."));
  }
  return { definition: definition as DailyPlugin, url };
}
