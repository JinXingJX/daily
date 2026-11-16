import { useCallback, useDeferredValue, useEffect, useMemo, useRef, useState, useSyncExternalStore, type ReactNode } from "react";
import { BookOpen, Plus, Search, ArrowUpRight, Check, CheckCheck, ListTodo, PanelRight, X, Download, Maximize2, Minimize2, Sparkles, LoaderCircle, Link, Trash2, ChevronDown, ChevronRight, Feather, RotateCcw, Circle, ExternalLink, Copy, Image as ImageIcon, CalendarDays } from "lucide-react";
import { dailyHost, observeTasks, taskIsActive, type PluginViewContext, type TaskSnapshot } from "@daily/plugin-sdk";
import { t, locale, message } from "@/i18n";
import { activity, calendar, level, localDate, markdown, publicUrl, streak, wordCount, type Entry, type Insight, type Mode, type TodoKind } from "./domain";
import { kinds, submitAnalysis } from "./agent";
import { JournalStore } from "./store";
import { download, quoteCards } from "./export";

type Effect = "quiet" | "ripple" | "glow";
type Tab = "entries" | "todos";
function IconButton({ label, children, onClick, active = false, disabled = false }: { label: string; children: ReactNode; onClick: () => void; active?: boolean; disabled?: boolean }) {
  return <button className={`icon-button${active ? " selected" : ""}`} title={label} aria-label={label} aria-pressed={active} onClick={onClick} disabled={disabled}>{children}</button>;
}
const kindLabel = (kind: TodoKind) => kind === "plan" ? t("Plan") : kind === "verify" ? t("Verify") : t("Prediction");
const language = () => locale() === "zh-CN" ? "Simplified Chinese" : "English";

function Heatmap({ entries, full = false, onDate }: { entries: Entry[]; full?: boolean; onDate: (date: string) => void }) {
  const totals = useMemo(() => activity(entries), [entries]);
  const today = localDate();
  const days = calendar(new Date(), full ? 53 : 13);
  return <div className={full ? "heatmap full" : "heatmap"}>
    <div className="heatmap-grid" style={{ gridTemplateColumns: `repeat(${full ? 53 : 13}, minmax(0, 1fr))` }}>
      {days.map(date => <button key={date} className={`heat-cell heat-${level(totals.get(date) ?? 0)}${date === today ? " today" : ""}`} disabled={date > today}
        title={`${date}: ${t("{count} words", { count: totals.get(date) ?? 0 })}`} aria-label={`${date}: ${t("{count} words", { count: totals.get(date) ?? 0 })}`} onClick={() => onDate(date)} />)}
    </div>
    <div className="heat-legend"><span>{full ? `${days[0]} – ${today}` : t("Past 13 weeks")}</span><span>{t("Less")} {[0, 1, 2, 3, 4].map(i => <i key={i} className={`heat-cell heat-${i}`} />)} {t("More")}</span></div>
  </div>;
}

export function App({ store, view }: { store: JournalStore; view: PluginViewContext }) {
  const state = useSyncExternalStore(store.subscribe, store.getSnapshot);
  const [selected, setSelected] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [dateFilter, setDateFilter] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>("entries");
  const [desk, setDesk] = useState(true);
  const [focus, setFocus] = useState(false);
  const [year, setYear] = useState(false);
  const [effect, setEffect] = useState<Effect>("ripple");
  const [error, setError] = useState<string | null>(null);
  const [, refreshLocale] = useState(0);
  const [visible, setVisible] = useState(view.visible);
  const deferredEntries = useDeferredValue(state.entries);
  const totals = useMemo(() => activity(deferredEntries), [deferredEntries]);
  const query = useDeferredValue(search.trim().toLowerCase());
  const entry = state.entries.find(e => e.id === selected) ?? state.entries[0];
  const filtered = deferredEntries.filter(e => (!dateFilter || e.date === dateFilter) && `${e.title}\n${e.body}`.toLowerCase().includes(query));
  const openCount = state.entries.reduce((sum, e) => sum + e.todos.filter(x => !x.done).length, 0);
  const run = useCallback((fn: () => Promise<unknown>) => { void fn().catch(e => setError(message(e))); }, []);
  useEffect(() => {
    const observer = new MutationObserver(() => refreshLocale(n => n + 1));
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ["lang"] });
    return () => observer.disconnect();
  }, []);
  useEffect(() => view.onVisibilityChange(next => { setVisible(next); if (!next) void store.flush().catch(e => setError(message(e))); }), [view, store]);
  useEffect(() => {
    void dailyHost().storage.get("writing-effect").then(value => { if (value === "quiet" || value === "ripple" || value === "glow") setEffect(value); }).catch(() => undefined);
  }, []);
  function create(date?: string) { const next = store.create(date); setSelected(next.id); setTab("entries"); setDateFilter(null); setSearch(""); }
  function selectDate(date: string) {
    setDateFilter(date); setTab("entries");
    const first = state.entries.find(e => e.date === date);
    if (first) setSelected(first.id);
  }
  function select(id: string) { setSelected(id); setTab("entries"); }
  if (!state.ready) return <main className="boot"><BookOpen size={36} /><p>{state.error ? message(state.error) : t("Loading journal…")}</p>{state.error && <button onClick={() => run(() => store.load())}>{t("Retry")}</button>}</main>;
  return <main className={`journal${focus ? " focus" : ""}${desk ? " with-desk" : ""}`} onKeyDown={e => {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "s") { e.preventDefault(); run(store.flush); }
    if (e.key === "Escape") setFocus(false);
  }}>
    {!focus && <aside className="library">
      <header className="library-heading"><span className="journal-mark"><Feather size={21} /></span><h1>{t("Journal")}</h1><span className="entry-total">{state.entries.length}</span></header>
      <p className="library-subtitle">{t("A little space for your thoughts.")}</p>
      <button className="new-entry primary" onClick={() => create()}><Plus size={17} />{t("New entry")}</button>
      <div className="library-tabs"><button className={tab === "entries" ? "active" : ""} onClick={() => setTab("entries")}><BookOpen size={15} />{t("All entries")}</button><button className={tab === "todos" ? "active" : ""} onClick={() => setTab("todos")}><ListTodo size={15} />{t("Follow-ups")}<small>{openCount}</small></button></div>
      <label className="search"><Search size={15} /><input value={search} onChange={e => setSearch(e.target.value)} aria-label={t("Search entries")} placeholder={t("Search entries")} /></label>
      {dateFilter && <button className="date-filter" onClick={() => setDateFilter(null)}>{dateFilter}<X size={13} aria-label={t("Clear date filter")} /></button>}
      <nav className="entry-list" aria-label={t("All entries")}>{filtered.length ? filtered.map(e => <button key={e.id} className={`entry-item${entry?.id === e.id ? " active" : ""}`} onClick={() => select(e.id)}>
        <span className="entry-date">{e.date}<span>{t("{count} words", { count: wordCount(e.body) })}</span></span>
        <strong>{e.title || t("Untitled")}</strong><p>{e.body.slice(0, 90) || t("Start with whatever is on your mind.")}</p>
      </button>) : <div className="list-empty">{state.entries.length ? t("No matching entries") : t("Nothing here yet")}{dateFilter && <button onClick={() => create(dateFilter)}>{t("Create a new entry for this date")}</button>}</div>}</nav>
      <section className="activity-mini"><div className="section-line"><strong>{t("Writing activity")}</strong><IconButton label={t("Show full year")} onClick={() => setYear(!year)} active={year}><ArrowUpRight size={15} /></IconButton></div><Heatmap entries={deferredEntries} onDate={selectDate} /><p className="streak">{t("{count} day streak", { count: streak(totals) })}<span>{t("{count} writing days", { count: [...totals.values()].filter(n => n > 0).length })}</span></p></section>
    </aside>}
    <section className="workspace">
      <header className="toolbar"><div className="save-state" role="status">{state.saving ? <LoaderCircle size={13} className="spin" /> : <CheckCheck size={14} />}<span>{state.saving ? t("Saving…") : t("All changes saved")}</span></div><select className="compact-entry-select" aria-label={t("All entries")} value={entry?.id ?? ""} onChange={e => select(e.target.value)}>{!entry && <option value="">{t("Nothing here yet")}</option>}{state.entries.map(e => <option value={e.id} key={e.id}>{e.date} {e.title || t("Untitled")}</option>)}</select><div className="toolbar-actions">
        <label className="effect-select" title={t("Writing effects")}><Sparkles size={14} /><select aria-label={t("Writing effects")} value={effect} onChange={e => { const next = e.target.value as Effect; setEffect(next); run(() => dailyHost().storage.set("writing-effect", next)); }}><option value="quiet">{t("Quiet")}</option><option value="ripple">{t("Ink ripple")}</option><option value="glow">{t("Soft glow")}</option></select><ChevronDown size={11} /></label>
        <IconButton label={t("Export Markdown")} disabled={!entry} onClick={() => { if (entry) download(new Blob([markdown(entry)], { type: "text/markdown;charset=utf-8" }), `journal-${entry.date}-${entry.id.slice(0, 8)}.md`); }}><Download size={16} /></IconButton>
        <IconButton label={focus ? t("Exit focus") : t("Focus mode")} onClick={() => setFocus(!focus)} active={focus}>{focus ? <Minimize2 size={16} /> : <Maximize2 size={16} />}</IconButton>
        {!focus && <IconButton label={desk ? t("Close agent desk") : t("Open agent desk")} active={desk} onClick={() => setDesk(!desk)}><PanelRight size={17} /></IconButton>}
      </div></header>
      {(error || state.error) && <div className="error-banner" role="alert"><div>{state.error && <strong>{t("Could not save your changes. Retry before closing.")} </strong>}{error || message(state.error)}</div>{state.error && <button onClick={() => run(async () => { await store.flush(); store.clearError(); })}>{t("Retry")}</button>}<IconButton label={t("Dismiss")} onClick={() => { setError(null); if (!state.saving) store.clearError(); }}><X size={15} /></IconButton></div>}
      {state.warnings.length > 0 && <div className="error-banner" role="alert"><div>{t("Some entries could not be opened. Original files were kept.")}<details><summary>{state.warnings.length}</summary>{state.warnings.join(", ")}</details></div></div>}
      {year && !focus && <section className="activity-year"><div className="section-line"><h2>{t("Writing this year")}</h2><IconButton label={t("Close activity")} onClick={() => setYear(false)}><X size={16} /></IconButton></div><Heatmap entries={deferredEntries} full onDate={selectDate} /></section>}
      {tab === "todos" && !focus ? <section className="all-todos"><h2>{t("Follow-ups")}<span>{openCount}</span></h2>{!state.entries.some(e => e.todos.length) && <p className="muted">{t("No to-dos yet. Extract them from an entry.")}</p>}{state.entries.filter(e => e.todos.length).map(e => <section key={e.id} className="todo-group"><button className="source-title" title={t("Open source entry")} onClick={() => select(e.id)}>{e.title || t("Untitled")}<span>{e.date}</span><ArrowUpRight size={14} /></button>{e.todos.map(todo => <label className={`todo-item${todo.done ? " done" : ""}`} key={todo.id}><input type="checkbox" checked={todo.done} onChange={() => store.update(e.id, { todos: e.todos.map(x => x.id === todo.id ? { ...x, done: !x.done } : x) })} /><span><strong>{todo.title}</strong><small><span className={`kind ${todo.kind}`}>{kindLabel(todo.kind)}</span>{todo.due}</small></span></label>)}</section>)}</section> : entry ? <Editor key={entry.id} entry={entry} store={store} effect={visible ? effect : "quiet"} /> : <section className="welcome"><div className="welcome-page"><Feather size={33} /><h2>{t("Start with whatever is on your mind.")}</h2><p>{t("Write anything. Paste anything. Make it yours.")}</p><button className="primary" onClick={() => create()}><Plus size={17} />{t("New entry")}</button></div></section>}
      <footer className="editor-footer"><span><Circle size={6} fill="currentColor" />{t("Writing saved locally in this workspace")}</span><span>{entry ? t("{count} words", { count: wordCount(entry.body) }) : ""}</span></footer>
    </section>
    {desk && !focus && entry && <AgentDesk key={entry.id} entry={entry} store={store} visible={visible} run={run} onClose={() => setDesk(false)} />}
  </main>;
}

function Editor({ entry, store, effect }: { entry: Entry; store: JournalStore; effect: Effect }) {
  const editor = useRef<HTMLTextAreaElement>(null);
  const mirror = useRef<HTMLDivElement>(null);
  const [pulse, setPulse] = useState<{ x: number; y: number; id: number } | null>(null);
  const sequence = useRef(0);
  const composing = useRef(false);
  const body = useDeferredValue(entry.body);
  function feedback() {
    const el = editor.current; const target = mirror.current;
    if (!el || !target || effect === "quiet" || matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    // A mirror measures the actual insertion point, including wrapping and textarea scrolling.
    target.style.width = `${el.clientWidth}px`;
    target.textContent = el.value.slice(0, el.selectionStart);
    const marker = document.createElement("span"); marker.textContent = "\u200b"; target.append(marker);
    setPulse({ x: 10 + Math.min(el.clientWidth - 12, marker.offsetLeft - el.scrollLeft), y: marker.offsetTop - el.scrollTop + 17, id: ++sequence.current });
  }
  return <article className="entry-editor">
    <div className="entry-paper"><div className="entry-dateline"><CalendarDays size={14} /><input type="date" aria-label={t("Entry date")} value={entry.date} max={localDate()} onChange={e => { if (e.target.value) store.update(entry.id, { date: e.target.value }); }} /><span>{t("{count} words", { count: wordCount(body) })}</span></div>
      <input className="title-input" aria-label={t("Entry title")} placeholder={t("Untitled")} value={entry.title} onChange={e => store.update(entry.id, { title: e.target.value })} />
      <div className="writing-area"><textarea ref={editor} className="body-input" aria-label={t("Entry text")} placeholder={t("Write anything. Paste anything. Make it yours.")} value={entry.body} spellCheck onChange={e => { store.update(entry.id, { body: e.target.value }); if (!composing.current) feedback(); }} onCompositionStart={() => { composing.current = true; }} onCompositionEnd={() => { composing.current = false; feedback(); }} /><div ref={mirror} className="caret-mirror" aria-hidden="true" />{pulse && <span key={pulse.id} className={`typing-pulse ${effect}`} style={{ left: pulse.x, top: pulse.y }} onAnimationEnd={() => setPulse(null)} />}</div>
    </div>
  </article>;
}

function AgentDesk({ entry, store, visible, run, onClose }: { entry: Entry; store: JournalStore; visible: boolean; run: (fn: () => Promise<unknown>) => void; onClose: () => void }) {
  const [mode, setMode] = useState<Mode>("summary");
  const [tasks, setTasks] = useState<TaskSnapshot[]>([]);
  const [submitting, setSubmitting] = useState(false);
  const [url, setUrl] = useState("");
  const [notes, setNotes] = useState("");
  const [materials, setMaterials] = useState(false);
  const [copied, setCopied] = useState(false);
  const [imageUrl, setImageUrl] = useState<string | null>(null);
  const sourceRef = useRef<HTMLTextAreaElement>(null);
  const insight = entry.insights[mode];
  const sorted = [...tasks].sort((a, b) => b.createdAt - a.createdAt);
  const task = sorted.find(x => x.kind === kinds[mode]);
  const auxiliary = sorted.filter(x => x.kind === "journal.image" || x.kind === "journal.source");
  const busy = submitting || !!(task && taskIsActive(task));
  useEffect(() => { if (visible) return observeTasks({ scope: entry.id }, setTasks, e => console.error("Journal task observer:", e)); }, [entry.id, visible]);
  useEffect(() => {
    if (!entry.image?.path) { setImageUrl(null); return; }
    let active = true; let resource: string | undefined;
    void dailyHost().files.readBase64(entry.image.path).then(base64 => {
      const bytes = Uint8Array.from(atob(base64), c => c.charCodeAt(0));
      resource = URL.createObjectURL(new Blob([bytes], { type: "image/png" }));
      if (active) setImageUrl(resource); else URL.revokeObjectURL(resource);
    }).catch(e => console.error("Journal image:", e));
    return () => { active = false; if (resource) URL.revokeObjectURL(resource); };
  }, [entry.image?.path]);
  async function analyze() {
    setSubmitting(true);
    try { await store.flush(); const next = await submitAnalysis(entry, mode, language()); setTasks(prev => [next, ...prev.filter(x => x.id !== next.id)]); }
    finally { setSubmitting(false); }
  }
  function updateInsight(patch: Partial<Insight>) { if (insight) store.update(entry.id, { insights: { ...entry.insights, [mode]: { ...insight, ...patch } } }); }
  async function source() {
    const validated = publicUrl(url); await store.flush();
    const task = await dailyHost().tasks.submit({ kind: "journal.source", title: validated, scope: entry.id, exclusiveKey: `${entry.id}:source`, requestKey: crypto.randomUUID(), input: { entryId: entry.id, url: validated, language: language() } });
    setTasks(prev => [task, ...prev]); setUrl("");
  }
  async function generateImage(prompt: string) {
    if (!prompt.trim()) return;
    if (!(await dailyHost().image.status()).configured) throw new Error("Configure an OpenAI image key in Daily settings first.");
    const task = await dailyHost().tasks.submit({ kind: "journal.image", title: entry.title || entry.date, scope: entry.id, exclusiveKey: `${entry.id}:image`, requestKey: crypto.randomUUID(), input: { entryId: entry.id, prompt } });
    setTasks(prev => [task, ...prev]);
  }
  const imageBusy = auxiliary.some(task => task.kind === "journal.image" && taskIsActive(task));
  const sourceBusy = auxiliary.some(task => task.kind === "journal.source" && taskIsActive(task));
  const modes: Mode[] = ["summary", "social", "todos"];
  const labels = { summary: t("Summary"), social: t("Social draft"), todos: t("To-dos") };
  const descriptions = { summary: t("A second look at what you wrote."), social: t("Keep your voice. Find a shareable point of view."), todos: t("Plans, questions and predictions deserve a follow-up.") };
  const action = { summary: t("Summarize"), social: t("Create social draft"), todos: t("Extract follow-ups") };
  return <aside className="agent-desk"><header className="desk-heading"><span><Sparkles size={16} />{t("Agent desk")}</span><IconButton label={t("Close agent desk")} onClick={onClose}><X size={16} /></IconButton></header><div className="desk-tabs" role="tablist" aria-label={t("Agent desk")}>{modes.map(item => <button key={item} role="tab" aria-selected={mode === item} className={mode === item ? "active" : ""} onClick={() => setMode(item)}>{labels[item]}</button>)}</div>
    <div className="desk-scroll">
      <div className="agent-intro"><p>{descriptions[mode]}</p><button className="agent-run" disabled={busy || !entry.body.trim()} title={!entry.body.trim() ? t("Write something first") : action[mode]} onClick={() => run(analyze)}>{busy ? <LoaderCircle size={15} className="spin" /> : insight ? <RotateCcw size={14} /> : <Sparkles size={15} />}{busy ? t("Working in the background…") : insight ? t("Run again") : action[mode]}</button></div>
      {task && <TaskStatus task={task} run={run} />}
      {insight && <div className="insight">
        {insight.revision !== entry.revision && <p className="stale-notice">{t("Entry changed since this result. Run again to refresh.")}</p>}
        {mode === "summary" && <><p className="summary-text">{insight.summary}</p>{insight.points.length > 0 && <><h3>{t("Key points")}</h3><ul className="key-points">{insight.points.map((point, i) => <li key={i}>{point}</li>)}</ul></>}</>}
        {mode === "social" && <>
          <input className="draft-title" aria-label={t("Draft title")} value={insight.title} onChange={e => updateInsight({ title: e.target.value })} />
          <textarea className="draft-body" aria-label={t("Draft text")} value={insight.body} onChange={e => updateInsight({ body: e.target.value })} />
          <div className="draft-actions"><button onClick={() => run(async () => { try { await navigator.clipboard.writeText(`${insight.title}\n\n${insight.body}`); setCopied(true); } catch { throw new Error("Copy failed. Select and copy the draft text."); } })}>{copied ? <Check size={14} /> : <Copy size={14} />}{copied ? t("Copied") : t("Copy draft")}</button><button onClick={() => run(() => quoteCards(insight.title, insight.body, entry.date))}><Download size={14} />{t("Save quote card")}</button></div>
          <p className="privacy-note">{t("Review personal details before sharing.")}</p>
          {insight.gaps.length > 0 && <section className="material-gaps"><h3>{t("Material to add")}</h3>{insight.gaps.map((gap, i) => <div key={i} className="gap"><span className={`gap-kind ${gap.action}`}>{gap.action === "find" ? t("Find material") : gap.action === "write" ? t("Add details") : t("Image idea")}</span><p>{gap.text}</p>{gap.action === "find" ? <a className="text-link" href={`https://www.google.com/search?q=${encodeURIComponent(gap.query)}`} target="_blank" rel="noreferrer">{t("Search the web")}<ExternalLink size={12} /></a> : <button className="text-link" onClick={() => { setMaterials(true); setNotes(gap.query); setTimeout(() => sourceRef.current?.focus(), 0); }}>{gap.action === "image" ? t("Image prompt") : t("Add to journal")}<ChevronDown size={12} /></button>}</div>)}</section>}
          <section className="image-section"><h3><ImageIcon size={14} />{t("Image idea")}</h3><textarea aria-label={t("Image prompt")} value={insight.imagePrompt} onChange={e => updateInsight({ imagePrompt: e.target.value })} /><p className="muted small">{t("Image generation uses your configured OpenAI image service.")}</p><button disabled={imageBusy || !insight.imagePrompt.trim()} onClick={() => run(() => generateImage(insight.imagePrompt))}>{imageBusy ? <LoaderCircle size={14} className="spin" /> : <ImageIcon size={14} />}{entry.image ? t("Generate a new image (paid)") : t("Generate image (paid)")}</button>{imageUrl && <><img className="generated-image" src={imageUrl} alt={t("AI-generated image")} /><a className="text-link" href={imageUrl} download={`journal-${entry.date}.png`}>{t("Download image")}<Download size={13} /></a></>}</section>
        </>}
        {mode === "todos" && <section className="todo-proposals">{!insight.todos.length && <p className="muted">{t("No follow-ups found in this entry.")}</p>}{insight.todos.map((todo, i) => {
          const id = `${insight.taskId}-${i}`; const added = entry.todos.some(x => x.id === id || (x.title === todo.title && x.evidence === todo.evidence));
          return <div className="todo-proposal" key={id}><div><span className={`kind ${todo.kind}`}>{kindLabel(todo.kind)}</span>{todo.due && <time>{todo.due}</time>}</div><h3>{todo.title}</h3><details><summary>{t("Source excerpt")}</summary><p>{todo.evidence}</p></details><button disabled={added} onClick={() => store.update(entry.id, { todos: [...entry.todos, { ...todo, id, done: false }] })}>{added ? <Check size={14} /> : <Plus size={14} />}{added ? t("Added") : t("Add to to-dos")}</button></div>;
        })}</section>}
      </div>}
      {auxiliary.filter(task => taskIsActive(task) || ["failed", "interrupted"].includes(task.status)).map(task => <TaskStatus key={task.id} task={task} run={run} />)}
      <section className="sources"><button className="sources-heading" aria-expanded={materials} onClick={() => setMaterials(!materials)}><Link size={14} />{t("Sources & extra context")}<small>{entry.sources.length || ""}</small>{materials ? <ChevronDown size={14} /> : <ChevronRight size={14} />}</button>{materials && <div className="sources-content">{entry.sources.map(source => <div className="source" key={source.id}><div>{source.url ? <a href={source.url} target="_blank" rel="noreferrer">{source.url}</a> : <span>{t("Add details")}</span>}<IconButton label={t("Remove material")} onClick={() => store.update(entry.id, { sources: entry.sources.filter(s => s.id !== source.id) })}><Trash2 size={13} /></IconButton></div><p>{source.text}</p></div>)}<input type="url" value={url} onChange={e => setUrl(e.target.value)} placeholder={t("Public article URL")} aria-label={t("Public article URL")} /><button disabled={!url.trim() || sourceBusy || entry.sources.length >= 24} onClick={() => run(source)}><Link size={14} />{t("Read & summarize link")}</button><textarea ref={sourceRef} value={notes} onChange={e => setNotes(e.target.value)} placeholder={t("Paste notes, an excerpt, or details the agent needs.")} aria-label={t("Add details")} /><button disabled={!notes.trim() || entry.sources.length >= 24} onClick={() => { store.update(entry.id, { sources: [...entry.sources, { id: crypto.randomUUID(), url: "", text: notes }] }); setNotes(""); }}><Plus size={14} />{t("Add material")}</button></div>}</section>
    </div><footer className="desk-footer"><span className="model-dot" />{t("Default model in Daily settings")}</footer>
  </aside>;
}

function TaskStatus({ task, run }: { task: TaskSnapshot; run: (fn: () => Promise<unknown>) => void }) {
  if (task.status === "completed" || task.status === "cancelled") return null;
  const active = taskIsActive(task);
  return <div className={`task-status${active ? "" : " task-error"}`} role="status"><p>{active ? t("Working in the background…") : task.status === "interrupted" ? t("Task interrupted. Continue to recover saved progress.") : message(task.error || t("Task failed"))}</p>{active ? <button disabled={task.status === "cancelling"} onClick={() => run(() => dailyHost().tasks.cancel(task.id))}><X size={13} />{t("Stop")}</button> : <button onClick={() => run(() => dailyHost().tasks.retry(task.id))}><RotateCcw size={13} />{t("Continue")}</button>}</div>;
}
