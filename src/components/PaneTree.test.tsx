import { afterEach, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { PaneTree } from "../App";
import { focusPane, layoutRoot, paneRouter, resetPaneLayoutToSingle, restorePaneLayout, type LayoutNode } from "../panes";
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

  it("wires query source focus to the active pane and route ID without stealing same-route control focus", async () => {
    const query = (id: string, source = ""): PaneSnapshot => ({
      tabs: [{ history: [{ kind: "query", id, sourceKind: "search", source, presentation: "search" }], pos: 0, pinned: false }],
      activeIndex: 0,
    });
    const rootNode: LayoutNode = {
      kind: "split" as const, dir: "row" as const, ratio: 0.5,
      children: [{ kind: "pane" as const, paneId: "main" }, { kind: "pane" as const, paneId: "pane-2" }],
    };
    const main: PaneSnapshot = {
      tabs: [
        { history: [{ kind: "query", id: "query-main-a", sourceKind: "search", source: "", presentation: "search" }], pos: 0, pinned: false },
        { history: [{ kind: "query", id: "query-main-b", sourceKind: "search", source: "beta", presentation: "search" }], pos: 0, pinned: false },
      ], activeIndex: 0,
    };
    restorePaneLayout(rootNode, new Map([["main", main], ["pane-2", query("query-side", "alpha")]]), "pane-2");
    const host = document.createElement("div"); document.body.append(host);
    const dispose = render(() => <PaneTree node={layoutRoot()} path={[]} />, host);
    await Promise.resolve(); await Promise.resolve();
    expect(document.activeElement).toBe(host.querySelector('[data-pane-id="pane-2"] .query-workspace-source'));

    focusPane("main");
    await Promise.resolve(); await Promise.resolve();
    // `query-main-a` mounted while pane-2 owned focus. Restoring/activating
    // main must focus that already-active route without requiring a tab change.
    expect(document.activeElement).toBe(host.querySelector('[data-pane-id="main"] .query-workspace-source'));
    paneRouter("main").setActiveTab(paneRouter("main").tabs()[1].id);
    await Promise.resolve(); await Promise.resolve();
    const activeSource = host.querySelector<HTMLInputElement>('[data-pane-id="main"] .query-workspace-source')!;
    expect(activeSource.value).toBe("beta");
    expect(document.activeElement).toBe(activeSource);
    const presentation = host.querySelector<HTMLButtonElement>('[data-pane-id="main"] .query-presentations button:not(.active)')!;
    presentation.focus();
    paneRouter("main").updateActiveQuery({ source: " beta ", presentation: "table" });
    await Promise.resolve(); await Promise.resolve();
    expect(document.activeElement).toBe(presentation);
    dispose();

    restorePaneLayout(rootNode, new Map([["main", main], ["pane-2", query("query-side", "alpha")]]), "main");
    const secondHost = document.createElement("div"); document.body.append(secondHost);
    const disposeSecond = render(() => <PaneTree node={layoutRoot()} path={[]} />, secondHost);
    await Promise.resolve(); await Promise.resolve();
    expect(document.activeElement).toBe(secondHost.querySelector('[data-pane-id="main"] .query-workspace-source'));
    disposeSecond();
  });

  it("renders the solo-pane edge highlight OUTSIDE the scroller (so it doesn't scroll away)", () => {
    // Single pane (the afterEach reset leaves exactly this). Target its right
    // edge, as ArrowRight in pane-select does on a solo pane.
    setPaneSel({ kind: "pane-edge", paneId: "main", side: "right" });
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <PaneTree node={layoutRoot()} path={[]} />, root);

    const seg = root.querySelector(".pane-edge-seg");
    expect(seg).not.toBeNull();
    // Regression guard: the highlight must NOT live inside .main-content (the
    // scroll container) — there it scrolled off-screen on a tall page and the
    // arrows looked dead. It belongs to the non-scrolling shell.
    const scroller = root.querySelector(".main-content");
    expect(scroller?.contains(seg)).toBe(false);
    expect(root.querySelector(".main-content-shell")?.contains(seg)).toBe(true);
    dispose();
  });
});
