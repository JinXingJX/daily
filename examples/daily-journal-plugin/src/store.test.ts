import { describe, expect, it, vi } from "vitest";
import type { DailyPluginApi } from "@daily/plugin-sdk";
import { newEntry } from "./domain";
import { JournalStore } from "./store";

function setup() {
  const files = new Map<string, string>();
  const writeText = vi.fn(async (path: string, text: string) => { files.set(path, text); });
  const setDirty = vi.fn(async () => ({}));
  const host = { files: {
    ensureDir: vi.fn(async () => ({})),
    list: vi.fn(async () => ({ entries: [...files.keys()].map(path => ({ name: path.split("/").at(-1)!, isDirectory: false })) })),
    readText: vi.fn(async (path: string) => files.get(path)!), writeText,
  }, host: { setDirty } } as unknown as DailyPluginApi;
  return { files, writeText, setDirty, host, store: new JournalStore(host) };
}
describe("local journal persistence", () => {
  it("autosaves and restores complete Unicode text and line breaks", async () => {
    const { store, host, setDirty } = setup(); await store.load();
    const entry = store.create(); store.update(entry.id, { title: "中文", body: "🙂\n\n随便粘贴\tMarkdown **bold**" });
    await store.flush();
    expect(setDirty).toHaveBeenLastCalledWith(false);
    const reopened = new JournalStore(host); await reopened.load();
    expect(reopened.getSnapshot().entries[0].body).toBe("🙂\n\n随便粘贴\tMarkdown **bold**");
    store.dispose(); reopened.dispose();
  });
  it("retains unsaved changes and the close guard when a disk write fails", async () => {
    const { store, writeText, setDirty, files } = setup(); await store.load();
    const entry = store.create(); store.update(entry.id, { body: "不能丢" });
    writeText.mockRejectedValueOnce(new Error("Disk full"));
    await expect(store.flush()).rejects.toThrow("Disk full");
    expect(store.getSnapshot().saving).toBe(true);
    expect(setDirty).toHaveBeenLastCalledWith(true);
    await store.flush();
    expect(JSON.parse(files.get(`entries/${entry.id}.json`)!).body).toBe("不能丢");
    expect(setDirty).toHaveBeenLastCalledWith(false);
    store.dispose();
  });
  it("serializes saves so edits made during an in-flight write win", async () => {
    const { store, writeText, files } = setup(); await store.load();
    const entry = store.create(); store.update(entry.id, { body: "first" });
    let release!: () => void;
    writeText.mockImplementationOnce(async (path, text) => { await new Promise<void>(resolve => { release = resolve; }); files.set(path, text); });
    const first = store.flush(); await vi.waitFor(() => expect(release).toBeDefined());
    store.update(entry.id, { body: "latest" }); const second = store.flush();
    release(); await first; await second;
    expect(JSON.parse(files.get(`entries/${entry.id}.json`)!).body).toBe("latest");
    expect(store.getSnapshot().saving).toBe(false); store.dispose();
  });
  it("keeps corrupted entries untouched and loads valid siblings", async () => {
    const { store, files, writeText } = setup(); const good = newEntry();
    files.set(`entries/${good.id}.json`, JSON.stringify(good)); files.set("entries/broken.json", "{broken");
    await store.load();
    expect(store.getSnapshot().ready).toBe(true);
    expect(store.getSnapshot().entries).toHaveLength(1);
    expect(store.getSnapshot().warnings).toEqual(["broken.json"]);
    expect(writeText).not.toHaveBeenCalled(); expect(files.get("entries/broken.json")).toBe("{broken"); store.dispose();
  });
  it("updates metadata without invalidating the analyzed source revision", async () => {
    const { store } = setup(); await store.load(); const entry = store.create();
    store.update(entry.id, { body: "source" });
    const revision = store.getSnapshot().entries[0].revision;
    store.update(entry.id, { todos: [] });
    expect(store.getSnapshot().entries[0].revision).toBe(revision);
    store.update(entry.id, { sources: [{ id: "one", url: "", text: "extra" }] });
    expect(store.getSnapshot().entries[0].revision).toBe(revision + 1); await store.flush(); store.dispose();
  });
});
