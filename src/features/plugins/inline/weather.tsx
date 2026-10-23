import { CloudSun } from "lucide-react";
import { Skeleton } from "@/components/ui/skeleton";
import { isRecord, type PluginSpec } from "@/features/plugins/inline/types";

export type WeatherForecastItem = {
  label: string;
  value: string;
};

export type WeatherPayload = {
  location: string;
  condition: string;
  temperatureC: number;
  forecast?: WeatherForecastItem[];
};

function isWeatherPayload(value: unknown): value is WeatherPayload {
  if (!isRecord(value)) {
    return false;
  }
  if (
    typeof value.location !== "string" ||
    typeof value.condition !== "string" ||
    typeof value.temperatureC !== "number"
  ) {
    return false;
  }
  if (value.forecast === undefined) {
    return true;
  }
  return (
    Array.isArray(value.forecast) &&
    value.forecast.every(
      (item) => isRecord(item) && typeof item.label === "string" && typeof item.value === "string",
    )
  );
}

function WeatherCard({ payload }: { payload: WeatherPayload }) {
  return (
    <div className="space-y-5">
      <div className="flex items-start justify-between gap-4">
        <div className="space-y-2">
          <div className="text-sm font-medium text-[var(--text-muted)]">{payload.location}</div>
          <div className="text-2xl font-semibold text-[var(--text-strong)]">{payload.condition}</div>
        </div>
        <div className="text-3xl font-semibold text-[var(--text-strong)]">{payload.temperatureC}°C</div>
      </div>
      {payload.forecast?.length ? (
        <div className="grid gap-3 md:grid-cols-[repeat(auto-fit,minmax(120px,1fr))]">
          {payload.forecast.map((item) => (
            <div key={item.label} className="border-b border-[var(--line)] py-3 last:border-b-0">
              <div className="text-xs font-medium text-[var(--text-muted)]">{item.label}</div>
              <div className="mt-1 font-medium text-[var(--text-strong)]">{item.value}</div>
            </div>
          ))}
        </div>
      ) : null}
    </div>
  );
}

function WeatherSkeleton() {
  return (
    <div className="space-y-5">
      <div className="flex items-start justify-between gap-4">
        <div className="space-y-2">
          <Skeleton className="h-4 w-28" />
          <Skeleton className="h-8 w-44" />
        </div>
        <Skeleton className="h-10 w-20" />
      </div>
      <div className="grid gap-3 md:grid-cols-3">
        {Array.from({ length: 3 }).map((_, index) => (
          <Skeleton className="h-20 rounded-[var(--radius-panel)]" key={index} />
        ))}
      </div>
    </div>
  );
}


export const weatherSpec: PluginSpec<WeatherPayload> = {
  pluginId: "builtin.weather",
  renderer: "weather",
  legacyIdentifier: "plugin:weather",
  label: "Weather",
  icon: <CloudSun className="size-4" />,
  validate: isWeatherPayload,
  render: (payload: WeatherPayload) => <WeatherCard payload={payload} />,
  skeleton: <WeatherSkeleton />,
};
