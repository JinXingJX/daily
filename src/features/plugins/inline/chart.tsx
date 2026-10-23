import { motion } from "motion/react";
import { BarChart3 } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { isRecord, type PluginSpec } from "@/features/plugins/inline/types";

export type ChartSeries = {
  name: string;
  data: number[];
};

export type ChartPayload = {
  title: string;
  type: "bar" | "line" | "area";
  labels: string[];
  series: ChartSeries[];
};

function isChartPayload(value: unknown): value is ChartPayload {
  if (!isRecord(value)) {
    return false;
  }
  if (typeof value.title !== "string" || !Array.isArray(value.labels) || !Array.isArray(value.series)) {
    return false;
  }
  return value.series.every(
    (item) =>
      isRecord(item) &&
      typeof item.name === "string" &&
      Array.isArray(item.data) &&
      item.data.every((point) => typeof point === "number"),
  );
}

function ChartCard({ payload }: { payload: ChartPayload }) {
  const series = payload.series[0] ?? { name: "Series", data: [] };
  const maxValue = Math.max(...series.data, 1);
  return (
    <div className="space-y-5">
      <div className="flex items-end justify-between gap-4">
        <div>
          <div className="text-lg font-semibold text-[var(--text-strong)]">{payload.title}</div>
          <div className="text-sm text-[var(--text-muted)]">Primary Series: {series.name}</div>
        </div>
        <Badge variant="secondary">{titleCaseToken(payload.type)}</Badge>
      </div>
      <div className="grid min-h-48 grid-cols-[repeat(auto-fit,minmax(72px,1fr))] gap-4">
        {payload.labels.map((label, index) => {
          const value = series.data[index] ?? 0;
          const height = `${Math.max((value / maxValue) * 100, 8)}%`;
          return (
            <div className="grid justify-items-center gap-2" key={`${label}-${index}`}>
              <div className="flex min-h-36 w-full items-end rounded-[var(--radius-control)] bg-[var(--surface-muted)] p-1.5">
                <motion.div
                  initial={{ height: 0 }}
                  animate={{ height }}
                  transition={{ duration: 0.22, ease: [0.22, 1, 0.36, 1] }}
                  className="w-full rounded-[var(--radius-control)] bg-[var(--accent)]"
                />
              </div>
              <div className="font-mono text-sm text-[var(--text-strong)]">{value}</div>
              <div className="text-xs text-[var(--text-muted)]">{label}</div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function ChartSkeleton() {
  return (
    <div className="space-y-5">
      <div className="flex items-center justify-between gap-4">
        <div className="space-y-2">
          <Skeleton className="h-5 w-32" />
          <Skeleton className="h-4 w-24" />
        </div>
        <Skeleton className="h-6 w-16 rounded-[var(--radius-full)]" />
      </div>
      <div className="grid min-h-48 grid-cols-[repeat(auto-fit,minmax(72px,1fr))] gap-4">
        {Array.from({ length: 5 }).map((_, index) => (
          <div className="grid justify-items-center gap-2" key={index}>
            <div className="flex min-h-36 w-full items-end rounded-[var(--radius-control)] bg-[var(--surface-muted)] p-1.5">
              <Skeleton className="w-full rounded-[var(--radius-control)]" style={{ height: `${30 + index * 10}%` }} />
            </div>
            <Skeleton className="h-4 w-10" />
            <Skeleton className="h-3 w-8" />
          </div>
        ))}
      </div>
    </div>
  );
}


function titleCaseToken(value: string) {
  return value
    .split(/[_\s-]+/)
    .filter(Boolean)
    .map((part) => `${part.slice(0, 1).toUpperCase()}${part.slice(1)}`)
    .join(" ");
}

export const chartSpec: PluginSpec<ChartPayload> = {
  pluginId: "builtin.chart",
  renderer: "chart",
  legacyIdentifier: "plugin:chart",
  label: "Chart",
  icon: <BarChart3 className="size-4" />,
  validate: isChartPayload,
  render: (payload: ChartPayload) => <ChartCard payload={payload} />,
  skeleton: <ChartSkeleton />,
};
