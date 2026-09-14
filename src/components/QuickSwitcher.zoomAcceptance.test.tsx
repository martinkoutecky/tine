import { afterEach, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { QuickSwitcher } from "./QuickSwitcher";
import { closeSwitcher, openCommandPalette, setGraphMeta, switcherOpen } from "../ui";
import { focusedPaneId, focusPane, layoutPaneIds, paneRouter, resetPaneLayoutToSingle, splitRootAtEdge } from "../panes";
import { interfaceZoom, installInterfaceZoomKeys, zoomIn, zoomOut, zoomReset } from "../zoom";

let dispose: (() => void) | undefined;
let uninstallKeys: (() => void) | undefined;

afterEach(() => {
  uninstallKeys?.();
  uninstallKeys = undefined;
  closeSwitcher();
  dispose?.();
  dispose = undefined;
  zoomReset();
  setGraphMeta(null);
  resetPaneLayoutToSingle();
  document.body.replaceChildren();
});

async function searchReset() {
  openCommandPalette();
  await Promise.resolve();
  const input = document.querySelector<HTMLInputElement>('[role="combobox"]')!;
  input.value = "zoom";
  input.dispatchEvent(new Event("input", { bubbles: true }));
  await Promise.resolve();
  const options = [...document.querySelectorAll<HTMLElement>('[role="option"]')];
  const reset = options.filter((option) => /Reset (interface )?zoom/.test(option.textContent ?? ""));
  expect(reset).toHaveLength(1);
  return { input, reset: reset[0] };
}

describe("Reset zoom through the rendered command palette", () => {
  it.each([2, -2, 100, -100, 0])("restores and persists the default after %i zoom steps, preserving both panes", async (steps) => {
    setGraphMeta({ root: "/graphs/reset-zoom-test" } as never);
    resetPaneLayoutToSingle({
      tabs: [{ history: [{ kind: "page", name: "Notes", pageKind: "page", block: "focused-block" }], pos: 0, pinned: false }],
      activeIndex: 0,
    });
    const pdfPane = splitRootAtEdge("right", "main")!;
    paneRouter(pdfPane).openInNewTab({ kind: "pdf", viewId: "reading-view", filename: "assets/paper.pdf", label: "Paper", page: 7, scale: 1.8 }, true);
    focusPane(pdfPane);
    const paneIds = layoutPaneIds();
    const snapshots = paneIds.map((id) => paneRouter(id).snapshot());
    const root = document.createElement("div");
    document.body.append(root);
    dispose = render(() => <QuickSwitcher />, root);
    zoomReset();
    for (let i = 0; i < Math.abs(steps); i++) (steps > 0 ? zoomIn : zoomOut)();
    if (steps !== 0) {
      expect(interfaceZoom()).not.toBe(1);
      expect(localStorage.getItem("logseq-claude.zoom")).toBe(String(interfaceZoom()));
    }

    const { reset } = await searchReset();
    reset.dispatchEvent(new MouseEvent("mousedown", { button: 0, bubbles: true, cancelable: true }));

    expect(switcherOpen()).toBe(false);
    expect(interfaceZoom()).toBe(1);
    expect(localStorage.getItem("logseq-claude.zoom")).toBeNull();
    expect(layoutPaneIds()).toEqual(paneIds);
    expect(focusedPaneId()).toBe(pdfPane);
    expect(paneIds.map((id) => paneRouter(id).snapshot())).toEqual(snapshots);

    // Keyboard selection is idempotent and does not navigate out of either view.
    const { input } = await searchReset();
    input.value = "Reset zoom";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    await Promise.resolve();
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    expect(switcherOpen()).toBe(false);
    expect(interfaceZoom()).toBe(1);
    expect(paneIds.map((id) => paneRouter(id).snapshot())).toEqual(snapshots);

    // The new unbound command must not steal the existing PDF key route.
    uninstallKeys = installInterfaceZoomKeys();
    const pdfPlus = new KeyboardEvent("keydown", { key: "+", ctrlKey: true, cancelable: true });
    window.dispatchEvent(pdfPlus);
    expect(pdfPlus.defaultPrevented).toBe(false);
    expect(interfaceZoom()).toBe(1);
    focusPane("main");
    const notesPlus = new KeyboardEvent("keydown", { key: "+", ctrlKey: true, cancelable: true });
    window.dispatchEvent(notesPlus);
    expect(notesPlus.defaultPrevented).toBe(true);
    expect(interfaceZoom()).toBeGreaterThan(1);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "0", metaKey: true, cancelable: true }));
    expect(interfaceZoom()).toBe(1);
  });
});
