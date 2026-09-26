import { describe, expect, it, vi } from "vitest";
import { openKnownGraph, openSidebarPageTarget, type KnownGraphOpenDeps, type SidebarPageOpenDeps } from "./Sidebar";
import { favorites, isFavorite, pageIdentityKey, setFavorites, toggleFavorite } from "../ui";
import { backend } from "../backend";
import { refreshPageIndex, resetPageIndex } from "../pageIndex";
import type { PageInventoryEntry, ResolvedPage } from "../types";

// Seed the one page index the way the backend would answer (ported from the
// deleted frontend alias-map seeding: the same names, now as inventory rows).
async function seedIndex(...rows: Array<[name: string, target: ResolvedPage]>) {
  const entries: PageInventoryEntry[] = rows.map(([name, target]) => ({
    key: pageIdentityKey(name), name, is_journal: false, day: null, target,
  }));
  vi.spyOn(backend(), "pageInventory").mockResolvedValue({ rev: "1", entries });
  resetPageIndex();
  await refreshPageIndex();
}
const file = (id: string): ResolvedPage => ({ kind: "existing", id, others: [] });

describe("known graph open gesture", () => {
  it("uses an in-place switch for an ordinary click", async () => {
    const deps: KnownGraphOpenDeps = {
      switchInPlace: vi.fn().mockResolvedValue(undefined),
      openNewWindow: vi.fn().mockResolvedValue(undefined),
    };
    await openKnownGraph("/graphs/a", false, deps);
    expect(deps.switchInPlace).toHaveBeenCalledWith("/graphs/a");
    expect(deps.openNewWindow).not.toHaveBeenCalled();
  });

  it("opens a new OS window for shift-click", async () => {
    const deps: KnownGraphOpenDeps = {
      switchInPlace: vi.fn().mockResolvedValue(undefined),
      openNewWindow: vi.fn().mockResolvedValue({ kind: "loaded" }),
    };
    await openKnownGraph("/graphs/b", true, deps);
    expect(deps.openNewWindow).toHaveBeenCalledWith("/graphs/b");
    expect(deps.switchInPlace).not.toHaveBeenCalled();
  });
});

describe("favorite alias navigation", () => {
  it("resolves the canonical page for normal, sidebar, new-tab, and context gestures", async () => {
    await seedIndex(["Canonical", file("pages/Canonical.md")], ["shortcut", { kind: "alias", owners: ["pages/Canonical.md"] }]);
    const deps: SidebarPageOpenDeps = {
      normal: vi.fn(),
      sidebar: vi.fn(),
      newTab: vi.fn(),
      context: vi.fn(),
    };

    openSidebarPageTarget("Shortcut", "page", "normal", undefined, deps);
    expect(deps.normal).toHaveBeenCalledWith("Canonical", "page");
    openSidebarPageTarget("Shortcut", "page", "sidebar", undefined, deps);
    expect(deps.sidebar).toHaveBeenCalledWith("Canonical", "page");
    openSidebarPageTarget("Shortcut", "page", "new-tab", undefined, deps);
    expect(deps.newTab).toHaveBeenCalledWith("Canonical", "page");
    openSidebarPageTarget("Shortcut", "page", "context", { x: 12, y: 34 }, deps);
    expect(deps.context).toHaveBeenCalledWith(12, 34, "Canonical", "page");

    setFavorites([{ name: "Shortcut", kind: "page" }]);
    expect(isFavorite("Canonical")).toBe(true);
    toggleFavorite("Canonical", "page");
    expect(favorites()).toEqual([]);

    resetPageIndex();
    vi.restoreAllMocks();
  });

  it("resolves mixed-case real-page identities across every sidebar gesture", async () => {
    await seedIndex(["page1", file("pages/page1.md")]);
    const deps: SidebarPageOpenDeps = {
      normal: vi.fn(),
      sidebar: vi.fn(),
      newTab: vi.fn(),
      context: vi.fn(),
    };

    openSidebarPageTarget("Page1", "page", "normal", undefined, deps);
    openSidebarPageTarget("PAGE1", "page", "sidebar", undefined, deps);
    openSidebarPageTarget("pAgE1", "page", "new-tab", undefined, deps);
    openSidebarPageTarget("PaGe1", "page", "context", { x: 4, y: 8 }, deps);

    expect(deps.normal).toHaveBeenCalledWith("page1", "page");
    expect(deps.sidebar).toHaveBeenCalledWith("page1", "page");
    expect(deps.newTab).toHaveBeenCalledWith("page1", "page");
    expect(deps.context).toHaveBeenCalledWith(4, 8, "page1", "page");
    resetPageIndex();
    vi.restoreAllMocks();
  });

  it("uses the same contextual Unicode lowercase key as core refs::page_key", async () => {
    expect(pageIdentityKey(" ΟΣ ")).toBe("ος");
    expect(pageIdentityKey("/Cafe\u{301}/")).toBe("café");
    await seedIndex(["ΟΣ", file("pages/ΟΣ.md")]);
    const normal = vi.fn();
    openSidebarPageTarget("ΟΣ", "page", "normal", undefined, {
      normal,
      sidebar: vi.fn(),
      newTab: vi.fn(),
      context: vi.fn(),
    });
    expect(normal).toHaveBeenCalledWith("ΟΣ", "page");
    resetPageIndex();
    vi.restoreAllMocks();
  });
});
