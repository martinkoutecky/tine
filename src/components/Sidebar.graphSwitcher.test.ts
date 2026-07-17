import { describe, expect, it, vi } from "vitest";
import { openKnownGraph, openSidebarPageTarget, type KnownGraphOpenDeps, type SidebarPageOpenDeps } from "./Sidebar";
import { favorites, isFavorite, setAliasMap, setFavorites, toggleFavorite } from "../ui";

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
  it("resolves the canonical page for normal, sidebar, new-tab, and context gestures", () => {
    setAliasMap({ shortcut: "Canonical" });
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

    setAliasMap({});
  });

  it("resolves mixed-case real-page identities across every sidebar gesture", () => {
    setAliasMap({ page1: "page1" });
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
    setAliasMap({});
  });
});
