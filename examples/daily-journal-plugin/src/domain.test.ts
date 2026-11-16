import { describe, expect, it } from "vitest";
import { activity, calendar, isEntry, localDate, markdown, newEntry, publicUrl, streak, wordCount } from "./domain";
import { parseInsight } from "./agent";
import type { AgentTaskResponse } from "@daily/plugin-sdk";

describe("journal content and activity", () => {
  it("counts mixed CJK and English, ignoring punctuation and emoji", () => {
    expect(wordCount("Hello世界，今天写了 hello-world 123！🙂")).toBe(9);
    expect(wordCount(" \n🙂，。! ")).toBe(0);
    expect(wordCount("abc中文def")).toBe(4);
  });
  it("aggregates the current text of all entries on the author's chosen local date", () => {
    const first = { ...newEntry("2026-11-15"), body: "你好" };
    const second = { ...newEntry("2026-11-15"), body: "hello world" };
    expect(activity([first, second]).get("2026-11-15")).toBe(4);
    expect(activity([{ ...first, body: "你" }, second]).get("2026-11-15")).toBe(3);
  });
  it("keeps heatmap columns aligned Sunday to Saturday across year boundaries", () => {
    const days = calendar(new Date(2026, 0, 1, 1), 53);
    expect(days).toHaveLength(371);
    expect(new Date(`${days[0]}T12:00:00`).getDay()).toBe(0);
    expect(new Date(`${days.at(-1)}T12:00:00`).getDay()).toBe(6);
    expect(days).toContain("2026-01-01");
    expect(new Set(days).size).toBe(days.length);
  });
  it("allows today to be unfinished when calculating a writing streak", () => {
    const day = new Date(2026, 10, 16);
    expect(streak(new Map([["2026-11-15", 4], ["2026-11-14", 3]]), day)).toBe(2);
    expect(streak(new Map([["2026-11-14", 3]]), day)).toBe(0);
    expect(localDate(day)).toBe("2026-11-16");
  });
  it("preserves text and actionable follow-ups in Markdown exports", () => {
    const entry = { ...newEntry(), title: "日记", body: "原文\n\n第二段", todos: [{ id: "one", title: "检验假设", kind: "verify" as const, due: "", evidence: "原文", done: false }] };
    expect(markdown(entry)).toContain("原文\n\n第二段");
    expect(markdown(entry)).toContain("- [ ] [verify] 检验假设");
    expect(isEntry(entry)).toBe(true);
    expect(isEntry({ ...entry, insights: { social: { body: 5 } } })).toBe(false);
  });
  it("rejects executable URLs and credentials before submitting a link", () => {
    expect(() => publicUrl("javascript:alert(1)")).toThrow();
    expect(() => publicUrl("file:///etc/passwd")).toThrow();
    expect(() => publicUrl("https://user:password@example.com")).toThrow();
    expect(publicUrl("https://example.com/article?q=1")).toBe("https://example.com/article?q=1");
  });
});

describe("agent results", () => {
  const entry = { ...newEntry(), body: "明天测试原型。我预测月底用户会增加。", revision: 4 };
  const response = (value: unknown, text = ""): AgentTaskResponse => ({ status: "completed", value, text, eventViews: [], iterationsUsed: 1, maxIterations: 3 });
  it("parses fenced JSON while retaining the revision the task actually analyzed", () => {
    const result = parseInsight(response(null, '```json\n{"summary":"原型实验","points":["明天测试"]}\n```'), "task", { entry, mode: "summary", language: "Chinese" });
    expect(result.revision).toBe(4);
    expect(result.summary).toBe("原型实验");
    expect(result.taskId).toBe("task");
  });
  it("accepts empty follow-ups without inventing tasks", () => {
    expect(parseInsight(response({ todos: [] }), "task", { entry, mode: "todos", language: "Chinese" }).todos).toEqual([]);
  });
  it("requires literal evidence and valid review dates for follow-ups", () => {
    const todo = { title: "测试原型", kind: "plan", due: "", evidence: "明天测试原型" };
    const parse = (value: unknown) => parseInsight(response({ todos: [value] }), "task", { entry, mode: "todos", language: "Chinese" });
    expect(parse(todo).todos[0].kind).toBe("plan");
    expect(() => parse({ ...todo, evidence: "虚构的承诺" })).toThrow();
    expect(() => parse({ ...todo, due: "2026-02-31" })).toThrow();
    expect(() => parse({ ...todo, kind: "done" })).toThrow();
  });
  it("rejects incomplete social drafts and unsupported gap actions", () => {
    const social = { title: "记录想法", body: "一个可以测试的观点", imagePrompt: "蓝色插画", gaps: [{ text: "补充数据", action: "find", query: "用户测试" }] };
    const parse = (value: unknown) => parseInsight(response(value), "task", { entry, mode: "social", language: "Chinese" });
    expect(parse(social).gaps[0].action).toBe("find");
    expect(() => parse({ ...social, gaps: [{ ...social.gaps[0], action: "publish" }] })).toThrow();
    expect(() => parse({ title: "only title" })).toThrow();
  });
});
