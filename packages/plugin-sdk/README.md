# @daily/plugin-sdk

Everything a Daily plugin imports: `definePlugin()` for the module's default export, the typed host API, and task helpers.

| File | Contents |
| --- | --- |
| `src/index.ts` | `definePlugin()`, view and `DailyPluginApi` types, `dailyHost()`, `errorMessage()`, `hostErrorCode()`, agent progress helpers |
| `src/host.ts` | `createPluginHost()` — builds the host API object over a transport (the app's, or a fake in tests) |
| `src/tasks.ts` | Durable task client: `runTaskHandlers()`, `waitForTask()`, `observeTasks()` |
| `src/methods.generated.ts` | Plugin RPC method names, generated from Rust by `npm run gen:protocol` |
| `src/hostText.ts` | `localizeHostText()` — translates the runtime's English event and status text |

The package is not published to npm yet. Depend on it by path from your plugin:

```json
{ "dependencies": { "@daily/plugin-sdk": "file:../daily/packages/plugin-sdk" } }
```

```tsx
import { createRoot } from "react-dom/client";
import { dailyHost, definePlugin } from "@daily/plugin-sdk";

export default definePlugin({
  views: {
    "notes.main": {
      mount(container, view) {
        view.addStyles(":host { background: Canvas; color: CanvasText; }");
        const root = createRoot(container);
        root.render(<button onClick={() => dailyHost().storage.set("draft", { text: "hello" })}>Save</button>);
        return () => root.unmount();
      },
    },
  },
});
```

`activate(host)` starts plugin-wide work and returns cleanup. Views mount on first use and remain mounted while hidden. `mount` may return a promise; stop a pending mount when `view.signal` is aborted. Use `view.onVisibilityChange` to release UI-only timers and `:host([data-daily-theme="dark"])` for theme styles.

See [docs/plugin-development.md](../../docs/plugin-development.md) for the manifest, API reference, and how to add a host method.
