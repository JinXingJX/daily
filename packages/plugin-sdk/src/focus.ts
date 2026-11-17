const panels = new WeakMap<Document | ShadowRoot, symbol[]>();
const selector = 'button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), a[href], summary, [tabindex]:not([tabindex="-1"])';

/** Keyboard focus for a view-local dialog or drawer, including nested dialogs in Shadow DOM. */
export function focusPluginPanel(element: HTMLElement, onClose: () => void, modal = true): () => void {
  const scope = element.getRootNode() as Document | ShadowRoot;
  const previous = scope.activeElement;
  const id = Symbol("panel");
  const stack = panels.get(scope) ?? [];
  panels.set(scope, stack);
  stack.push(id);
  const controls = () => Array.from(element.querySelectorAll<HTMLElement>(selector))
    .filter(control => control.tabIndex >= 0 && control.getClientRects().length > 0 && !control.closest('[hidden], [inert], [aria-hidden="true"]'));
  const frame = requestAnimationFrame(() => {
    if (stack[stack.length - 1] !== id) return;
    (element.querySelector<HTMLElement>("[autofocus], [data-autofocus]") ?? controls()[0] ?? element).focus({ preventScroll: true });
  });
  const keydown = (event: Event) => {
    if (!(event instanceof KeyboardEvent) || event.isComposing || event.keyCode === 229 || stack[stack.length - 1] !== id) return;
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      onClose();
    } else if (modal && event.key === "Tab") {
      const items = controls();
      const active = scope.activeElement;
      const index = items.indexOf(active as HTMLElement);
      if (!items.length) { event.preventDefault(); element.focus(); }
      else if (index < 0 || (!event.shiftKey && index === items.length - 1) || (event.shiftKey && index === 0)) {
        event.preventDefault();
        items[event.shiftKey ? items.length - 1 : 0].focus();
      }
    }
  };
  scope.addEventListener("keydown", keydown);
  return () => {
    cancelAnimationFrame(frame);
    scope.removeEventListener("keydown", keydown);
    const wasTop = stack[stack.length - 1] === id;
    const index = stack.indexOf(id);
    if (index >= 0) stack.splice(index, 1);
    if (wasTop && previous instanceof HTMLElement && previous.isConnected) previous.focus({ preventScroll: true });
  };
}
