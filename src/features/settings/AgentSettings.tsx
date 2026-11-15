import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { changedAgentFields } from "@/features/agents/agentDraftDiff";
import {
  agentToFormState,
  moveHookAgent,
  toggleHookAgentSelection,
  wouldIntroduceHookCycle,
  type AgentFormState,
  type HookField,
} from "@/features/agents/agentHooks";
import { pluginViewIcon } from "@/features/workspace/Sidebar";
import { cn } from "@/lib/utils";
import type {
  AgentAppConfig,
  AgentHookConfig,
  ContextRootOption,
  PluginManifest,
  SettingsView
} from "@/types";
import {
  ArrowDown,
  ArrowUp,
  Bot,
  Plug,
  Plus,
  Save,
  Trash2,
  X
} from "lucide-react";
import { useEffect, useState, type Dispatch, type SetStateAction } from "react";
import { Field, formatBytes, MAX_SYSTEM_PROMPT_BYTES } from "./settingsShared";
import { t } from "@/i18n";

export function AgentSettings({
  agentApps,
  installedPlugins,
  settings,
  agentDraft,
  busy,
  setAgentDraft,
  onCreateAgent,
  onDeleteAgent,
  onSaveAgent,
  onSelectAgent,
  onLoadContextRoots,
}: {
  agentApps: AgentAppConfig[];
  installedPlugins: PluginManifest[];
  settings: SettingsView;
  agentDraft: AgentFormState | null;
  busy: boolean;
  setAgentDraft: Dispatch<SetStateAction<AgentFormState | null>>;
  onCreateAgent: () => void;
  onDeleteAgent: () => void;
  onSaveAgent: () => void;
  onSelectAgent: (agentId: number | null) => void;
  onLoadContextRoots?: () => Promise<ContextRootOption[]>;
}) {
  const [contextRootOptions, setContextRootOptions] = useState<ContextRootOption[] | null>(null);
  const savedAgent = agentApps.find((agent) => agent.id === agentDraft?.id) ?? null;
  const changes = changedAgentFields(agentDraft, savedAgent);
  const authConfigs = settings.effective.model.authConfigs;
  const selectedAuthConfig =
    authConfigs.find((authConfig) => authConfig.id === agentDraft?.authConfigId) ??
    authConfigs.find((authConfig) => authConfig.id === settings.effective.model.authConfigId) ??
    authConfigs[0] ??
    null;
  const modelOptions = selectedAuthConfig
    ? Array.from(
        new Set(
          [agentDraft?.model, selectedAuthConfig.defaultModel, ...selectedAuthConfig.models].filter(
            (model): model is string => Boolean(model),
          ),
        ),
      )
    : [];
  const promptBytes = agentDraft ? new TextEncoder().encode(agentDraft.systemPrompt).length : 0;

  useEffect(() => {
    if (!onLoadContextRoots) return;
    let cancelled = false;
    onLoadContextRoots()
      .then((options) => {
        if (!cancelled) setContextRootOptions(options);
      })
      .catch(() => {
        if (!cancelled) setContextRootOptions([]);
      });
    return () => {
      cancelled = true;
    };
  }, [onLoadContextRoots]);

  useEffect(() => {
    if (!changes.length) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "s") {
        event.preventDefault();
        if (!busy) onSaveAgent();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [busy, changes.length, onSaveAgent]);

  function patchAgent(patch: Partial<AgentFormState>) {
    setAgentDraft((current) => (current ? { ...current, ...patch } : current));
  }

  function chooseAuthConfig(authConfigId: string) {
    const authConfig = authConfigs.find((candidate) => candidate.id === authConfigId);
    patchAgent({
      authConfigId,
      providerKind: authConfig?.providerKind ?? "codex_login",
      model: authConfig?.defaultModel ?? agentDraft?.model ?? "",
    });
  }

  function toggleListValue(field: "mountedPluginIds" | "contextRoots", value: string) {
    setAgentDraft((current) => {
      if (!current) return current;
      const list = current[field];
      return { ...current, [field]: list.includes(value) ? list.filter((item) => item !== value) : [...list, value] };
    });
  }

  function selectAgent(agentId: number) {
    if (agentId === agentDraft?.id) return;
    if (changes.length && !window.confirm(t("“{name}” has unsaved changes: {changes}. Discard them and switch?", { name: agentDraft?.name ?? "", changes: changes.join(", ") }))) {
      return;
    }
    onSelectAgent(agentId);
  }

  function discardChanges() {
    if (savedAgent) setAgentDraft(agentToFormState(savedAgent, settings.effective.model.authConfigId));
  }

  const availableRoots = (contextRootOptions ?? []).filter((option) => !agentDraft?.contextRoots.includes(option.path));

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col md:flex-row">
      <section aria-label={t("Agent list")} className="flex max-h-48 w-full shrink-0 flex-col border-b border-[var(--line-soft)] md:max-h-none md:w-[15.5rem] md:border-b-0 md:border-r">
        <div className="flex items-center justify-between px-4 pb-2.5 pt-3.5">
          <h2 className="settings-section-title">{t("Agents")}</h2>
          <Button variant="secondary" size="sm" className="h-7 gap-1 px-2 text-xs" onClick={onCreateAgent}>
            <Plus className="size-3.5" />
            {t("Create")}
          </Button>
        </div>
        <div className="flex flex-col gap-0.5 overflow-y-auto px-2 pb-3">
          {agentApps.map((agent) => {
            const selected = agentDraft?.id === agent.id;
            const finishHooks = agent.onFinishHook.agentAppIds.length;
            return (
              <button
                key={agent.id}
                className={cn("settings-list-row", selected && "settings-list-row-active")}
                type="button"
                aria-current={selected ? "true" : undefined}
                onClick={() => selectAgent(agent.id)}
              >
                <span className="flex items-center gap-1.5">
                  <span className="truncate font-medium">{selected ? agentDraft?.name || agent.name : agent.name}</span>
                  <span className="flex-1" />
                  {selected && changes.length ? (
                    <span className="size-1.5 shrink-0 rounded-full bg-[var(--warning-fill)]" role="img" aria-label={t("Unsaved changes")} />
                  ) : null}
                </span>
                <span className="truncate text-[11px] text-[var(--text-muted)]">
                  {[
                    agent.model,
                    agent.contextRoots.length ? t("{count} folders", { count: agent.contextRoots.length }) : null,
                    finishHooks ? t("{count} finish hooks", { count: finishHooks }) : null,
                  ]
                    .filter(Boolean)
                    .join(" · ")}
                </span>
              </button>
            );
          })}
        </div>
      </section>

      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        {agentDraft ? (
          <>
            <div className="min-h-0 flex-1 overflow-y-auto px-6 py-6 lg:px-8">
              <div className="flex max-w-[44rem] flex-col gap-7">
                <div className="flex items-start gap-3">
                  <div className="min-w-0 flex-1">
                    <h1 className="m-0 truncate text-[20px] font-semibold leading-7 tracking-[-0.01em] text-[var(--text-strong)]">
                      {agentDraft.name || t("Untitled agent")}
                    </h1>
                    <p className="settings-section-detail">{t("Save after editing. In a chat you can switch agents next to the input at any time.")}</p>
                  </div>
                  <Button
                    variant="secondary"
                    size="sm"
                    className="h-8 text-xs text-[var(--danger)]"
                    disabled={agentApps.length <= 1}
                    title={agentApps.length <= 1 ? t("At least one agent must remain") : undefined}
                    onClick={onDeleteAgent}
                  >
                    <Trash2 className="size-3.5" />
                    {t("Delete")}
                  </Button>
                </div>

                <section className="flex flex-col gap-3.5">
                  <h2 className="settings-section-title">{t("Model")}</h2>
                  <div className="grid gap-3.5 md:grid-cols-2">
                    <Field label={t("Name")}>
                      <Input value={agentDraft.name} onChange={(event) => patchAgent({ name: event.currentTarget.value })} />
                    </Field>
                    <Field label={t("Provider")}>
                      <Select value={agentDraft.authConfigId || selectedAuthConfig?.id} onValueChange={chooseAuthConfig}>
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          {authConfigs.map((authConfig) => (
                            <SelectItem key={authConfig.id} value={authConfig.id}>
                              {authConfig.name}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </Field>
                    <Field label={t("Model")}>
                      {modelOptions.length ? (
                        <Select value={agentDraft.model} onValueChange={(model) => patchAgent({ model })}>
                          <SelectTrigger>
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            {modelOptions.map((model) => (
                              <SelectItem key={model} value={model}>
                                {model}
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                      ) : (
                        <Input value={agentDraft.model} onChange={(event) => patchAgent({ model: event.currentTarget.value })} />
                      )}
                    </Field>
                    <div className="grid gap-2">
                      <div className="flex justify-between text-[13px]">
                        <label htmlFor="agent-temperature" className="font-medium text-[var(--text-soft)]">
                          {t("Temperature")}
                        </label>
                        <span className="mono text-[var(--text-strong)]">{agentDraft.temperature.toFixed(1)}</span>
                      </div>
                      <div className="flex h-10 items-center gap-2.5">
                        <span className="text-[11px] text-[var(--text-muted)]">{t("Precise")}</span>
                        <input
                          id="agent-temperature"
                          type="range"
                          min={0}
                          max={2}
                          step={0.1}
                          value={agentDraft.temperature}
                          className="h-1 min-w-0 flex-1 cursor-pointer accent-[var(--accent)]"
                          onChange={(event) => patchAgent({ temperature: Number(event.currentTarget.value) })}
                        />
                        <span className="text-[11px] text-[var(--text-muted)]">{t("Creative")}</span>
                      </div>
                    </div>
                  </div>
                </section>

                <section className="flex flex-col gap-2">
                  <div className="flex items-baseline justify-between">
                    <h2 className="settings-section-title">
                      <label htmlFor="agent-system-prompt">{t("System prompt")}</label>
                    </h2>
                    <span
                      className={cn(
                        "mono text-[11px]",
                        promptBytes > MAX_SYSTEM_PROMPT_BYTES ? "text-[var(--danger)]" : "text-[var(--text-muted)]",
                      )}
                    >
                      {formatBytes(promptBytes)} / 256 KB
                    </span>
                  </div>
                  <Textarea
                    id="agent-system-prompt"
                    className="mono min-h-40 resize-y text-[12.5px] leading-[1.65]"
                    value={agentDraft.systemPrompt}
                    onChange={(event) => patchAgent({ systemPrompt: event.currentTarget.value })}
                  />
                </section>

                <section className="flex flex-col gap-2.5">
                  <div>
                    <h2 className="settings-section-title">{t("Context folders")}</h2>
                    <p className="settings-section-detail">{t("The agent reads and writes only these folders; when empty it can access the whole workspace.")}</p>
                  </div>
                  <div className="flex flex-wrap gap-1.5">
                    {agentDraft.contextRoots.map((root) => (
                      <span key={root} className="path-chip">
                        <span className="truncate">{root === "." ? t("(project root)") : root}</span>
                        <button
                          type="button"
                          className="chip-remove"
                          aria-label={t("Remove {name}", { name: root })}
                          onClick={() => toggleListValue("contextRoots", root)}
                        >
                          <X className="size-3" />
                        </button>
                      </span>
                    ))}
                    {onLoadContextRoots ? (
                      <Select value="" onValueChange={(path) => toggleListValue("contextRoots", path)}>
                        <SelectTrigger
                          aria-label={t("Add folder")}
                          className="dashed-button h-7 w-auto min-w-0 gap-1 [&>svg:last-child]:hidden"
                          disabled={contextRootOptions == null || !availableRoots.length}
                        >
                          <Plus className="size-3" />
                          <span>{contextRootOptions == null ? t("Reading folders…") : availableRoots.length ? t("Add folder") : t("No more folders")}</span>
                        </SelectTrigger>
                        <SelectContent className="max-h-72">
                          {availableRoots.map((option) => (
                            <SelectItem key={option.path} value={option.path}>
                              <span className="mono text-[12px]">{option.path === "." ? t("(project root)") : option.label}</span>
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    ) : null}
                  </div>
                </section>

                <section className="flex flex-col gap-2.5">
                  <div>
                    <h2 className="settings-section-title">{t("Callable plugins")}</h2>
                    <p className="settings-section-detail">{t("Mounted plugins tell the agent which renderers and tools it can use in replies.")}</p>
                  </div>
                  {installedPlugins.length ? (
                    <div className="check-list">
                      {installedPlugins.map((plugin) => {
                        const mainView = plugin.contributes.views[0];
                        const Icon = mainView ? pluginViewIcon(mainView) : Plug;
                        return (
                          <label key={plugin.identifier} className="check-list-row">
                            <Checkbox
                              checked={agentDraft.mountedPluginIds.includes(plugin.identifier)}
                              onCheckedChange={() => toggleListValue("mountedPluginIds", plugin.identifier)}
                            />
                            <Icon className="size-[15px] shrink-0 text-[var(--text-soft)]" aria-hidden="true" />
                            <span className="min-w-0 flex-1 truncate text-[var(--text-strong)]">{plugin.name}</span>
                            {!plugin.enabled ? <span className="settings-state">{t("Disabled")}</span> : null}
                            <span className="mono hidden truncate text-[11px] text-[var(--text-muted)] sm:inline">{plugin.identifier}</span>
                          </label>
                        );
                      })}
                    </div>
                  ) : (
                    <p className="m-0 text-[12px] text-[var(--text-muted)]">{t("No plugins installed yet.")}</p>
                  )}
                </section>

                <section className="flex flex-col gap-2.5">
                  <div>
                    <h2 className="settings-section-title">{t("Lifecycle hooks")}</h2>
                    <p className="settings-section-detail">{t("Run other agents automatically before or after each turn.")}</p>
                  </div>
                  <div className="grid gap-3 md:grid-cols-2">
                    <HookEditor
                      title={t("Before")}
                      field="onStartHook"
                      draft={agentDraft}
                      agents={agentApps}
                      onChange={(hook) => patchAgent({ onStartHook: hook })}
                    />
                    <HookEditor
                      title={t("After")}
                      field="onFinishHook"
                      draft={agentDraft}
                      agents={agentApps}
                      onChange={(hook) => patchAgent({ onFinishHook: hook })}
                    />
                  </div>
                </section>
              </div>
            </div>

            <div className="save-bar" role="status" aria-live="polite">
              <span
                className={cn("status-dot", !changes.length && "invisible")}
                data-tone="warning"
                aria-hidden="true"
              />
              <span className="min-w-0 flex-1 truncate text-[12px] text-[var(--text-soft)]">
                {changes.length ? t("{count} unsaved changes: {changes}", { count: changes.length, changes: changes.join(", ") }) : t("All changes saved")}
              </span>
              <Button variant="secondary" size="sm" className="h-8 text-xs" disabled={!changes.length || busy} onClick={discardChanges}>
                {t("Discard")}
              </Button>
              <Button size="sm" className="h-8 gap-1.5 text-xs" disabled={!changes.length || busy} onClick={onSaveAgent}>
                <Save className="size-3.5" />
                {busy ? t("Saving…") : t("Save")}
                <kbd className="text-[11px] font-normal opacity-60">⌘S</kbd>
              </Button>
            </div>
          </>
        ) : (
          <div className="grid flex-1 place-items-center p-10 text-[13px] text-[var(--text-muted)]">
            {t("Create an agent to configure it here.")}
          </div>
        )}
      </div>
    </div>
  );
}

export function HookEditor({
  title,
  field,
  draft,
  agents,
  onChange,
}: {
  title: string;
  field: HookField;
  draft: AgentFormState;
  agents: AgentAppConfig[];
  onChange: (hook: AgentHookConfig) => void;
}) {
  const hook = draft[field];
  const selected = hook.agentAppIds
    .map((id) => agents.find((agent) => agent.id === id))
    .filter((agent): agent is AgentAppConfig => Boolean(agent));
  const candidates = agents.filter((agent) => agent.id !== draft.id && !hook.agentAppIds.includes(agent.id));
  const active = selected.length > 0;
  const mode = hook.mode ?? "serial";

  function addAgent(agentId: number) {
    onChange({ ...toggleHookAgentSelection(hook, agentId), mode });
  }

  return (
    <section className="hook-card" data-active={active} aria-label={t("{title} hooks", { title })}>
      <div className="flex min-h-6 items-center justify-between gap-2">
        <span className="font-medium text-[var(--text-strong)]">{title}</span>
        {active ? (
          <div className="segmented segmented-sm" role="radiogroup" aria-label={t("{title} run mode", { title })}>
            <button
              type="button"
              role="radio"
              aria-checked={mode === "serial"}
              className="segmented-item"
              onClick={() => onChange({ ...hook, mode: "serial" })}
            >
              {t("Serial")}
            </button>
            <button
              type="button"
              role="radio"
              aria-checked={mode === "parallel"}
              className="segmented-item"
              onClick={() => onChange({ ...hook, mode: "parallel", appendPreviousOutput: false })}
            >
              {t("Parallel")}
            </button>
          </div>
        ) : (
          <span className="text-[11px] text-[var(--text-muted)]">{t("Off")}</span>
        )}
      </div>

      {selected.map((agent, index) => (
        <div key={agent.id} className="hook-agent">
          <Bot className="size-[13px] shrink-0 text-[var(--text-soft)]" aria-hidden="true" />
          <span className="min-w-0 flex-1 truncate text-[var(--text-strong)]">{agent.name}</span>
          {mode === "serial" && selected.length > 1 ? (
            <>
              <button
                type="button"
                className="chip-remove"
                aria-label={t("Move {name} up", { name: agent.name })}
                disabled={index === 0}
                onClick={() => onChange(moveHookAgent(hook, agent.id, -1))}
              >
                <ArrowUp className="size-3" />
              </button>
              <button
                type="button"
                className="chip-remove"
                aria-label={t("Move {name} down", { name: agent.name })}
                disabled={index === selected.length - 1}
                onClick={() => onChange(moveHookAgent(hook, agent.id, 1))}
              >
                <ArrowDown className="size-3" />
              </button>
            </>
          ) : null}
          <button
            type="button"
            className="chip-remove"
            aria-label={t("Remove {name}", { name: agent.name })}
            onClick={() => onChange(toggleHookAgentSelection(hook, agent.id))}
          >
            <X className="size-3" />
          </button>
        </div>
      ))}

      {candidates.length ? (
        <Select value="" onValueChange={(value) => addAgent(Number(value))}>
          <SelectTrigger
            aria-label={t("Add an agent to the {title} hooks", { title })}
            className="dashed-button h-8 w-full justify-center gap-1 [&>svg:last-child]:hidden"
          >
            <Plus className="size-3" />
            <span>{t("Add agent")}</span>
          </SelectTrigger>
          <SelectContent>
            {candidates.map((agent) => {
              const cycle = wouldIntroduceHookCycle(agents, draft, field, toggleHookAgentSelection(hook, agent.id), agent.id);
              return (
                <SelectItem key={agent.id} value={String(agent.id)} disabled={cycle}>
                  {agent.name}
                  {cycle ? ` ${t("(would create a cycle)")}` : ""}
                </SelectItem>
              );
            })}
          </SelectContent>
        </Select>
      ) : !active ? (
        <p className="m-0 text-[12px] text-[var(--text-muted)]">{t("No other agents available.")}</p>
      ) : null}

      {active && mode === "serial" ? (
        <label className="flex cursor-pointer items-center gap-2 text-[12px] text-[var(--text)]">
          <Checkbox
            checked={hook.appendPreviousOutput}
            onCheckedChange={(checked) => onChange({ ...hook, appendPreviousOutput: checked === true })}
          />
          {t("Pass each agent's output to the next")}
        </label>
      ) : null}
    </section>
  );
}
