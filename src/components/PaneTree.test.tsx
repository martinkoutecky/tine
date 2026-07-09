import { afterEach, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { PaneTree } from "../App";
import { layoutRoot, resetPaneLayoutToSingle, restorePaneLayout } from "../panes";
import { setPaneSel } from "../paneSelect";
import { resetStore } from "../store";
import type { PaneSnapshot } from "../router";

const pageSnapshot = (name: string): PaneSnapshot => ({
  tabs: [{ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false }],
  activeIndex: 0,
});

afterEach(() => {
  document.body.innerHTML = "";
  setPaneSel(null);
  resetStore();
  resetPaneLayoutToSingle({
    tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }],
    activeIndex: 0,
  });
});

describe("PaneTree", () => {
  it("renders split leaves with independent pane tab bars", () => {
    restorePaneLayout(
      {
        kind: "split",
        dir: "row",
        ratio: 0.5,
        children: [
          { kind: "pane", paneId: "main" },
          { kind: "pane", paneId: "pane-2" },
        ],
      },
      new Map([["main", pageSnapshot("Left")], ["pane-2", pageSnapshot("Right")]]),
      "main"
    );
    const root = document.createElement("div");
    document.body.appendChild(root);

    const dispose = render(() => <PaneTree node={layoutRoot()} path={[]} />, root);

    expect(root.querySelectorAll("[data-pane-id]")).toHaveLength(2);
    expect(root.querySelectorAll(".pane-tab-bar")).toHaveLength(2);
    expect(root.querySelector('[data-pane-id="main"] .tab-title')?.textContent).toContain("Left");
    expect(root.querySelector('[data-pane-id="pane-2"] .tab-title')?.textContent).toContain("Right");
    dispose();
  });

  it("remounts a leaf whose paneId changes in place (frozen-router regression)", () => {
    const split = (second: string): Extract<ReturnType<typeof layoutRoot>, { kind: "split" }> => ({
      kind: "split",
      dir: "row",
      ratio: 0.5,
      children: [
        { kind: "pane", paneId: "main" },
        { kind: "pane", paneId: second },
      ],
    });
    restorePaneLayout(
      split("pane-2"),
      new Map([["main", pageSnapshot("Left")], ["pane-2", pageSnapshot("Right")]]),
      "main"
    );
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <PaneTree node={layoutRoot()} path={[]} />, root);
    expect(root.querySelector('[data-pane-id="pane-2"] .tab-title')?.textContent).toContain("Right");

    // Same tree POSITION, different pane identity: PaneLeaf must remount with
    // the new pane's router, not keep serving the old one's tabs.
    restorePaneLayout(
      split("pane-3"),
      new Map([["main", pageSnapshot("Left")], ["pane-3", pageSnapshot("Third")]]),
      "main"
    );
    expect(root.querySelector('[data-pane-id="pane-3"] .tab-title')?.textContent).toContain("Third");
    dispose();
  });

  it("renders pane-select ring and seam highlight targets", () => {
    restorePaneLayout(
      {
        kind: "split",
        dir: "row",
        ratio: 0.5,
        children: [
          { kind: "pane", paneId: "main" },
          { kind: "pane", paneId: "pane-2" },
        ],
      },
      new Map([["main", pageSnapshot("Left")], ["pane-2", pageSnapshot("Right")]]),
      "main"
    );
    setPaneSel({ kind: "pane", paneId: "pane-2" });
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <PaneTree node={layoutRoot()} path={[]} />, root);

    expect(root.querySelector('[data-pane-id="pane-2"]')?.classList.contains("pane-selected")).toBe(true);

    setPaneSel({ kind: "seam", path: [] });
    expect(root.querySelector(".pane-resizer")?.classList.contains("pane-seam-selected")).toBe(true);
    dispose();
  });
});
