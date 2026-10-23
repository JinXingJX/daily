import { type AppRoute } from "@/useAppStore";

export function sameRoute(left: AppRoute, right: AppRoute) {
  if (left.kind !== right.kind) return false;
  if (left.kind === "core" && right.kind === "core") return left.page === right.page;
  return left.kind === "pluginView"
    && right.kind === "pluginView"
    && left.pluginId === right.pluginId
    && left.viewId === right.viewId;
}
