import { Search } from "lucide-react";
import { Fragment, useEffect, useId, useMemo, useRef, useState, type ReactNode } from "react";
import { type CommandGroup, type CommandItem } from "@/app/useWorkspaceCommands";
import { t } from "@/i18n";

const GROUP_ORDER: CommandGroup[] = ["task", "global"];

function groupLabel(group: CommandGroup) {
  return group === "task" ? t("Tasks") : t("Global");
}

export function filterCommands(items: CommandItem[], query: string) {
  const normalized = query.trim().toLowerCase();
  const matches = normalized
    ? items.filter((item) => `${item.label} ${item.detail}`.toLowerCase().includes(normalized))
    : items.filter((item) => item.group === "global" || item.current).concat(
        items.filter((item) => item.group === "task" && !item.current).slice(0, 8),
      );
  // With a query, tasks are the likelier target, so they lead; idle, commands lead.
  const order = normalized ? GROUP_ORDER : [...GROUP_ORDER].reverse();
  return order.flatMap((group) => matches.filter((item) => item.group === group));
}

export function CommandPalette({
  open,
  items,
  onClose,
}: {
  open: boolean;
  items: CommandItem[];
  onClose: () => void;
}) {
  const [query, setQuery] = useState("");
  const [activeIndex, setActiveIndex] = useState(0);
  const panelRef = useRef<HTMLElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const listId = useId();
  const filteredItems = useMemo(() => filterCommands(items, query), [items, query]);
  const activeItem = filteredItems[Math.min(activeIndex, filteredItems.length - 1)] ?? null;

  useEffect(() => {
    if (!open) {
      setQuery("");
    }
  }, [open]);

  useEffect(() => {
    setActiveIndex(0);
  }, [query]);

  useEffect(() => {
    if (!activeItem) return;
    listRef.current
      ?.querySelector<HTMLElement>(`[data-command-id="${CSS.escape(activeItem.id)}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [activeItem]);

  useEffect(() => {
    if (!open) return;
    const previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Tab" || !panelRef.current) return;
      // The search field owns focus; options are reached with the arrow keys.
      event.preventDefault();
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("keydown", handleKeyDown);
      if (previousFocus?.isConnected) previousFocus.focus();
    };
  }, [open]);

  if (!open) {
    return null;
  }

  const run = (item: CommandItem | null) => {
    if (!item) return;
    onClose();
    item.action();
  };

  return (
    <div className="command-overlay" role="presentation" onMouseDown={onClose}>
      <section
        ref={panelRef}
        className="command-panel motion-reveal"
        role="dialog"
        aria-modal="true"
        aria-label={t("Command palette")}
        onMouseDown={(event) => event.stopPropagation()}
      >
        <label className="flex h-[52px] shrink-0 items-center gap-2.5 border-b border-[var(--line-soft)] px-4">
          <Search className="size-[17px] shrink-0 text-[var(--text-soft)]" aria-hidden="true" />
          <input
            autoFocus
            role="combobox"
            aria-label={t("Search tasks and commands")}
            aria-expanded="true"
            aria-controls={listId}
            aria-activedescendant={activeItem ? `${listId}-${activeItem.id}` : undefined}
            className="h-10 min-w-0 flex-1 bg-transparent text-[15px] text-[var(--text-strong)] outline-none placeholder:text-[var(--text-muted)]"
            placeholder={t("Search tasks, plugins and commands")}
            value={query}
            onChange={(event) => setQuery(event.currentTarget.value)}
            onKeyDown={(event) => {
              if (event.key === "ArrowDown" || event.key === "ArrowUp") {
                event.preventDefault();
                if (!filteredItems.length) return;
                const delta = event.key === "ArrowDown" ? 1 : -1;
                setActiveIndex((index) => (index + delta + filteredItems.length) % filteredItems.length);
              } else if (event.key === "Enter" && !event.nativeEvent.isComposing) {
                event.preventDefault();
                run(activeItem);
              }
            }}
          />
          <kbd className="kbd">esc</kbd>
        </label>
        <div ref={listRef} id={listId} role="listbox" aria-label={t("Results")} className="min-h-0 flex-1 overflow-auto p-1.5">
          {filteredItems.map((item, index) => {
            const startsGroup = index === 0 || filteredItems[index - 1].group !== item.group;
            const selected = item === activeItem;
            return (
              <Fragment key={item.id}>
                {startsGroup ? (
                  <div className="px-2.5 pb-1 pt-2 text-[11px] font-medium text-[var(--text-muted)]" role="presentation">
                    {groupLabel(item.group)}
                  </div>
                ) : null}
                <div
                  id={`${listId}-${item.id}`}
                  data-command-id={item.id}
                  role="option"
                  aria-selected={selected}
                  className="command-option cursor-pointer"
                  onMouseMove={() => {
                    if (!selected) setActiveIndex(index);
                  }}
                  onClick={() => run(item)}
                >
                  <span className="shrink-0 text-[var(--text-soft)]" aria-hidden="true">
                    {item.icon}
                  </span>
                  <span className="min-w-0 flex-1 truncate">{highlight(item.label, query)}</span>
                  <span className="shrink-0 truncate text-[11px] text-[var(--text-muted)]">{item.detail}</span>
                  {item.shortcut ? <kbd className="kbd shrink-0">{item.shortcut}</kbd> : null}
                </div>
              </Fragment>
            );
          })}
          {!filteredItems.length ? (
            <div className="px-3 py-8 text-center text-[13px] text-[var(--text-muted)]">{t("No matching tasks or commands")}</div>
          ) : null}
        </div>
        <div className="flex h-9 shrink-0 items-center gap-4 border-t border-[var(--line-soft)] bg-[var(--surface)] px-4 text-[11px] text-[var(--text-soft)]">
          <span>
            <kbd>↑↓</kbd> {t("Select")}
          </span>
          <span>
            <kbd>⏎</kbd> {t("Open")}
          </span>
          <span className="flex-1" />
          <span>{t("{count} results", { count: filteredItems.length })}</span>
        </div>
      </section>
    </div>
  );
}

function highlight(text: string, query: string): ReactNode {
  const needle = query.trim();
  if (!needle) return text;
  const index = text.toLowerCase().indexOf(needle.toLowerCase());
  if (index === -1) return text;
  return (
    <>
      {text.slice(0, index)}
      <mark>{text.slice(index, index + needle.length)}</mark>
      {text.slice(index + needle.length)}
    </>
  );
}
