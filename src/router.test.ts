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
  togglePin,
  closeTab,
  setActiveTab,
  sameRoute,
  resetTabsToJournals,
} from "./router";

// The router holds singleton tab state, so reset to a single unpinned journals
// tab before each test. confirm() is stubbed true so closing pinned tabs (which
// now prompts) doesn't hang the teardown.
beforeEach(() => {
  vi.stubGlobal("confirm", () => true);
  // Unpin everything first so closeTab (which prompts for pinned tabs, via the
  // mock's window.confirm) takes the synchronous unpinned path during teardown.
  for (const t of tabs()) if (t.pinned) togglePin(t.id);
  while (tabs().length > 1) void closeTab(tabs()[tabs().length - 1].id);
  setActiveTab(tabs()[0].id);
  openJournals(); // reset its route in place
});

const pinActive = () => togglePin(activeId());

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
