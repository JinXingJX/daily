import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { pluginViewIcon, pluginViewTone } from "@/features/workspace/Sidebar";
import { cn } from "@/lib/utils";
import type {
  PluginManifest,
  PluginSnapshot
} from "@/types";
import {
  ChevronDown,
  Plug,
  Plus,
  RefreshCcw,
  Trash2
} from "lucide-react";
import { useState } from "react";
import { PLUGIN_SOURCE_LABELS, SettingsHeader } from "./settingsShared";
import { t } from "@/i18n";

export function PluginSettings({
  plugins,
  busyCommand,
  onReloadPlugins,
  onInstallPlugin,
  onInstallLocalPlugin,
  onUninstallPlugin,
  onEnablePlugin,
  onDisablePlugin,
}: {
  plugins: PluginSnapshot;
  busyCommand: string | null;
  onReloadPlugins: () => void;
  onInstallPlugin: (pluginId: string) => void;
  onInstallLocalPlugin: (manifestPath: string, devUrl?: string) => void;
  onUninstallPlugin: (plugin: PluginManifest) => void;
  onEnablePlugin: (pluginId: string) => void;
  onDisablePlugin: (pluginId: string) => void;
}) {
  const [tab, setTab] = useState<"installed" | "marketplace">("installed");
  const [installOpen, setInstallOpen] = useState(false);
  const [localManifestPath, setLocalManifestPath] = useState("");
  const [localDevUrl, setLocalDevUrl] = useState("");
  const [expandedId, setExpandedId] = useState<string | null>(() => plugins.installed[0]?.identifier ?? null);
  const installBusy = busyCommand === "install_local_dev_plugin";

  return (
    <section className="flex flex-col gap-5">
      <SettingsHeader
        title={t("Plugins")}
        detail={t("Plugins run in sandboxed views and can use every host capability (network, storage, workspace, models and agents). Install only plugins you trust.")}
        action={
          <div className="flex gap-2">
            <Button variant="secondary" size="sm" disabled={busyCommand === "reload_plugins"} onClick={onReloadPlugins}>
              <RefreshCcw className="size-3.5" />
              {t("Reload")}
            </Button>
            <Button size="sm" aria-expanded={installOpen} onClick={() => setInstallOpen((value) => !value)}>
              <Plus className="size-3.5" />
              {t("Install local plugin")}
            </Button>
          </div>
        }
      />

      {installOpen ? (
        <form
          className="plugin-install motion-reveal"
          onSubmit={(event) => {
            event.preventDefault();
            if (!localManifestPath.trim() || installBusy) return;
            onInstallLocalPlugin(localManifestPath, localDevUrl || undefined);
          }}
        >
          <div className="flex flex-wrap gap-2">
            <Input
              className="min-w-[14rem] flex-[2_1_16rem]"
              value={localManifestPath}
              placeholder={t("Absolute path to a plugin folder or .daily-plugin/plugin.json")}
              aria-label={t("Local plugin path")}
              spellCheck={false}
              autoFocus
              onChange={(event) => setLocalManifestPath(event.currentTarget.value)}
            />
            <Input
              className="min-w-[10rem] flex-[1_1_12rem]"
              value={localDevUrl}
              placeholder={t("Dev server URL (optional)")}
              aria-label={t("Dev server URL (optional)")}
              inputMode="url"
              spellCheck={false}
              onChange={(event) => setLocalDevUrl(event.currentTarget.value)}
            />
            <Button type="submit" disabled={!localManifestPath.trim() || installBusy}>
              {installBusy ? t("Installing…") : t("Install")}
            </Button>
          </div>
          <p className="mb-0 mt-2.5 text-xs leading-5 text-[var(--text-muted)]">
            {t("Without a dev URL the built files are loaded from disk. Enter a local URL such as")} <code className="mono">http://127.0.0.1:5174</code> {t("for hot reload. Only localhost addresses are accepted.")}
          </p>
        </form>
      ) : null}

      <div role="tablist" aria-label={t("Plugin source")} className="tab-list gap-1">
        <button
          type="button"
          role="tab"
          aria-selected={tab === "installed"}
          className="tab"
          onClick={() => setTab("installed")}
        >
          {t("Installed")} <span className="mono font-normal text-[var(--text-muted)]">{plugins.installed.length}</span>
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={tab === "marketplace"}
          className="tab"
          onClick={() => setTab("marketplace")}
        >
          {t("Marketplace")} <span className="mono font-normal text-[var(--text-muted)]">{plugins.marketplace.length}</span>
        </button>
      </div>

      {tab === "installed" ? (
        <div className="flex flex-col gap-3">
          {plugins.installed.map((plugin) => (
            <PluginCard
              key={plugin.identifier}
              plugin={plugin}
              expanded={expandedId === plugin.identifier}
              onToggleExpanded={() => setExpandedId((current) => (current === plugin.identifier ? null : plugin.identifier))}
              onUninstall={() => onUninstallPlugin(plugin)}
              onEnabledChange={(enabled) => (enabled ? onEnablePlugin(plugin.identifier) : onDisablePlugin(plugin.identifier))}
            />
          ))}
          {!plugins.installed.length ? (
            <div className="rounded-[var(--radius-panel)] border border-dashed border-[var(--line-strong)] px-6 py-10 text-center text-[13px] text-[var(--text-muted)]">
              {t("No plugins installed. Install one from the marketplace or a local development plugin.")}
            </div>
          ) : null}
        </div>
      ) : (
        <div className="settings-group">
          {plugins.marketplace.map((entry) => (
            <div key={entry.plugin.identifier} className="settings-row">
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <h3 className="m-0 truncate text-[13px] font-semibold text-[var(--text-strong)]">{entry.plugin.name}</h3>
                  <span className="mono text-[11px] text-[var(--text-muted)]">{entry.plugin.version}</span>
                </div>
                <p className="m-0 mt-0.5 text-[12px] leading-5 text-[var(--text-soft)]">{entry.plugin.description}</p>
              </div>
              <Button
                variant="secondary"
                size="sm"
                disabled={entry.installed || busyCommand === "install_plugin"}
                onClick={() => onInstallPlugin(entry.plugin.identifier)}
              >
                {entry.installed ? t("Installed") : t("Install")}
              </Button>
            </div>
          ))}
          {!plugins.marketplace.length ? (
            <div className="px-6 py-10 text-center text-[13px] text-[var(--text-muted)]">{t("The marketplace has no plugins yet.")}</div>
          ) : null}
        </div>
      )}
    </section>
  );
}

export function PluginCard({
  plugin,
  expanded,
  onToggleExpanded,
  onUninstall,
  onEnabledChange,
}: {
  plugin: PluginManifest;
  expanded: boolean;
  onToggleExpanded: () => void;
  onUninstall: () => void;
  onEnabledChange: (enabled: boolean) => void;
}) {
  const mainView = plugin.contributes.views.find((view) => view.location === "main") ?? plugin.contributes.views[0];
  const Icon = mainView ? pluginViewIcon(mainView) : Plug;
  const tone = mainView ? pluginViewTone(mainView) : "neutral";
  const views = plugin.contributes.views.map((view) => (view.location === "background" ? t("{title} (background)", { title: view.title }) : view.title));
  const panelId = `plugin-${plugin.identifier}`;

  return (
    <article
      className={cn("plugin-card", !plugin.enabled && "plugin-card-disabled")}
      data-expanded={expanded}
      aria-label={plugin.name}
    >
      <div className="flex items-center gap-3 px-4 py-3.5">
        <span className="plugin-tile" style={{ color: `var(--tone-${tone})`, background: `var(--tone-${tone}-soft)` }}>
          <Icon className="size-[18px]" />
        </span>
        <button
          type="button"
          className="focus-ring flex min-w-0 flex-1 flex-col rounded-[var(--radius-control)] text-left"
          aria-expanded={expanded}
          aria-controls={panelId}
          onClick={onToggleExpanded}
        >
          <span className="flex flex-wrap items-center gap-x-2 gap-y-0.5">
            <span className="truncate text-[14px] font-semibold text-[var(--text-strong)]">{plugin.name}</span>
            <span className="mono text-[11px] text-[var(--text-muted)]">
              {plugin.identifier} · {plugin.version}
            </span>
            <span className="settings-state" data-tone={plugin.source === "local_dev" ? "accent" : undefined}>
              {PLUGIN_SOURCE_LABELS[plugin.source]}
            </span>
          </span>
          <span className={cn("text-[12px] leading-5 text-[var(--text-soft)]", !expanded && "truncate")}>
            {expanded
              ? plugin.description
              : views.join(" · ") || t("No views")}
          </span>
        </button>
        <label className="flex shrink-0 cursor-pointer items-center gap-2 text-[12px] text-[var(--text)]">
          {plugin.enabled ? t("Enabled") : t("Disabled")}
          <Switch
            checked={plugin.enabled}
            aria-label={plugin.enabled ? t("Disable {name}", { name: plugin.name }) : t("Enable {name}", { name: plugin.name })}
            onCheckedChange={onEnabledChange}
          />
        </label>
        <ChevronDown
          className={cn("size-4 shrink-0 text-[var(--text-muted)] transition-transform", expanded && "rotate-180")}
          aria-hidden="true"
        />
      </div>

      {plugin.runtimeErrors.length ? (
        <p className="mx-4 mb-3 mt-0 rounded-[var(--radius-control)] bg-[var(--danger-soft)] px-2.5 py-1.5 text-xs leading-5 text-[var(--danger)]">
          {plugin.runtimeErrors[plugin.runtimeErrors.length - 1]}
        </p>
      ) : null}

      {expanded ? (
        <div id={panelId}>
          <div className="stat-grid grid-cols-1 !rounded-none !border-x-0 sm:grid-cols-3">
            <div className="!bg-[var(--surface)] !px-4">
              <div className="stat-label">{t("Source")}</div>
              <div className="stat-value">{PLUGIN_SOURCE_LABELS[plugin.source]}</div>
            </div>
            <div className="!bg-[var(--surface)] !px-4">
              <div className="stat-label">{t("Views")}</div>
              <div className="stat-value" title={views.join("、")}>
                {views.join(" · ") || t("None")}
              </div>
            </div>
            <div className="!bg-[var(--surface)] !px-4">
              <div className="stat-label">{t("Entry")}</div>
              <div className="stat-value mono text-[12px]" title={plugin.entry ?? undefined}>
                {plugin.entry ?? "—"}
              </div>
            </div>
          </div>

          <div className="flex flex-col gap-2.5 px-4 pb-4 pt-3.5">
            {!plugin.builtIn ? (
              <div className="flex justify-end pt-1">
                <Button variant="ghost" size="sm" className="h-8 text-xs text-[var(--danger)]" onClick={onUninstall}>
                  <Trash2 className="size-3.5" />
                  {t("Remove plugin")}
                </Button>
              </div>
            ) : null}
          </div>
        </div>
      ) : null}
    </article>
  );
}
