# Plugin interfaces

`DESIGN.md` is the visual and interaction standard for Daily and its plugins. Plugins keep their own layouts, but use the same paper surfaces, ink buttons, typography, spacing, focus rings and state colors.

## Shared foundation

The host installs the shared foundation in every view's Shadow DOM. Plugins also install it explicitly so their development previews can use the same styles:

```ts
import { pluginDesignStyles } from "@daily/plugin-sdk";

mount(container, view) {
  view.addStyles(pluginDesignStyles);
  view.addStyles(styles);
  // Mount the view into container.
}
```

The shared stylesheet is generated from `src/styles/tokens.css`. After changing tokens, run `npm run gen:plugin-design`, then `npm run check:plugin-design`. The generator updates the SDK and, when the sibling Journal repository is present, its vendored SDK. Do not edit the generated `design.ts` files. A plugin may alias these tokens for its domain styles; it must not maintain an independent UI palette. Map artwork retains its geographic data colors.

Use `--app-sidebar-bg` for navigation, `--app-main-bg` for content and inspectors, and `--surface-strong` for inputs and raised surfaces. Text buttons use `--primary` with `--primary-contrast`; clay is reserved for icon emphasis, focus and selected modes. Status colors always accompany text or an icon. Page titles use regular serif type; controls use `--font-sans`.

## Panels and keyboard interaction

Keep inspectors optional. At narrow view widths, open them over the content with an opaque canvas surface and a separate backdrop. Use view container queries, not window media queries, because the host sidebar changes the space available to the plugin. All drawers need a visible close button and Escape behavior.

`focusPluginPanel` handles initial focus, Tab containment, nested panel ordering, IME-safe Escape and focus restoration inside Shadow DOM:

```ts
import { focusPluginPanel } from "@daily/plugin-sdk";

// Call after the panel is mounted. Dispose when it closes.
const dispose = focusPluginPanel(panel, closePanel);
// Pass false as the third argument for a nonmodal inspector.
```

Use `aria-pressed` for panel toggles and filters, `aria-current` for navigation, and arrow keys with a single Tab stop for tablists. Icon buttons need both `title` and `aria-label`. Preserve drafts while an assistant is running, disable duplicate submissions, and keep stop/retry actions next to their task. Composer submission uses Command/Ctrl+Enter; Enter inserts a newline and IME confirmation never submits.

Keep loading skeletons shaped like the content. Put validation in the form being edited, and runtime failures beside the relevant content. Avoid a permanent bottom status bar. Follow new output only while the reader is near the bottom, and respect reduced motion.

## Verification

Build each changed plugin and the host, run their existing tests, and check light, dark and narrow views. Verify panel toggles, Escape, Tab/Shift+Tab, focus restoration, form errors, empty search results and local save behavior. Use an isolated temporary workspace for interaction checks.
