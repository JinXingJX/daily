# Daily Desktop Design System

## Physical scene

Daily is used by a developer on a 14 to 27-inch display for long agent sessions in mixed ambient light. The interface defaults to the operating-system theme, keeps contrast comfortable for sustained reading, and prioritizes visible runtime state over decoration.

## Visual strategy

- Register: product.
- Color strategy: restrained graphite neutrals with one cobalt accent below 10 percent of the visible surface.
- Surfaces use warm graphite OKLCH neutrals with a subtly cool content canvas. Pure black, pure white, gradients, glass effects, and decorative color are excluded.
- Green, amber, and red communicate success, waiting, and failure only. Every semantic color is paired with text or an icon.

## Typography

- UI: `Geist Sans`, `-apple-system`, `BlinkMacSystemFont`, `Segoe UI`, `system-ui`, sans-serif.
- Technical metadata: `Geist Mono`, `SFMono-Regular`, `Roboto Mono`, ui-monospace, monospace.
- Product labels range from 11 to 16px. Body prose stays within 65 to 75 characters where possible.

## Desktop shell

- Unified titlebar: 44px, draggable except controls marked `.no-drag`. Its left segment spans the 56px app rail plus the task panel (or settings navigation) so the columns line up; it shows the open task's run state (running step and elapsed time, waiting, or failed step).
- Status bar: 26px along the bottom edge with runtime health, the default model, and the enabled plugin count.
- macOS traffic-light safe area: 78px from the left edge.
- Task panel: 264px default, resizable from 220 to 400px, hidden with the titlebar toggle.
- Conversation views may open a 340px inspector (run, context, hooks tabs) on the right. Below 1180px it overlays the canvas instead of compressing it.
- Settings replace the task panel with their own 288px navigation.
- Window: 1440×960 default, 1180×760 minimum.
- Main canvas owns all remaining space. Plugin iframe views begin below the unified titlebar and fill the canvas.
- The conversation composer is a bounded, floating control near the bottom of the canvas rather than a full-width footer.
- Z-index: canvas 0, inspector overlay 25, status banner 20, titlebar 30, command overlay 50.
- Keyboard: ⌘K command palette, ⌘N new task, ⌘I inspector, ⌘, settings, ⌘⏎ send, ⌘S save agent.

## Component states

- Controls implement default, hover, focus-visible, active, disabled, loading, and error states.
- Runtime states are `starting`, `ready`, `restarting`, `failed`, and `stopped`.
- Loading uses structural skeletons. Runtime failure uses an inline banner with retry instead of a blocking modal.
- Focus rings use a two-step surface and cobalt outline and must remain visible in both themes.

## Motion and accessibility

- State transitions use 150 to 200ms ease-out-quart timing.
- Layout properties are not animated. Reduced-motion disables pulses, rotations, and reveals.
- Text and interactive controls target WCAG AA contrast.
- Icon-only controls require accessible labels and a minimum 32px target in dense desktop contexts.
