import type { DailyPluginApi } from "@daily/plugin-sdk";
import { isEntry, newEntry, type Entry } from "./domain";

export type Snapshot = { entries: Entry[]; ready: boolean; saving: boolean; error: string | null; warnings: string[] };
export class JournalStore {
  private state: Snapshot = { entries: [], ready: false, saving: false, error: null, warnings: [] };
  private listeners = new Set<() => void>();
  private pending = new Map<string, Entry>();
  private timer: ReturnType<typeof setTimeout> | undefined;
  private queue: Promise<void> = Promise.resolve();
  private disposed = false;
  constructor(private host: DailyPluginApi) {}
  getSnapshot = () => this.state;
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  private emit(patch: Partial<Snapshot>) { this.state = { ...this.state, ...patch }; this.listeners.forEach(fn => fn()); }
  async load() {
    try {
      await this.host.files.ensureDir("entries");
      const { entries } = await this.host.files.list("entries");
      const loaded: Entry[] = [];
      const failures: string[] = [];
      // Bounded batches avoid flooding the JSONL bridge for large journals.
      const files = entries.filter(f => !f.isDirectory && /^[a-zA-Z0-9-]+\.json$/.test(f.name));
      for (let i = 0; i < files.length; i += 16) {
        await Promise.all(files.slice(i, i + 16).map(async file => {
          try {
            const entry: unknown = JSON.parse(await this.host.files.readText(`entries/${file.name}`));
            if (!isEntry(entry) || file.name !== `${entry.id}.json`) throw new Error("Invalid journal entry");
            loaded.push(entry);
          } catch { failures.push(file.name); }
        }));
      }
      if (!this.disposed) this.emit({ entries: loaded.sort((a, b) => b.createdAt - a.createdAt), ready: true, error: null, warnings: failures });
    } catch (error) { if (!this.disposed) this.emit({ error: String(error), ready: false }); }
  }
  create(date?: string): Entry {
    const entry = newEntry(date);
    this.emit({ entries: [entry, ...this.state.entries] });
    this.schedule(entry);
    return entry;
  }
  update(id: string, patch: Partial<Pick<Entry, "title" | "body" | "date" | "sources" | "insights" | "todos" | "image">>) {
    const entry = this.state.entries.find(e => e.id === id);
    if (!entry) return;
    const changed = "body" in patch || "title" in patch || "sources" in patch || "date" in patch;
    const next = { ...entry, ...patch, updatedAt: Date.now(), revision: entry.revision + (changed ? 1 : 0) };
    this.emit({ entries: this.state.entries.map(e => e.id === id ? next : e) });
    this.schedule(next);
  }
  private schedule(entry: Entry) {
    this.pending.set(entry.id, entry);
    this.emit({ saving: true });
    void this.host.host.setDirty(true);
    clearTimeout(this.timer);
    this.timer = setTimeout(() => { void this.flush().catch(() => undefined); }, 450);
  }
  flush = (): Promise<void> => {
    clearTimeout(this.timer);
    this.queue = this.queue.catch(() => undefined).then(async () => {
      for (const [id, entry] of [...this.pending]) {
        try {
          await this.host.files.writeText(`entries/${id}.json`, JSON.stringify(entry));
          if (this.pending.get(id) === entry) this.pending.delete(id);
        } catch (error) {
          this.emit({ saving: true, error: String(error) });
          throw error;
        }
      }
      if (!this.pending.size) { await this.host.host.setDirty(false); this.emit({ saving: false }); }
    });
    return this.queue;
  };
  clearError() { this.emit({ error: null }); }
  dispose() { this.disposed = true; clearTimeout(this.timer); void this.flush().catch(() => undefined); this.listeners.clear(); }
}
