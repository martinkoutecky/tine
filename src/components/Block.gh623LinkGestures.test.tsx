import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Block } from "./Block";
import { backend } from "../backend";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { initParser } from "../render/parse";
import { layoutPaneIds, paneRouter, resetPaneLayoutToSingle } from "../panes";
import { rightSidebar, setRightSidebar, setRightSidebarOpen } from "../ui";

const uuid = "62300000-0000-4000-8000-000000000014";
const snapshot = () => ({ tabs: [{ history: [{ kind: "journals" as const }], pos: 0, pinned: false }], activeIndex: 0 });
beforeAll(() => initParser());
afterEach(() => {
  vi.restoreAllMocks();
  resetStore();
  setRightSidebar([]);
  setRightSidebarOpen(false);
  resetPaneLayoutToSingle(snapshot());
  document.body.innerHTML = "";
});

describe("GH #623 rendered block link gesture siblings", () => {
  for (const [surface, raw, selector] of [
    ["page reference", "Open [[Destination]]", "a.page-ref"],
    ["tag", "Open #Destination", "a.tag"],
    ["property value", "Body\nrelated:: [[Destination]]", ".prop-value a.page-ref"],
    ["block reference", `Open ((${uuid}))`, ".block-ref"],
  ]) {
    it.each([
      ["Shift", { shiftKey: true }, "sidebar"],
      ["Ctrl", { ctrlKey: true }, "background"],
      ["Cmd", { metaKey: true }, "background"],
      ["middle", { button: 1 }, "background"],
      ["Alt", { altKey: true }, "pane"],
    ] as const)(`${surface}: %s retains its destination through Block rendering`, async (_gesture, modifier, destination) => {
      resetPaneLayoutToSingle(snapshot());
      vi.spyOn(backend(), "resolveBlocks").mockResolvedValue([{
        page: "Destination", kind: "page", path: "pages/Destination.md",
        blocks: [{ id: uuid, raw: "Referenced body", collapsed: false, children: [] }],
      }]);
      loadSingle({ name: "Source", kind: "page", title: "Source", pre_block: null,
        blocks: [{ id: "source-link", raw, collapsed: false, children: [] }] });
      const root = document.createElement("div");
      document.body.append(root);
      const dispose = render(() => <Block id="source-link" />, root);
      try {
        const anchor = await vi.waitFor(() => {
          const element = root.querySelector<HTMLElement>(selector);
          expect(element).not.toBeNull();
          if (surface === "block reference") expect(element?.textContent).toContain("Referenced body");
          return element!;
        });
        anchor.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, ...modifier }));
        anchor.dispatchEvent(new MouseEvent(_gesture === "middle" ? "auxclick" : "click", { bubbles: true, cancelable: true, ...modifier }));
        if (destination === "sidebar") {
          expect(rightSidebar()[0]).toMatchObject(surface === "block reference" ? { kind: "block", page: "Destination" } : { kind: "page", name: "Destination" });
          expect(paneRouter("main").route()).toEqual({ kind: "journals" });
        } else if (destination === "background") {
          expect(paneRouter("main").tabs()).toHaveLength(2);
          expect(paneRouter("main").route()).toEqual({ kind: "journals" });
          expect(paneRouter("main").tabs()[1].history[0]).toMatchObject({ kind: "page", name: "Destination" });
        } else {
          expect(layoutPaneIds()).toHaveLength(2);
          expect(paneRouter(layoutPaneIds()[1]).route()).toMatchObject({ kind: "page", name: "Destination" });
        }
      } finally { dispose(); }
    });
  }
});
