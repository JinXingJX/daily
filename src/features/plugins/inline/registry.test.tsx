import { describe, expect, it } from "vitest";
import { renderPluginBlock } from "@/features/plugins/inline/registry";

const chart = {
  pluginId: "builtin.chart",
  renderer: "chart",
  payload: { title: "Sales", type: "bar", labels: ["Q1"], series: [{ name: "A", data: [1] }] },
};

describe("renderPluginBlock", () => {
  it("renders registered contributions and legacy identifiers", () => {
    expect(renderPluginBlock("daily-plugin", JSON.stringify(chart), false).ok).toBe(true);
    expect(renderPluginBlock("plugin:chart", JSON.stringify(chart.payload), false).ok).toBe(true);
  });

  it("rejects unknown renderers and malformed payloads", () => {
    expect(renderPluginBlock("plugin:unknown", "{}", false).ok).toBe(false);
    expect(renderPluginBlock("daily-plugin", JSON.stringify({ ...chart, renderer: "nope" }), false).ok).toBe(false);
    expect(renderPluginBlock("daily-plugin", JSON.stringify({ ...chart, payload: { title: 1 } }), false).ok).toBe(false);
    expect(renderPluginBlock("daily-plugin", "{not json", false).ok).toBe(false);
  });

  it("shows a placeholder while a block is still streaming", () => {
    expect(renderPluginBlock("daily-plugin", '{"pluginId":"builtin.ch', true).ok).toBe(true);
  });
});
