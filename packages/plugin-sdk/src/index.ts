// A Daily plugin is an ES module whose default export comes from `definePlugin`. Daily imports
// it into the app window, calls `activate(host)` once, and mounts each view into its own shadow
// root when the user opens it (see src/features/plugins/ in the app).
// Rejected host calls throw a DailyHostError whose `code` matches the sidecar's error codes.

export type HostListEntry = {
  name: string;
  isDirectory: boolean;
  size: number;
};

export type HostContext = {
  plugin: { identifier: string; name: string; version: string; settings: unknown };
};

export type NetworkFetchResponse = {
  ok: boolean;
  status: number;
  url: string;
  contentType: string;
  text: string;
};

/** Picks a configured provider and/or one of its models; omitted fields use the app default. */
export type ModelSelection = {
  authConfigId?: string;
  model?: string;
};

/** A configured model provider, as returned by `model.list` (no credentials or endpoints). */
export type ModelProviderOption = {
  id: string;
  name: string;
  providerKind: string;
  models: string[];
  defaultModel: string;
  /** Whether `model.invoke` can send images to this provider. */
  acceptsImages?: boolean;
};

export type ModelListResponse = {
  default: { authConfigId: string; model: string };
  providers: ModelProviderOption[];
};

export type ModelInvokeRequest = {
  prompt: string;
  /** Unknown providers or models are rejected instead of falling back to the default. */
  model?: ModelSelection;
  system?: string;
  temperature?: number;
  maxOutputTokens?: number;
  /** Optional per-call reasoning effort; hosts that do not support it ignore the hint. */
  reasoningEffort?: "low" | "medium" | "high" | "xhigh";
  /** Up to 4 plugin-private PNG, JPEG or WebP files shown to the model; see `acceptsImages`. */
  images?: string[];
};

export type ModelInvokeResponse = {
  text: string;
  /** `max_tokens` means `text` was cut off at the output limit. */
  stopReason: "complete" | "tool_calls" | "max_tokens" | "refusal" | "other";
  /** Token counts, when the provider reports them. */
  usage: {
    inputTokens: number;
    outputTokens: number;
    cachedInputTokens: number;
    reasoningTokens: number;
  } | null;
};

/** Options of `image.generate` (GPT Image 2.6); the host owns the API key. */
export type ImageGenerateRequest = {
  prompt: string;
  /** `sunburst` for detailed work, `flare` for quick iteration. Defaults to `sunburst`. */
  model?: "sunburst" | "flare" | "gpt-image-2.6-sunburst" | "gpt-image-2.6-flare";
  format?: "png" | "jpeg" | "webp" | "svg" | "gif";
  /** `auto` or `WIDTHxHEIGHT` in multiples of 16. */
  size?: string;
  quality?: "low" | "medium" | "high" | "xhigh" | "max" | "auto";
  complexity?: "low" | "medium" | "high";
  background?: "transparent" | "opaque" | "auto";
  moderation?: "auto" | "low";
  n?: number;
  /** 0-100, JPEG or WebP only. */
  compression?: number;
  frames?: number;
  frameDurationMs?: number;
  /** Plugin-private PNG, JPEG or WebP files; sending any switches to the edit endpoint. */
  references?: string[];
  mask?: string;
  markup?: string;
  /** Plugin-private directory the images are saved in. */
  outputDir: string;
  timeoutSeconds?: number;
};

export type ImageGenerateResponse = {
  files: { path: string; bytes: number; mime: string }[];
  model: string;
  endpoint: string;
  requestId: string | null;
  outputFormat: string;
  size: string;
  usage: unknown;
};

export type ImageStatusResponse = {
  configured: boolean;
  source: "daily_provider" | "environment" | "credentials_file" | null;
  models: string[];
};

/** Event view emitted while an agent task runs (mirrors the host's AgentEventView). */
export type AgentEventView = {
  id: string;
  parserId: string;
  kind: "user_input" | "llm_thought" | "tool_call" | "tool_result" | "assistant_text" | "error" | "runtime";
  status: "in_progress" | "waiting" | "completed" | "failed";
  role: "user" | "assistant" | null;
  title: string;
  content: string;
  ply: number | null;
  toolId: string | null;
  toolName: string | null;
  metadata: unknown;
  createdAt: number;
  updatedAt: number;
};

/** One live change to a step: `replace` sets the whole view, `append_content` appends streamed text. */
export type AgentEventViewPatch = {
  view: AgentEventView;
  merge: "replace" | "append_content" | "update";
};

/** Payload of the `agent.progress` event, sent while a request with `streamId` runs. */
export type AgentProgressEvent = {
  pluginId: string;
  streamId: string;
  patches: AgentEventViewPatch[];
};

export type AgentToolDefinition = {
  id: string;
  name: string;
  description: string;
  inputSchema: Record<string, unknown>;
  execution:
    | { type: "async"; pendingMessage: string; metadata?: unknown }
    | { type: "sync"; observation: string; metadata?: unknown };
};

/**
 * An agent the model calls like a tool (`subagent.<id>`): it runs with its own instructions, the
 * task's context blocks and the listed tools, and returns a text result. Tool calls it makes come
 * back as ordinary `tool_call` responses with `subAgent` set.
 */
export type AgentToolSpec = {
  id: string;
  name: string;
  description?: string;
  /** The sub-agent's system prompt. */
  instructions: string;
  /** Ids of the task's tools it may use. */
  tools?: string[];
  /** Defaults to 12, at most 40. */
  maxIterations?: number;
};

export type AgentDefinition = {
  spec: AgentToolSpec;
  modelKey?: string;
  delegates?: string[];
  allowCreate?: boolean;
  allowHandoff?: boolean;
  temperature?: number;
  maxOutputTokens?: number;
};

export type AgentRunLimits = {
  maxModelRequests?: number;
  maxToolCalls?: number;
  maxConcurrency?: number;
  maxDepth?: number;
  maxReportedTokens?: number;
  deadlineUnixMs?: number;
};

export type AgentTaskStartRequest = {
  allowHandoff?: boolean;
  /** Definition registry; root agents are selected by subAgents.agents. */
  definitions?: AgentDefinition[];
  /** Provider selections use configured host providers; credentials remain in the host. */
  modelBindings?: Record<string, ModelSelection>;
  /** Shared parent/child limits, bounded by the desktop host. */
  runLimits?: AgentRunLimits;
  systemPrompt: string;
  userPrompt: string;
  finalInstructions?: string;
  context?: { title: string; content: string }[];
  tools?: AgentToolDefinition[];
  agentAppId?: number;
  /** Used for every segment of the task, including resumes and retries; overrides the agent app's model. */
  model?: ModelSelection;
  /** Capped by the host (see `maxIterations` in the response); defaults to 8. */
  maxIterations?: number;
  maxOutputTokens?: number;
  temperature?: number;
  requireToolBeforeFinal?: boolean;
  continueOnToolError?: boolean;
  /** Record the task in this session and replay its history (messages, tool calls, results) to the model. */
  sessionId?: string;
  /** The author's own words, stored in the session instead of `userPrompt`. */
  displayMessage?: string;
  /** Plugin-owned metadata stored on the author's session entry. */
  messageMeta?: unknown;
  /** Receive `agent.progress` events for this call (see `subscribeAgentProgress`). */
  streamId?: string;
  /**
   * Agents-as-tools: `agents` are offered as tools; with `allowCreate` the model may create its own
   * through the `agent.create` tool. Created agents are kept in the session (`AgentSession.agentTools`).
   */
  subAgents?: { allowCreate?: boolean; agents?: AgentToolSpec[] };
};

export type AgentTaskResumeRequest = {
  runId: string;
  ok?: boolean;
  result?: unknown;
  observation?: string;
  streamId?: string;
};

export type AgentTaskRetryRequest = {
  runId: string;
  streamId?: string;
};

type AgentTaskResponseBase = {
  eventViews: AgentEventView[];
  iterationsUsed: number;
  maxIterations: number;
  /** Set when the requested `maxIterations` exceeded the host limit. */
  limitNotice?: string;
  sessionId?: string;
  /** The assistant session entry holding this task's run record. */
  entryId?: string;
};

export type AgentTaskResponse =
  | (AgentTaskResponseBase & { status: "completed"; text: string; value: unknown })
  | (AgentTaskResponseBase & {
      status: "tool_call";
      runId: string;
      toolId: string;
      toolName: string;
      pendingMessage: string;
      input: unknown;
      metadata: unknown;
      /** The sub-agent that called this tool, when not the task's own agent. */
      subAgent?: { id: string; name: string } | null;
    });

export type AgentSessionToolCall = {
  ply: number;
  toolId: string;
  toolName: string;
  rationale: string;
  input: unknown;
  ok: boolean;
  result: string;
};

export type AgentSessionRun = {
  /** Checkpoint to continue: resume it while `waiting`, retry it while `failed`. */
  runId: string | null;
  status: "running" | "waiting" | "completed" | "failed" | "cancelled";
  toolCalls: AgentSessionToolCall[];
  eventViews: AgentEventView[];
  pendingMessage: string | null;
  pendingToolId: string | null;
  finalValue: unknown;
  error: string | null;
  iterationsUsed: number;
  maxIterations: number;
};

export type AgentSessionEntry = {
  id: string;
  role: "user" | "assistant";
  content: string;
  meta: unknown;
  createdAt: number;
  /** Present on assistant entries produced by `agent.task`. */
  run: AgentSessionRun | null;
};

export type AgentSession = {
  id: string;
  pluginId: string;
  scope: string;
  title: string;
  createdAt: number;
  updatedAt: number;
  activeEntryId: string | null;
  summary: { content: string; throughEntryId: string; createdAt: number } | null;
  entries: AgentSessionEntry[];
  /** Agents the model created in this session; later tasks in it can call them. */
  agentTools?: AgentToolSpec[];
  /** A task is executing in this session right now (a `running` run may be left over from a crash). */
  running?: boolean;
};

export type AgentSessionSummary = {
  id: string;
  scope: string;
  title: string;
  createdAt: number;
  updatedAt: number;
  entryCount: number;
};

export type AgentSessionMessageInput = { role: "user" | "assistant"; content: string; meta?: unknown; idempotencyKey?: string };

export * from "./tasks";

export type DailyPluginApi = {
  tasks: import("./tasks").TaskApi;
  pluginId: string;
  request: (method: string, payload?: unknown) => Promise<unknown>;
  /** Listens for host events (e.g. `agent.progress`); returns an unsubscribe function. */
  on: (event: "agent.progress", listener: (payload: AgentProgressEvent) => void) => () => void;
  host: {
    getContext: () => Promise<HostContext>;
    /** Marks the plugin as holding unsaved changes; Daily asks before closing the workspace. */
    setDirty: (dirty: boolean) => Promise<unknown>;
  };
  storage: {
    get: (key: string) => Promise<unknown>;
    set: (key: string, value: unknown) => Promise<unknown>;
    compareAndSet: (key: string, expected: unknown, value: unknown) => Promise<boolean>;
    compareAndRemove: (key: string, expected: unknown) => Promise<boolean>;
    remove: (key: string) => Promise<unknown>;
    list: () => Promise<Record<string, unknown>>;
  };
  files: {
    readText: (path: string) => Promise<string>;
    writeText: (path: string, content: string) => Promise<unknown>;
    /** Binary read as base64, up to 20 MiB. */
    readBase64: (path: string) => Promise<string>;
    /** Binary write from base64, limited by the 16 MiB request cap. */
    writeBase64: (path: string, base64: string) => Promise<unknown>;
    remove: (path: string) => Promise<unknown>;
    list: (path: string) => Promise<{ entries: HostListEntry[] }>;
    ensureDir: (path: string) => Promise<unknown>;
  };
  workspace: {
    readText: (path: string) => Promise<string>;
    list: (path: string) => Promise<{ entries: HostListEntry[] }>;
    writeText: (path: string, content: string) => Promise<unknown>;
    ensureDir: (path: string) => Promise<unknown>;
    removeFile: (path: string) => Promise<unknown>;
    /** Copies a file from this plugin's private folder into the workspace. */
    copyFromPluginFiles: (from: string, to: string) => Promise<unknown>;
  };
  image: {
    /** Whether a key is available; makes no network request. */
    status: () => Promise<ImageStatusResponse>;
    /** A paid request, never retried by the host; the files are saved under `outputDir`. */
    generate: (payload: ImageGenerateRequest) => Promise<ImageGenerateResponse>;
  };
  model: {
    invoke: (payload: ModelInvokeRequest) => Promise<ModelInvokeResponse>;
    list: () => Promise<ModelListResponse>;
  };
  network: {
    fetch: (
      url: string,
      options?: { timeoutMs?: number; headers?: Record<string, string> },
    ) => Promise<NetworkFetchResponse>;
  };
  agent: {
    list: () => Promise<unknown>;
    run: (payload: unknown) => Promise<unknown>;
    task: {
      start: (payload: AgentTaskStartRequest) => Promise<AgentTaskResponse>;
      resume: (payload: AgentTaskResumeRequest) => Promise<AgentTaskResponse>;
      /**
       * Continues a task from its checkpoint after a failure or interruption: replays a
       * failed resume, re-sends a failed model request, or returns the unanswered tool call.
       */
      retry: (payload: AgentTaskRetryRequest) => Promise<AgentTaskResponse>;
      /** Drops a task's checkpoint; a waiting run is marked cancelled. */
      cancel: (runId: string) => Promise<{ cancelled: boolean }>;
    };
    /** Core-owned transcripts: messages, tool calls and results, compacted by the host. */
    session: {
      create: (payload?: { scope?: string; title?: string }) => Promise<AgentSession>;
      list: (scope?: string) => Promise<AgentSessionSummary[]>;
      get: (sessionId: string) => Promise<AgentSession>;
      remove: (sessionId: string) => Promise<unknown>;
      /** Records plugin-produced messages (answers, errors, notes) in the transcript. */
      appendMessages: (sessionId: string, messages: AgentSessionMessageInput[]) => Promise<AgentSessionEntry[]>;
      updateEntry: (sessionId: string, entryId: string, patch: { content?: string; meta?: unknown }) => Promise<AgentSessionEntry>;
      /**
       * Stops the task executing in this session. Its pending request rejects and the run is
       * recorded as failed with its checkpoint, so it can be continued with `task.retry`.
       */
      stop: (sessionId: string) => Promise<{ stopping: boolean }>;
      /** Drops an agent the model created in this session. */
      removeAgentTool: (sessionId: string, agentId: string) => Promise<{ removed: boolean }>;
    };
  };
};

/** Error rejected by the host bridge, carrying the host's stable error code. */
export type DailyHostError = Error & { code?: string };

export type PluginTheme = "light" | "dark";

/** What a view gets when it mounts. `theme` and `visible` hold the current values. */
export type PluginViewContext = {
  viewId: string;
  /** The shadow root the view lives in; its styles reach neither Daily nor other plugins. */
  root: ShadowRoot;
  theme: PluginTheme;
  /** False while another page is shown. The view stays mounted and keeps its state. */
  visible: boolean;
  /** Aborted when the view unmounts; asynchronous mounts can stop before rendering. */
  signal: AbortSignal;
  /** Adds a stylesheet to the view's shadow root. */
  addStyles(css: string): void;
  onThemeChange(listener: (theme: PluginTheme) => void): () => void;
  onVisibilityChange(listener: (visible: boolean) => void): () => void;
};

export type PluginView = {
  /**
   * Renders the view into `container`, which fills the page and receives keyboard events while
   * the view is shown. Called once per opened view; the returned function tears it down.
   */
  mount(container: HTMLElement, context: PluginViewContext): void | (() => void) | Promise<void | (() => void)>;
};

type PluginCleanup = void | (() => void);

export type PluginDefinition = {
  /**
   * Runs once when the plugin loads, before any view mounts. Start background work here and
   * return a function that stops it.
   */
  activate?(host: DailyPluginApi): PluginCleanup | Promise<PluginCleanup>;
  /** Views by the id they have in `plugin.json`. */
  views?: Record<string, PluginView>;
};

/** The default export Daily expects from a plugin module. */
export type DailyPlugin = {
  activate(host: DailyPluginApi): PluginCleanup | Promise<PluginCleanup>;
  views: Record<string, PluginView>;
};

let currentHost: DailyPluginApi | null = null;

/** Wraps a plugin definition so `dailyHost()` works everywhere in the plugin's code. */
export function definePlugin(definition: PluginDefinition): DailyPlugin {
  return {
    views: definition.views ?? {},
    activate(host) {
      currentHost = host;
      return definition.activate?.(host);
    },
  };
}

/** Sets the host `dailyHost()` returns; plugin tests install a fake with it. */
export function setDailyHost(host: DailyPluginApi | null): void {
  currentHost = host;
}

/** The host API of the running plugin. Available from `activate` onwards. */
export function dailyHost(unavailableMessage = "Daily plugin host SDK is unavailable."): DailyPluginApi {
  if (!currentHost) {
    throw new Error(unavailableMessage);
  }
  return currentHost;
}

/** Calls `onPatches` with the live step changes of requests sent with this `streamId`. */
export function subscribeAgentProgress(streamId: string, onPatches: (patches: AgentEventViewPatch[]) => void): () => void {
  if (!currentHost?.on) return () => undefined;
  return currentHost.on("agent.progress", (event) => {
    if (event.streamId === streamId) onPatches(event.patches);
  });
}

/** Applies live patches to a step list, keeping step order. */
export function applyAgentEventViewPatches(views: AgentEventView[], patches: AgentEventViewPatch[]): AgentEventView[] {
  const next = [...views];
  for (const { view, merge } of patches) {
    const index = next.findIndex((item) => item.id === view.id);
    if (index < 0) {
      next.push(view);
      continue;
    }
    const current = next[index];
    next[index] = merge === "append_content" ? { ...current, content: current.content + view.content, updatedAt: view.updatedAt } : view;
  }
  return next;
}

/** Returns the host error code (e.g. `plugin_disabled`), if the error came from the bridge. */
export function hostErrorCode(error: unknown): string | null {
  return error instanceof Error && typeof (error as DailyHostError).code === "string"
    ? ((error as DailyHostError).code as string)
    : null;
}

export function errorMessage(error: unknown, fallback: string): string {
  if (error instanceof Error && error.message.trim()) {
    return error.message;
  }
  if (typeof error === "string" && error.trim()) {
    return error;
  }
  return fallback;
}

export { createPluginHost, type PluginHostTransport } from "./host";
export { PLUGIN_RPC_METHODS, type PluginRpcMethod } from "./methods.generated";
export { localizeHostText, type HostTextLocale } from "./hostText";
