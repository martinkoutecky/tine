import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { QuickSwitcher } from "./QuickSwitcher";
import { closeSwitcher, openCommandPalette, switcherOpen } from "../ui";
import { focusedPaneId, resetPaneLayoutToSingle } from "../panes";
import { route, type Route } from "../router";
import { interfaceZoom, installInterfaceZoomKeys, zoomIn, zoomOut, zoomReset } from "../zoom";

let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  closeSwitcher();
  zoomReset();
  resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 });
  document.body.innerHTML = "";
});

describe("Reset interface zoom command (GH #522)", () => {
  it.each([
    { name: "enlarged notes", adjust: zoomIn, current: { kind: "page", name: "Notes", pageKind: "page", block: "focused-block" } },
    { name: "reduced PDF", adjust: zoomOut, current: { kind: "pdf", viewId: "reading", filename: "paper.pdf", label: "Paper", page: 4, scale: 1.7 } },
    { name: "already default", adjust: zoomReset, current: { kind: "journals" } },
  ] satisfies { name: string; adjust: () => void; current: Route }[])("resets $name through the searchable palette without navigating", async ({ adjust, current }) => {
    resetPaneLayoutToSingle({ tabs: [{ history: [current], pos: 0, pinned: false }], activeIndex: 0 });
    adjust();
    if (interfaceZoom() !== 1) {
      expect(localStorage.getItem("logseq-claude.zoom")).toBe(String(interfaceZoom()));
    }
    const previousRoute = { ...route() };
    const previousPane = focusedPaneId();
    const root = document.createElement("div");
    document.body.append(root);
    dispose = render(() => <QuickSwitcher />, root);
    openCommandPalette();
    const input = root.querySelector<HTMLInputElement>('[role="combobox"]')!;
    input.value = "reset zoom";
    input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    await vi.waitFor(() => {
      const option = [...root.querySelectorAll<HTMLElement>('[role="option"]')]
        .find((el) => el.textContent?.includes("Reset interface zoom"));
      expect(option).toBeDefined();
    });
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    expect(interfaceZoom()).toBe(1);
    expect(localStorage.getItem("logseq-claude.zoom")).toBeNull();
    expect(switcherOpen()).toBe(false);
    expect(route()).toEqual(previousRoute);
    expect(focusedPaneId()).toBe(previousPane);
  });

  it("keeps Ctrl+0 owned by the PDF route and working for notes", () => {
    zoomIn();
    resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "pdf", viewId: "reading", filename: "paper.pdf", label: "Paper", scale: 1.7 }], pos: 0, pinned: false }], activeIndex: 0 });
    const uninstall = installInterfaceZoomKeys();
    try {
      const pdfKey = new KeyboardEvent("keydown", { key: "0", ctrlKey: true, cancelable: true });
      window.dispatchEvent(pdfKey);
      expect(pdfKey.defaultPrevented).toBe(false);
      expect(interfaceZoom()).toBe(1.1);
      resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 });
      const notesKey = new KeyboardEvent("keydown", { key: "0", ctrlKey: true, cancelable: true });
      window.dispatchEvent(notesKey);
      expect(notesKey.defaultPrevented).toBe(true);
      expect(interfaceZoom()).toBe(1);
    } finally {
      uninstall();
    }
  });
});
