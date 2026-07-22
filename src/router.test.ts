import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  tabs,
  activeId,
  activeTab,
  route,
  tabRoute,
  openPage,
  openFile,
  openJournals,
  openInNewTab,
  focusBlock,
  togglePin,
  closeTab,
  setActiveTab,
  sameRoute,
  resetTabsToJournals,
  reopenClosedTab,
  activateNextTab,
  activatePrevTab,
  openQueryInNewTab,
  updateActiveQuery,
  replaceActiveRoute,
} from "./router";
import { setNavReuseTabs } from "./navSettings";
import { setDoc } from "./store";

// The router holds singleton tab state, so reset to a single unpinned journals
// tab before each test. confirm() is stubbed true so closing pinned tabs (which
// now prompts) doesn't hang the teardown.
beforeEach(() => {
  vi.stubGlobal("confirm", () => true);
  setNavReuseTabs(true);
  setDoc({ byId: {}, pages: [], feed: [], loaded: false });
  // Unpin everything first so closeTab (which prompts for pinned tabs, via the
  // mock's window.confirm) takes the synchronous unpinned path during teardown.
  for (const t of tabs()) if (t.pinned) togglePin(t.id);
  while (tabs().length > 1) void closeTab(tabs()[tabs().length - 1].id);
  setActiveTab(tabs()[0].id);
  openJournals(); // reset its route in place
});

const pinActive = () => togglePin(activeId());

describe("independent empty query workspaces (GH #172)", () => {
  it("keeps same-timestamp empty query routes identity-distinct through edits, presentation, and tab history", () => {
    vi.spyOn(Date, "now").mockReturnValue(1_725_000_000_000);
    const first = openQueryInNewTab("", "search", true);
    const firstTab = activeId();
    const second = openQueryInNewTab("", "search", true);
    const secondTab = activeId();
    expect(first.id).not.toBe(second.id);
    expect(sameRoute(first, second)).toBe(false);

    setActiveTab(firstTab);
    updateActiveQuery({ source: "alpha", presentation: "table" });
    expect(route()).toMatchObject({ id: first.id, source: "alpha", presentation: "table" });
    expect(activeTab().history).toHaveLength(1);
    setActiveTab(secondTab);
    expect(route()).toMatchObject({ id: second.id, source: "", presentation: "search" });
    expect(activeTab().history).toHaveLength(1);
    vi.restoreAllMocks();
  });
});

describe("reuse already-open tabs on user navigation", () => {
  it("edits a virtual query in one stable history entry and can materialize it in place", () => {
    openQueryInNewTab("alpha", "search", true);
    const queryTab = activeTab();
    const original = route();
    expect(original).toMatchObject({ kind: "query", source: "alpha", presentation: "search" });

    updateActiveQuery({ source: "alpha -draft" });
    updateActiveQuery({ presentation: "table" });

    expect(activeTab().history).toHaveLength(1);
    expect(route()).toMatchObject({
      kind: "query",
      id: original.kind === "query" ? original.id : "",
      source: "alpha -draft",
      presentation: "table",
    });

    replaceActiveRoute({ kind: "page", name: "Saved search", pageKind: "page" });
    expect(activeId()).toBe(queryTab.id);
    expect(activeTab().history).toEqual([{ kind: "page", name: "Saved search", pageKind: "page" }]);
  });

  it("sticky navigation to a route already open in another pinned tab focuses it without changing the active tab's content", () => {
    pinActive();
    const journalsId = activeId();
    openInNewTab({ kind: "page", name: "Draft", pageKind: "page" }, true);
    const draftId = activeId();

    openJournals();

    expect(activeId()).toBe(journalsId);
    expect(tabs().length).toBe(2);
    expect(tabRoute(tabs().find((t) => t.id === draftId)!)).toEqual({
      kind: "page",
      name: "Draft",
      pageKind: "page",
    });
    expect(tabRoute(tabs().find((t) => t.id === journalsId)!)).toEqual({ kind: "journals" });
  });

  it("prefers the first matching tab in strip order", () => {
    openInNewTab({ kind: "page", name: "Target", pageKind: "page" }, true);
    const firstTargetId = activeId();
    togglePin(firstTargetId);
    openInNewTab({ kind: "page", name: "Target", pageKind: "page" }, true);
    openInNewTab({ kind: "page", name: "Other", pageKind: "page" }, true);

    openPage("Target");

    expect(activeId()).toBe(firstTargetId);
  });

  it("toggle OFF restores duplicate/replace behavior", () => {
    setNavReuseTabs(false);
    pinActive();
    const journalsId = activeId();
    openInNewTab({ kind: "page", name: "Draft", pageKind: "page" }, true);
    const draftId = activeId();

    openJournals();

    expect(activeId()).toBe(draftId);
    expect(tabs().length).toBe(2);
    expect(tabRoute(tabs().find((t) => t.id === draftId)!)).toEqual({ kind: "journals" });
    expect(tabRoute(tabs().find((t) => t.id === journalsId)!)).toEqual({ kind: "journals" });
  });

  it("inPlace navigation never retargets to another tab", () => {
    openInNewTab({ kind: "page", name: "Target", pageKind: "page" }, true);
    const existingTargetId = activeId();
    setActiveTab(tabs()[0].id);
    const sourceId = activeId();

    openPage("Target", "page", { inPlace: true });

    expect(activeId()).toBe(sourceId);
    expect(activeId()).not.toBe(existingTargetId);
    expect(tabRoute(activeTab())).toEqual({ kind: "page", name: "Target", pageKind: "page" });
  });

  it("zoom navigation never retargets to another tab", () => {
    setDoc("byId", "block-zoom", {
      id: "block-zoom",
      raw: "Zoom target",
      collapsed: false,
      parent: null,
      page: "Target",
      children: [],
    });
    const zoomed = { kind: "page" as const, name: "Target", pageKind: "page" as const, block: "block-zoom" };
    openInNewTab(zoomed, true);
    const existingZoomId = activeId();
    setActiveTab(tabs()[0].id);
    openPage("Other");
    const sourceId = activeId();

    focusBlock("block-zoom");

    expect(activeId()).toBe(sourceId);
    expect(activeId()).not.toBe(existingZoomId);
    expect(route()).toEqual(zoomed);
  });

  it("stores a fresh block's durable UUID in the persistent zoom route", () => {
    const uuid = "12345678-1234-4234-8234-123456789abc";
    vi.spyOn(crypto, "randomUUID").mockReturnValue(uuid);
    setDoc({
      byId: {
        "bfresh-route": {
          id: "bfresh-route",
          raw: "Fresh route target",
          collapsed: false,
          parent: null,
          page: "Target",
          children: [],
        },
      },
      pages: [{
        name: "Target", kind: "page", title: "Target", preBlock: null,
        roots: ["bfresh-route"], format: "md", readOnly: false, guide: false,
        path: "pages/Target.md",
      }],
      feed: ["Target"],
      loaded: true,
    });

    focusBlock("bfresh-route");

    expect(route()).toEqual({
      kind: "page",
      name: "Target",
      pageKind: "page",
      path: "pages/Target.md",
      block: uuid,
    });
    expect((route() as { block?: string }).block).not.toBe("bfresh-route");
  });

  it("explicit new-tab navigation still duplicates", () => {
    const target = { kind: "page" as const, name: "Target", pageKind: "page" as const };
    openInNewTab(target, true);
    setActiveTab(tabs()[0].id);
    const sourceId = activeId();

    openInNewTab(target);

    expect(activeId()).toBe(sourceId);
    expect(tabs().filter((t) => sameRoute(tabRoute(t), target))).toHaveLength(2);
  });

  it("navigating to the current route is a no-op", () => {
    openPage("Current");
    const id = activeId();
    const historyLength = activeTab().history.length;

    openPage("Current");

    expect(activeId()).toBe(id);
    expect(activeTab().history).toHaveLength(historyLength);
    expect(route()).toEqual({ kind: "page", name: "Current", pageKind: "page" });
  });
});

describe("sticky (pinned) tabs", () => {
  it("a user navigation from a pinned tab opens a new FOREGROUND tab, leaving the pinned view", () => {
    pinActive();
    const pinnedId = activeId();
    expect(activeTab().pinned).toBe(true);

    openPage("Some Page");

    // A new tab was created and focused; the pinned tab still shows journals.
    expect(tabs().length).toBe(2);
    expect(activeId()).not.toBe(pinnedId);
    expect(activeTab().pinned).toBe(false);
    expect(route()).toEqual({ kind: "page", name: "Some Page", pageKind: "page" });
    const pinned = tabs().find((t) => t.id === pinnedId)!;
    expect(tabRoute(pinned)).toEqual({ kind: "journals" });
  });

  it("an UNPINNED tab navigates in place (no new tab)", () => {
    openPage("Page A");
    expect(tabs().length).toBe(1);
    expect(route()).toEqual({ kind: "page", name: "Page A", pageKind: "page" });
  });

  it("navigating to the route the pinned tab already shows is a no-op (no stray tab)", () => {
    openPage("Repeat");
    pinActive();
    openPage("Repeat"); // same route → no redirect, no new tab
    expect(tabs().length).toBe(1);
  });

  it("inPlace navigation overrides stickiness (used by graph switch / post-delete)", () => {
    pinActive();
    openJournals({ inPlace: true });
    expect(tabs().length).toBe(1);
    expect(activeTab().pinned).toBe(true);
  });

  it("closing a pinned tab asks first and respects the answer", async () => {
    openInNewTab({ kind: "page", name: "Keep", pageKind: "page" }, true);
    const id = activeId();
    togglePin(id);
    vi.stubGlobal("confirm", () => false); // backend.confirm → mock → window.confirm
    await closeTab(id);
    expect(tabs().some((t) => t.id === id)).toBe(true); // cancelled → still open
    vi.stubGlobal("confirm", () => true);
    await closeTab(id);
    expect(tabs().some((t) => t.id === id)).toBe(false); // confirmed → closed
  });
});

describe("path-pinned routes (#21 — reach a duplicate-day stray)", () => {
  it("openFile pins the route to a specific file path", () => {
    openFile("journals/Friday, 26-06-2026.org", "Friday, 26-06-2026", "journal");
    expect(route()).toEqual({
      kind: "page",
      name: "Friday, 26-06-2026",
      pageKind: "journal",
      path: "journals/Friday, 26-06-2026.org",
    });
  });

  it("sameRoute distinguishes two same-name pages pinned to different files", () => {
    const canonical = {
      kind: "page" as const,
      name: "Friday, 26-06-2026",
      pageKind: "journal" as const,
    };
    const stray = { ...canonical, path: "journals/Friday, 26-06-2026.org" };
    const strayB = { ...canonical, path: "journals/2026_06_26.org" };
    expect(sameRoute(stray, stray)).toBe(true);
    expect(sameRoute(stray, canonical)).toBe(false); // path-pinned ≠ by-name
    expect(sameRoute(stray, strayB)).toBe(false); // two distinct files
  });

  it("navigating from a name route to the same name pinned to a file is NOT a no-op", () => {
    openPage("Friday, 26-06-2026", "journal");
    expect(route().kind).toBe("page");
    openFile("journals/Friday, 26-06-2026.org", "Friday, 26-06-2026", "journal");
    // The path-pinned route is distinct, so it actually navigates (new history entry).
    expect((route() as { path?: string }).path).toBe("journals/Friday, 26-06-2026.org");
  });

  it("retains the loaded physical owner while zooming into and back out of a block", () => {
    const path = "pages/client-b/Twin.md";
    const id = "11111111-1111-4111-8111-111111111111";
    setDoc({
      byId: {
        [id]: { id, raw: "Client B", collapsed: false, parent: null, page: "Twin", children: [] },
      },
      pages: [{
        name: "Twin", kind: "page", title: "Twin", preBlock: null, roots: [id],
        format: "md", readOnly: false, guide: false, path,
      }],
      feed: ["Twin"],
      loaded: true,
    });
    openFile(path, "Twin", "page");

    focusBlock(id);
    expect(route()).toEqual({ kind: "page", name: "Twin", pageKind: "page", path, block: id });

    focusBlock(null);
    expect(route()).toEqual({ kind: "page", name: "Twin", pageKind: "page", path });
  });
});

describe("pinned-left ordering", () => {
  it("pinning moves the tab to the right end of the pinned group; pinned stay left", () => {
    // Three tabs: [journals, A, B], A active.
    openInNewTab({ kind: "page", name: "A", pageKind: "page" });
    openInNewTab({ kind: "page", name: "B", pageKind: "page" });
    const aId = tabs()[1].id;
    setActiveTab(aId);

    togglePin(aId); // pin A → should slide to the front (boundary = index 0)
    expect(tabs()[0].id).toBe(aId);
    expect(tabs()[0].pinned).toBe(true);

    // Pin B too → it goes to the end of the pinned group (rightmost pinned).
    const bId = tabs().find((t) => tabRoute(t).kind === "page" && (tabRoute(t) as any).name === "B")!.id;
    togglePin(bId);
    const pinnedIds = tabs().filter((t) => t.pinned).map((t) => t.id);
    expect(pinnedIds).toEqual([aId, bId]); // A then B, all to the left
    expect(tabs().every((t, i) => (t.pinned ? i < pinnedIds.length : true))).toBe(true);
  });

  it("unpinning moves the tab to the left end of the unpinned group", () => {
    openInNewTab({ kind: "page", name: "X", pageKind: "page" });
    const xId = tabs().find((t) => tabRoute(t).kind === "page" && (tabRoute(t) as any).name === "X")!.id;
    togglePin(xId); // X pinned, now leftmost
    expect(tabs()[0].id).toBe(xId);
    togglePin(xId); // unpin → boundary = first unpinned slot (index 0 here, journals follows)
    expect(tabs()[0].id).toBe(xId);
    expect(tabs()[0].pinned).toBe(false);
  });
});

describe("graph switch tab reset", () => {
  it("collapses every tab to a single fresh journals tab", () => {
    openPage("Page A");
    openInNewTab({ kind: "page", name: "Page B", pageKind: "page" });
    pinActive(); // pin one for good measure
    expect(tabs().length).toBeGreaterThan(1);

    resetTabsToJournals();

    expect(tabs().length).toBe(1);
    expect(tabs()[0].pinned).toBe(false);
    expect(route()).toEqual({ kind: "journals" });
    expect(activeId()).toBe(tabs()[0].id);
  });
});

describe("reopen closed tab (Ctrl+Shift+T)", () => {
  it("restores the most-recently-closed tab's route and focuses it", async () => {
    openInNewTab({ kind: "page", name: "Gamma", pageKind: "page" }, true);
    const gammaId = activeId();
    expect(route()).toEqual({ kind: "page", name: "Gamma", pageKind: "page" });

    await closeTab(gammaId);
    expect(tabs().some((t) => t.id === gammaId)).toBe(false);

    reopenClosedTab();
    expect(route()).toEqual({ kind: "page", name: "Gamma", pageKind: "page" });
    expect(activeTab().pinned).toBe(false);
  });

  it("reopens most-recent first (LIFO)", async () => {
    openInNewTab({ kind: "page", name: "One", pageKind: "page" }, true);
    const oneId = activeId();
    openInNewTab({ kind: "page", name: "Two", pageKind: "page" }, true);
    const twoId = activeId();

    await closeTab(oneId);
    await closeTab(twoId);

    reopenClosedTab(); // Two was closed last → comes back first
    expect(route()).toEqual({ kind: "page", name: "Two", pageKind: "page" });
    reopenClosedTab(); // then One
    expect(route()).toEqual({ kind: "page", name: "One", pageKind: "page" });
  });
});

describe("browser-style tab cycling (Ctrl+PgDn / Ctrl+PgUp)", () => {
  it("moves to the next / previous tab and wraps around", () => {
    // journals (idx0) + A (idx1) + B (idx2); B is active (last opened, foreground).
    openInNewTab({ kind: "page", name: "A", pageKind: "page" }, true);
    openInNewTab({ kind: "page", name: "B", pageKind: "page" }, true);
    const ids = tabs().map((t) => t.id);
    expect(activeId()).toBe(ids[2]);

    activateNextTab(); // wraps to first
    expect(activeId()).toBe(ids[0]);
    activatePrevTab(); // wraps back to last
    expect(activeId()).toBe(ids[2]);
    activatePrevTab();
    expect(activeId()).toBe(ids[1]);
  });
});
