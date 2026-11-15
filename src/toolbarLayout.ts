import { createEffect, createMemo, createSignal, onCleanup, onMount, untrack, type JSX } from "solid-js";
import { placeToolbarActions, toolbarActions, toolbarChoice, type ToolbarActionId, type ToolbarPlacement } from "./toolbarChoice";
import { documentOf, windowOf, newResizeObserver } from "./windowRealm";

/** One owner of optional-action placement. One ResizeObserver measurement per
 * resize, O(buttons) layout reads, no files/IPC or navigation/edit subscription.
 * A preference or chrome-availability change recomputes from the last width.
 * The observer and mount-scoped state are released with the toolbar. */
export function createToolbarLayout(availability: () => { available: ToolbarActionId[]; workspace: boolean; tabs: boolean; windowControls: boolean; navigationAtBottom: boolean }) {
  let root: HTMLElement;
  let width = 0;
  const [placement, setPlacement] = createSignal<ToolbarPlacement>({ bar: toolbarActions.map(action => action.id), menu: [], overflow: false });
  const visible = createMemo(() => new Set(placement().bar));
  const measure = () => {
    if (!root) return;
    const options = availability();
    const controls = root.querySelector<HTMLElement>(".win-controls");
    const choice = toolbarChoice();
    const win = windowOf(root);
    let budget = 0;
    const costs: Partial<Record<ToolbarActionId, number>> = {};
    if (choice) {
      const left = options.navigationAtBottom ? null : root.querySelector<HTMLElement>(".topbar-left");
      const right = root.querySelector<HTMLElement>(".topbar-right")!;
      const menu = right.querySelector<HTMLElement>(".topbar-overflow")!;
      const elements = [...root.querySelectorAll<HTMLElement>("[data-toolbar-action]")];
      const displays = elements.map(element => element.style.display);
      const menuDisplay = menu.style.display;
      const gap = parseFloat(win.getComputedStyle(right).columnGap) || 0;
      const outerGap = parseFloat(win.getComputedStyle(root).columnGap) || 0;
      const outerGaps = Math.max(0, [...root.children].filter(element => win.getComputedStyle(element).display !== "none").length - 1);
      const strip = root.querySelector<HTMLElement>(".tab-bar");
      const fixed = (left?.offsetWidth ?? 0) + (controls?.offsetWidth ?? 0)
        + (strip ? parseFloat(win.getComputedStyle(strip).minWidth) || 0 : 0) + outerGap * outerGaps;
      // Measure intrinsic control widths in one synchronous batch, then restore
      // their visibility before painting. This also measures custom.css widths
      // for buttons which were in the menu at the previous size.
      elements.forEach(element => { element.style.display = ""; });
      menu.style.display = "block";
      for (const element of elements) {
        const id = element.dataset.toolbarAction as ToolbarActionId;
        costs[id] = element.offsetWidth + gap;
      }
      const divider = [...right.querySelectorAll<HTMLElement>(":scope > .topbar-sep")].reduce((total, element) => {
        const size = element.offsetWidth;
        if (!size) return total;
        const style = win.getComputedStyle(element);
        return total + size + (parseFloat(style.marginInlineStart) || 0) + (parseFloat(style.marginInlineEnd) || 0) + gap;
      }, 0);
      budget = Math.max(0, width - fixed - menu.offsetWidth - divider - (controls ? gap : 0));
      elements.forEach((element, index) => { element.style.display = displays[index]; });
      menu.style.display = menuDisplay;
    }
    setPlacement(placeToolbarActions(choice, width / (parseFloat(win.getComputedStyle(documentOf(root).documentElement).fontSize) / 16), {
      ...options, budget, costs,
    }));
  };
  onMount(() => {
    const style = windowOf(root).getComputedStyle(root);
    width = root.clientWidth - parseFloat(style.paddingLeft || "0") - parseFloat(style.paddingRight || "0");
    untrack(measure);
    const observer = newResizeObserver(root, entries => {
      width = entries[0].contentRect.width;
      untrack(measure);
    });
    observer?.observe(root);
    onCleanup(() => observer?.disconnect());
  });
  createEffect(() => { toolbarChoice(); availability(); untrack(measure); });
  const style = (id: ToolbarActionId): JSX.CSSProperties => ({
    display: visible().has(id) ? undefined : "none",
  });
  return { ref: (element: HTMLElement) => { root = element; }, placement, style };
}
