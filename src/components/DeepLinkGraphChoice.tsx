import { createSignal, createEffect, onCleanup, For, Show } from "solid-js";
import { registerTransientLayer } from "../transientLayers";
import type { LinkTarget } from "../deepLinkNavigation";
import "./DeepLinkGraphChoice.css";
const [choice, setChoice] = createSignal<{ candidates: LinkTarget[]; finish: (target: LinkTarget | null) => void } | null>(null);
/** One explicit choice for a copied graph. O(number of matching known roots).
 * Cancellation resolves null; a subsequent chooser cancels its predecessor. */
export function chooseLinkGraph(candidates: LinkTarget[]): Promise<LinkTarget | null> {
  choice()?.finish(null);
  return new Promise((resolve) => setChoice({ candidates, finish: (target) => { setChoice(null); resolve(target); } }));
}
export function DeepLinkGraphChoice() {
  let root: HTMLElement | undefined;
  createEffect(() => {
    const pending = choice();
    if (!pending) return;
    const unregister = registerTransientLayer({ id: "tine-link-choice", root: () => root ?? null,
      dismiss: () => { pending.finish(null); return true; } });
    onCleanup(unregister);
    queueMicrotask(() => root?.querySelector<HTMLButtonElement>("button")?.focus());
  });
  onCleanup(() => choice()?.finish(null));
  return <Show when={choice()}>{(pending) =>
    <div class="modal-overlay tine-link-overlay" onKeyDown={(event) => {
      if (event.key === "Escape") pending().finish(null);
      if (event.key === "Tab" && root) {
        const buttons = [...root.querySelectorAll<HTMLButtonElement>("button")];
        const first = buttons[0], last = buttons.at(-1);
        if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
        else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
      }
    }}>
      <section ref={root} class="tine-link-choice" role="dialog" aria-modal="true" aria-label="Choose graph for Tine link">
        <h2>Choose graph for Tine link</h2>
        <p>This link matches more than one copy of a graph. Tine will remember your choice on this device.</p>
        <div class="tine-link-roots"><For each={pending().candidates}>{(target) => <button type="button" onClick={() => pending().finish(target)}>{target.root}</button>}</For></div>
        <button type="button" onClick={() => pending().finish(null)}>Cancel</button>
      </section>
    </div>
  }</Show>;
}
