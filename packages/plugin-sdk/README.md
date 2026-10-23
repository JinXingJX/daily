# @daily/plugin-sdk

Typed client for the `window.dailyPlugin` bridge that Daily injects into every plugin frame, plus the bridge itself.

| File | Contents |
| --- | --- |
| `src/index.ts` | `DailyPluginApi` types, `dailyHost()`, `errorMessage()`, `hostErrorCode()`, agent progress helpers |
| `src/bridge.ts` | `installDailyBridge()` — the runtime the host serializes into each frame (must stay self-contained) |
| `src/methods.generated.ts` | Plugin RPC method names, generated from Rust by `npm run gen:protocol` |
| `src/hostText.ts` | `localizeHostText()` — translates the runtime's English event and status text |

The package is not published to npm yet. Depend on it by path from your plugin:

```json
{ "dependencies": { "@daily/plugin-sdk": "file:../daily/packages/plugin-sdk" } }
```

```ts
import { dailyHost } from "@daily/plugin-sdk";

const host = dailyHost();
await host.storage.set("draft", { text: "hello" });
```

See [docs/plugin-development.md](../../docs/plugin-development.md) for the manifest, API reference, and how to add a host method.
