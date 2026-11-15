// Loads plugin modules into the app window and keeps one running instance per plugin.
// A plugin is an ES module whose default export has `activate(host)` and `views`
// (see `definePlugin` in @daily/plugin-sdk); its background work and its views share the instance.
import { createPluginHost, type DailyPlugin, type PluginView } from "@daily/plugin-sdk";
import type { PluginManifest } from "@/types";

export type PluginInstance = {
  views: Record<string, PluginView>;
  /** Stylesheets the plugin's views added, shared between their shadow roots. */
  styles: Map<string, CSSStyleSheet>;
};

export type PluginRuntimeDeps = {
  /** Imports the plugin's entry module and returns its default export with the URL it came from. */
  importPlugin(plugin: PluginManifest): Promise<{ definition: DailyPlugin; url: string }>;
  /** URL the built entry module would be imported from now; changes when it is rebuilt. */
  builtModuleUrl(pluginId: string): Promise<string>;
  request(pluginId: string, method: string, payload: unknown): Promise<unknown>;
};

type Slot = {
  signature: string;
  instance: Promise<PluginInstance>;
  url: string | null;
  dirty: boolean;
  stopped: boolean;
  deactivate: (() => void) | null;
  listeners: Map<string, Set<(payload: unknown) => void>>;
};

/** A plugin is re-imported when where its module comes from changes. */
function signature(plugin: PluginManifest) {
  return JSON.stringify([plugin.entry, plugin.devUrl, plugin.devEntry]);
}

export function pluginIsLoadable(plugin: PluginManifest) {
  return plugin.enabled && !plugin.builtIn && Boolean(plugin.devUrl || plugin.entry);
}

export class PluginRuntime {
  private readonly slots = new Map<string, Slot>();
  private readonly generations = new Map<string, number>();
  private readonly subscribers = new Set<() => void>();

  constructor(private readonly deps: PluginRuntimeDeps) {}

  /** Loads and activates a plugin once; later calls return the same instance. */
  load(plugin: PluginManifest): Promise<PluginInstance> {
    if (!pluginIsLoadable(plugin)) return Promise.reject(new Error(`Plugin ${plugin.identifier} is not enabled or has no module.`));
    const existing = this.slots.get(plugin.identifier);
    if (existing) return existing.instance;
    const slot: Slot = {
      signature: signature(plugin),
      instance: Promise.resolve(null as never),
      url: null,
      dirty: false,
      stopped: false,
      deactivate: null,
      listeners: new Map(),
    };
    slot.instance = this.start(plugin, slot);
    // Views report the failure when they mount; startup activation only logs it.
    slot.instance.catch(() => undefined);
    this.slots.set(plugin.identifier, slot);
    return slot.instance;
  }

  /**
   * Brings running plugins in line with the installed list: stops plugins that were disabled,
   * removed or pointed at another module, and starts the ones that activate at startup.
   */
  sync(installed: PluginManifest[]): void {
    const loadable = new Map(installed.filter(pluginIsLoadable).map((plugin) => [plugin.identifier, plugin]));
    for (const [pluginId, slot] of this.slots) {
      const plugin = loadable.get(pluginId);
      if (!plugin || signature(plugin) !== slot.signature) this.unload(pluginId);
    }
    for (const plugin of loadable.values()) {
      if (plugin.activation === "startup" && !this.slots.has(plugin.identifier)) {
        this.load(plugin).catch((error) => console.error(`Plugin ${plugin.identifier} failed to start`, error));
      }
    }
  }

  /** Restarts plugins whose built module changed on disk; `sync` then starts them again. */
  async reloadChanged(): Promise<void> {
    await Promise.all(
      [...this.slots].map(async ([pluginId, slot]) => {
        if (!slot.url?.startsWith("daily-plugin:")) return;
        const url = await this.deps.builtModuleUrl(pluginId).catch(() => null);
        if (url !== slot.url && this.slots.get(pluginId) === slot) this.unload(pluginId);
      }),
    );
  }

  /** Stops a plugin; its mounted views unmount because the generation changes. */
  unload(pluginId: string): void {
    const slot = this.slots.get(pluginId);
    if (!slot) return;
    this.slots.delete(pluginId);
    slot.stopped = true;
    slot.listeners.clear();
    slot.deactivate?.();
    this.generations.set(pluginId, this.generation(pluginId) + 1);
    this.notify();
  }

  /** Changes whenever the plugin's instance is replaced. */
  generation(pluginId: string): number {
    return this.generations.get(pluginId) ?? 0;
  }

  /** Whether any running plugin reported unsaved changes. */
  isDirty = (): boolean => [...this.slots.values()].some((slot) => slot.dirty);

  subscribe = (listener: () => void): (() => void) => {
    this.subscribers.add(listener);
    return () => this.subscribers.delete(listener);
  };

  /** Delivers a host event (e.g. `agent.progress`) to the plugin it belongs to. */
  dispatch(pluginId: string, event: string, payload: unknown): void {
    this.slots.get(pluginId)?.listeners.get(event)?.forEach((listener) => {
      try {
        listener(payload);
      } catch (error) {
        console.error(error);
      }
    });
  }

  private async start(plugin: PluginManifest, slot: Slot): Promise<PluginInstance> {
    const pluginId = plugin.identifier;
    const { definition, url } = await this.deps.importPlugin(plugin);
    slot.url = url;
    const host = createPluginHost(pluginId, {
      request: (method, payload) => slot.stopped
        ? Promise.reject(new Error(`Plugin ${pluginId} was stopped.`))
        : this.deps.request(pluginId, method, payload),
      on: (event, listener) => {
        if (slot.stopped) return () => undefined;
        let listeners = slot.listeners.get(event);
        if (!listeners) slot.listeners.set(event, (listeners = new Set()));
        listeners.add(listener);
        return () => listeners.delete(listener);
      },
      setDirty: (dirty) => {
        if (slot.stopped || slot.dirty === dirty) return;
        slot.dirty = dirty;
        this.notify();
      },
    });
    const cleanup = slot.stopped ? undefined : await definition.activate(host);
    const deactivate = () => {
      try {
        if (typeof cleanup === "function") cleanup();
      } catch (error) {
        console.error(`Plugin ${pluginId} failed to stop cleanly`, error);
      }
    };
    // Unloaded while it was still starting.
    if (slot.stopped) {
      deactivate();
      throw new Error(`Plugin ${pluginId} was stopped.`);
    }
    slot.deactivate = deactivate;
    return { views: definition.views, styles: new Map() };
  }

  private notify(): void {
    this.subscribers.forEach((listener) => listener());
  }
}
