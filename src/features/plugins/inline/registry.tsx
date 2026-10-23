import { LoaderCircle } from "lucide-react";
import { Skeleton } from "@/components/ui/skeleton";
import {
  isRecord,
  type DailyPluginEnvelope,
  type PluginRenderResult,
  type PluginSpec,
} from "@/features/plugins/inline/types";
import { PluginFallback, PluginShell } from "@/features/plugins/inline/PluginShell";
import { chartSpec } from "@/features/plugins/inline/chart";
import { weatherSpec } from "@/features/plugins/inline/weather";
import { agentHooksSpec } from "@/features/plugins/inline/agentHooks";

function isDailyPluginEnvelope(value: unknown): value is DailyPluginEnvelope {
  return (
    isRecord(value) &&
    typeof value.pluginId === "string" &&
    typeof value.renderer === "string" &&
    "payload" in value
  );
}


const pluginRegistry: Record<string, PluginSpec<any>> = Object.fromEntries(
  [chartSpec, weatherSpec, agentHooksSpec].map((spec) => [pluginKey(spec.pluginId, spec.renderer), spec]),
);

const legacyPluginMap: Record<string, { pluginId: string; renderer: string }> = {
  "plugin:chart": { pluginId: "builtin.chart", renderer: "chart" },
  "plugin:weather": { pluginId: "builtin.weather", renderer: "weather" },
  "plugin:agent_hooks": { pluginId: "builtin.agent_hooks", renderer: "agent_hooks" },
};

function pluginKey(pluginId: string, renderer: string) {
  return `${pluginId}:${renderer}`;
}

export function renderPluginBlock(identifier: string, source: string, partial: boolean): PluginRenderResult {
  if (identifier === "daily-plugin") {
    return renderDailyPluginEnvelope(source, partial);
  }

  const legacy = legacyPluginMap[identifier];
  if (!legacy) {
    return {
      ok: false,
      content: <PluginFallback identifier={identifier} source={source} reason="Unknown plugin identifier." />,
    };
  }
  const spec = pluginRegistry[pluginKey(legacy.pluginId, legacy.renderer)];
  if (!spec) {
    return {
      ok: false,
      content: <PluginFallback identifier={identifier} source={source} reason="Unknown plugin identifier." />,
    };
  }
  return renderSpecPayload(spec, source, partial, identifier);
}

function renderDailyPluginEnvelope(source: string, partial: boolean): PluginRenderResult {
  let envelope: DailyPluginEnvelope | null = null;
  try {
    const parsed = JSON.parse(source) as unknown;
    if (isDailyPluginEnvelope(parsed)) {
      envelope = parsed;
    }
  } catch {
    if (!partial) {
      return {
        ok: false,
        content: <PluginFallback identifier="daily-plugin" source={source} reason="Invalid JSON envelope." />,
      };
    }
  }

  if (!envelope) {
    if (partial) {
      return {
        ok: true,
        content: (
          <PluginShell
            identifier="daily-plugin"
            label="Plugin"
            icon={<LoaderCircle className="size-4 animate-spin" />}
            partial
            source={source}
          >
            <Skeleton className="h-24 rounded-[var(--radius-panel)]" />
          </PluginShell>
        ),
      };
    }
    return {
      ok: false,
      content: <PluginFallback identifier="daily-plugin" source={source} reason="Envelope must include pluginId, renderer, and payload." />,
    };
  }

  const spec = pluginRegistry[pluginKey(envelope.pluginId, envelope.renderer)];
  if (!spec) {
    return {
      ok: false,
      content: (
        <PluginFallback
          identifier={`${envelope.pluginId}:${envelope.renderer}`}
          source={source}
          reason="No renderer is registered for this plugin contribution."
        />
      ),
    };
  }

  if (partial) {
    if (spec.chrome === "bare") {
      return {
        ok: true,
        content: spec.skeleton,
      };
    }
    return {
      ok: true,
      content: (
        <PluginShell identifier={`${spec.pluginId}:${spec.renderer}`} label={spec.label} icon={spec.icon} partial source={source}>
          {spec.skeleton}
        </PluginShell>
      ),
    };
  }

  if (!spec.validate(envelope.payload)) {
    return {
      ok: false,
      content: (
        <PluginFallback
          identifier={`${envelope.pluginId}:${envelope.renderer}`}
          source={source}
          reason="Payload does not match the expected renderer shape."
        />
      ),
    };
  }

  return {
    ok: true,
    content:
      spec.chrome === "bare" ? (
        spec.render(envelope.payload)
      ) : (
        <PluginShell identifier={`${spec.pluginId}:${spec.renderer}`} label={spec.label} icon={spec.icon}>
          {spec.render(envelope.payload)}
        </PluginShell>
      ),
  };
}

function renderSpecPayload<T>(
  spec: PluginSpec<T>,
  source: string,
  partial: boolean,
  identifier = `${spec.pluginId}:${spec.renderer}`,
): PluginRenderResult {
  if (partial) {
    if (spec.chrome === "bare") {
      return {
        ok: true,
        content: spec.skeleton,
      };
    }
    return {
      ok: true,
      content: (
        <PluginShell identifier={identifier} label={spec.label} icon={spec.icon} partial source={source}>
          {spec.skeleton}
        </PluginShell>
      ),
    };
  }

  try {
    const payload = JSON.parse(source) as unknown;
    if (!spec.validate(payload)) {
      return {
        ok: false,
        content: (
          <PluginFallback
            identifier={identifier}
            source={source}
            reason="Payload does not match the expected shape."
          />
        ),
      };
    }
    return {
      ok: true,
      content:
        spec.chrome === "bare" ? (
          spec.render(payload)
        ) : (
          <PluginShell identifier={identifier} label={spec.label} icon={spec.icon}>
            {spec.render(payload)}
          </PluginShell>
        ),
    };
  } catch (error) {
    return {
      ok: false,
      content: (
        <PluginFallback
          identifier={identifier}
          source={source}
          reason={error instanceof Error ? error.message : "Invalid JSON payload."}
        />
      ),
    };
  }
}
