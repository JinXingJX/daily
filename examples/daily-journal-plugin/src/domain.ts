export type Source = { id: string; url: string; text: string };
export type TodoKind = "plan" | "verify" | "prediction";
export type Todo = { id: string; title: string; kind: TodoKind; due: string; evidence: string; done: boolean };
export type Gap = { text: string; action: "find" | "write" | "image"; query: string };
export type Insight = {
  taskId: string; mode: Mode; revision: number; createdAt: number;
  summary: string; points: string[]; title: string; body: string;
  imagePrompt: string; gaps: Gap[]; todos: Omit<Todo, "id" | "done">[];
};
export type Mode = "summary" | "social" | "todos";
export type Entry = {
  version: 1; id: string; date: string; title: string; body: string;
  createdAt: number; updatedAt: number; revision: number; sources: Source[];
  insights: Partial<Record<Mode, Insight>>; todos: Todo[];
  image?: { path: string; prompt: string; createdAt: number };
};
export function localDate(date = new Date()): string {
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}
export function newEntry(date = localDate()): Entry {
  return { version: 1, id: crypto.randomUUID(), date, title: "", body: "", createdAt: Date.now(), updatedAt: Date.now(), revision: 0, sources: [], insights: {}, todos: [] };
}
// CJK characters count individually; Latin/numeric runs count as words. Punctuation and emoji do not count.
export function wordCount(text: string): number {
  let cjk = 0;
  const rest = text.replace(/[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}]/gu, () => { cjk++; return " "; });
  return cjk + (rest.match(/[\p{L}\p{N}]+(?:['’\-][\p{L}\p{N}]+)*/gu) ?? []).length;
}
export function activity(entries: Entry[]): Map<string, number> {
  const totals = new Map<string, number>();
  for (const entry of entries) totals.set(entry.date, (totals.get(entry.date) ?? 0) + wordCount(entry.body));
  return totals;
}
export function calendar(end: Date, weeks: number): string[] {
  const day = new Date(end.getFullYear(), end.getMonth(), end.getDate(), 12);
  day.setDate(day.getDate() + (6 - day.getDay()) - (weeks * 7 - 1));
  return Array.from({ length: weeks * 7 }, () => { const key = localDate(day); day.setDate(day.getDate() + 1); return key; });
}
export function streak(totals: Map<string, number>, now = new Date()): number {
  const day = new Date(now.getFullYear(), now.getMonth(), now.getDate(), 12);
  if (!totals.get(localDate(day))) day.setDate(day.getDate() - 1);
  let days = 0;
  while ((totals.get(localDate(day)) ?? 0) > 0) { days++; day.setDate(day.getDate() - 1); }
  return days;
}
export function level(count: number): number { return count === 0 ? 0 : count < 100 ? 1 : count < 300 ? 2 : count < 800 ? 3 : 4; }
export function isEntry(value: unknown): value is Entry {
  if (!value || typeof value !== "object") return false;
  const v = value as Entry;
  return v.version === 1 && typeof v.id === "string" && /^[a-zA-Z0-9-]+$/.test(v.id) &&
    typeof v.date === "string" && /^\d{4}-\d{2}-\d{2}$/.test(v.date) &&
    typeof v.title === "string" && typeof v.body === "string" && Number.isFinite(v.revision) &&
    Number.isFinite(v.createdAt) && Number.isFinite(v.updatedAt) && Array.isArray(v.sources) &&
    v.sources.every(s => typeof s.id === "string" && typeof s.url === "string" && typeof s.text === "string") &&
    !!v.insights && typeof v.insights === "object" && Object.entries(v.insights).every(([mode, insight]) =>
      ["summary", "social", "todos"].includes(mode) && insight && insight.mode === mode &&
      typeof insight.taskId === "string" && Number.isFinite(insight.revision) && Number.isFinite(insight.createdAt) &&
      [insight.summary, insight.title, insight.body, insight.imagePrompt].every(x => typeof x === "string") &&
      Array.isArray(insight.points) && insight.points.every(x => typeof x === "string") &&
      Array.isArray(insight.gaps) && insight.gaps.every(g => g && typeof g.text === "string" && typeof g.query === "string" && ["find", "write", "image"].includes(g.action)) &&
      Array.isArray(insight.todos) && insight.todos.every(t => t && typeof t.title === "string" && ["plan", "verify", "prediction"].includes(t.kind) && typeof t.due === "string" && typeof t.evidence === "string")
    ) && (!v.image || (typeof v.image.path === "string" && typeof v.image.prompt === "string" && Number.isFinite(v.image.createdAt))) && Array.isArray(v.todos) &&
    v.todos.every(t => typeof t.id === "string" && typeof t.title === "string" && ["plan", "verify", "prediction"].includes(t.kind) && typeof t.due === "string" && typeof t.evidence === "string" && typeof t.done === "boolean");
}
export function markdown(entry: Entry): string {
  return `# ${entry.title || entry.date}\n\n${entry.date}\n\n${entry.body}\n\n` +
    (entry.sources.length ? `## Sources\n\n${entry.sources.map(s => `${s.url}\n\n${s.text}`).join("\n\n")}\n\n` : "") +
    (entry.insights.summary ? `## Summary\n\n${entry.insights.summary.summary}\n\n` : "") +
    (entry.todos.length ? `## Follow-ups\n\n${entry.todos.map(t => `- [${t.done ? "x" : " "}] [${t.kind}] ${t.title}${t.due ? ` (${t.due})` : ""}`).join("\n")}\n` : "");
}
export function publicUrl(value: string): string {
  const url = new URL(value);
  if (!["http:", "https:"].includes(url.protocol) || url.username || url.password) throw new Error("Use a public HTTP or HTTPS link.");
  return url.href;
}
