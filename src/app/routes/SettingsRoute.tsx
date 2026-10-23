import { lazy, Suspense, type ComponentProps } from "react";
import { useAppStore } from "@/useAppStore";
import { RouteLoading } from "@/app/chrome/RouteLoading";

const SettingsPage = lazy(() =>
  import("@/features/settings/SettingsPage").then((module) => ({ default: module.SettingsPage })),
);

export type SettingsRouteProps = Omit<
  ComponentProps<typeof SettingsPage>,
  "workspace" | "agentApps" | "plugins" | "settings" | "runtime"
>;

export function SettingsRoute(props: SettingsRouteProps) {
  const workspace = useAppStore((state) => state.workspace);
  const agentApps = useAppStore((state) => state.agentApps);
  const plugins = useAppStore((state) => state.plugins);
  const settings = useAppStore((state) => state.settings);
  const runtime = useAppStore((state) => state.runtime);

  if (!workspace || !plugins || !settings || !runtime) {
    return null;
  }
  return (
    <Suspense fallback={<RouteLoading label="Loading settings…" />}>
      <SettingsPage
        workspace={workspace}
        agentApps={agentApps}
        plugins={plugins}
        settings={settings}
        runtime={runtime}
        {...props}
      />
    </Suspense>
  );
}
