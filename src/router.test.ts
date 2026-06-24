import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  tabs,
  activeId,
  activeTab,
  route,
  tabRoute,
  openPage,
  openJournals,
  openInNewTab,
  togglePin,
  closeTab,
  setActiveTab,
} from "./router";

// The router holds singleton tab state, so reset to a single unpinned journals
// tab before each test. confirm() is stubbed true so closing pinned tabs (which
// now prompts) doesn't hang the teardown.
beforeEach(() => {
  vi.stubGlobal("confirm", () => true);
  // Collapse to one tab.
  while (tabs().length > 1) closeTab(tabs()[tabs().length - 1].id);
  setActiveTab(tabs()[0].id);
  if (tabs()[0].pinned) togglePin(tabs()[0].id); // unpin
  openJournals(); // reset its route in place (now unpinned)
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
