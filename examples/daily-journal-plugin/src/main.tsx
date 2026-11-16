import { createRoot } from "react-dom/client";
import { definePlugin, dailyHost } from "@daily/plugin-sdk";
import { JournalStore } from "./store";
import { startWorkers } from "./agent";
import { App } from "./App";
import styles from "./styles.css?inline";

let store: JournalStore;
export default definePlugin({
  async activate() {
    store = new JournalStore(dailyHost());
    let stopWorkers: (() => void) | undefined;
    const ready = () => { if (store.getSnapshot().ready && !stopWorkers) stopWorkers = startWorkers(store); };
    const unsubscribe = store.subscribe(ready);
    await store.load();
    ready();
    return () => { unsubscribe(); stopWorkers?.(); store.dispose(); };
  },
  views: {
    "journal.main": {
      mount(container, view) {
        view.addStyles(styles);
        const root = createRoot(container);
        root.render(<App store={store} view={view} />);
        return () => { root.unmount(); void store.flush().catch(() => undefined); };
      },
    },
  },
});
