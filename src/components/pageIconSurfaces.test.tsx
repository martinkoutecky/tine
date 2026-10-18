import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import "../graph";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { bindTestHost } from "../document/host/wiring.test.support";
import { loadSingle, reloadPage } from "../document/workingSet";
import { resetStore, setPageProperty, setRaw } from "../document";
import { bumpDataRev } from "../graphSession";
import { applySidebarSession, seedFavorites, setRecentPages, setRightSidebar } from "../ui";
import { openJournals } from "../router";
import { installPageIndex, refreshPageIndex, resetPageIndex } from "../pageIndex";
import type { PageDto } from "../types";
import { Sidebar } from "./Sidebar";
import { RightSidebar } from "./RightSidebar";
import { clearAssetBlobCache } from "../assetCache";
import { refreshAsset } from "../assetCache";
import { PageView } from "./Page";
import { mainPaneRouter } from "../router";

// GH #346: a page's `icon::` shows wherever OG shows it. OG (og-1.0.0
// src/main/frontend): page title (components/page.cljs:357), All pages table
// (page.cljs:1245), left sidebar Favorites and Recent (components/container.cljs
// get-page-icon, used at :226 and :256), right-sidebar page item (right_sidebar.cljs:151).
// OG shows no icon in search results or namespace rows beyond page refs.
const settle = async () => { for (let i = 0; i < 30; i++) await Promise.resolve(); await new Promise((r) => setTimeout(r, 0)); };

beforeAll(async () => { await initParser(); });
beforeEach(async () => {
  await bindTestHost();
  vi.spyOn(backend(), "pageIcons").mockImplementation(async (names: string[]) =>
    Object.fromEntries(names.filter((n) => n.startsWith("Icon")).map((n) => [n, "🏁"])));
  vi.spyOn(backend(), "getBacklinks").mockResolvedValue([]);
  vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([]);
  vi.spyOn(backend(), "getBlockRefCounts").mockResolvedValue({});
});
afterEach(async () => {
  await settle();
  seedFavorites([]);
  setRecentPages([]);
  setRightSidebar([]);
  applySidebarSession({ right: false, items: [] });
  resetStore();
  clearAssetBlobCache();
  document.body.innerHTML = "";
  openJournals();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

// EmojiText draws an emoji as <img alt=…> on some platforms: read the logical text.
const logical = (n: Node): string => n.nodeType === 3 ? n.textContent ?? "" : (n as Element).tagName === "IMG" ? (n as Element).getAttribute("alt") ?? "" : [...n.childNodes].map(logical).join("");
const labels = (root: Element, selector: string) => [...root.querySelectorAll(selector)].map((n) => logical(n).trim());

describe("left sidebar rows carry the page icon", () => {
  it("title and Favorites share reads, follow replacement and property edits, and refuse remote images", async () => {
    const page = { id: "pages/Image.md", name: "Image", kind: "page" as const, title: "Image", pre_block: "tine.icon:: ![](../assets/a.png)", blocks: [] };
    loadSingle(page);
    vi.spyOn(backend(), "getPage").mockResolvedValue(page);
    vi.spyOn(backend(), "getPageByPath").mockResolvedValue(page);
    const read = vi.spyOn(backend(), "readAsset").mockResolvedValue(Uint8Array.from(atob("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aFOkAAAAASUVORK5CYII="), c => c.charCodeAt(0)));
    let version = 0;
    vi.stubGlobal("URL", Object.assign(class extends URL {}, { createObjectURL: vi.fn(() => `blob:icon-${++version}`), revokeObjectURL: vi.fn() }));
    seedFavorites(["Image"]);
    mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Image", pageKind: "page" });
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <><Sidebar /><PageView /></>, root);
    const images = () => [...root.querySelectorAll<HTMLImageElement>('.page-title-icon img[data-page-image-icon], #sidebar-favorites-list img[data-page-image-icon]')];
    const noIcon = () => {
      expect(images()).toHaveLength(0);
      expect(root.querySelector(".page-title-icon")?.textContent ?? "").toBe("");
      expect(labels(root, "#sidebar-favorites-list .nav-page-label")).toEqual(["Image"]);
    };
    try {
      await vi.waitFor(() => expect(images().map(img => img.getAttribute("src"))).toEqual(["blob:icon-1", "blob:icon-1"]));
      expect(read).toHaveBeenCalledTimes(1);
      refreshAsset("a.png");
      await vi.waitFor(() => expect(images().map(img => img.getAttribute("src"))).toEqual(["blob:icon-2", "blob:icon-2"]));
      expect(read).toHaveBeenCalledTimes(2);
      images()[0].dispatchEvent(new Event("error"));
      expect(root.querySelector(".page-title-icon")?.textContent ?? "").toBe("");
      setPageProperty("Image", "tine.icon", "![](../assets/b.png)");
      await vi.waitFor(() => expect(images().map(img => img.getAttribute("src"))).toEqual(["blob:icon-3", "blob:icon-3"]));
      read.mockRejectedValue(new Error("not-found"));
      setPageProperty("Image", "tine.icon", "![](../assets/missing.png)");
      await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(4));
      await settle();
      noIcon();
      read.mockRejectedValue(new Error("permission-denied"));
      setPageProperty("Image", "tine.icon", "![](../assets/unreadable.png)");
      await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(5));
      await settle();
      noIcon();
      for (const value of ["![](https://example.com/assets/icon.png)", "![](data:image/png;base64,AAAA)", "![](../images/a.png)", "![](../assets/a.gif)", "![](../../assets/a.png)"]) {
        setPageProperty("Image", "tine.icon", value);
        await settle();
        noIcon();
      }
      expect(root.querySelector('img[src^="https://example.com"]')).toBeNull();
      expect(read).toHaveBeenCalledTimes(5);
      setPageProperty("Image", "tine.icon", "AB");
      expect(root.querySelector(".page-title-icon")?.textContent).toBe("AB");
      expect(labels(root, "#sidebar-favorites-list .nav-page-label")).toEqual(["AB Image"]);
    } finally { dispose(); }
  });
  it("renders a local image icon from the OG-compatible icon:: fallback in Favorites without using its link sizing", async () => {
    loadSingle({ id: "pages/Image.md", name: "Image", kind: "page", title: "Image", pre_block: 'icon:: ![](../assets/logo.PNG){:width "90%"}', blocks: [] });
    vi.spyOn(backend(), "readAsset").mockResolvedValue(Uint8Array.from(atob("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aFOkAAAAASUVORK5CYII="), c => c.charCodeAt(0)));
    vi.stubGlobal("URL", Object.assign(class extends URL {}, { createObjectURL: vi.fn(() => "blob:page-icon"), revokeObjectURL: vi.fn() }));
    seedFavorites(["Image"]);
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <Sidebar />, root);
    try {
      await vi.waitFor(() => expect(root.querySelector('#sidebar-favorites-list img[data-page-image-icon]')?.getAttribute("src")).toBe("blob:page-icon"));
      expect(backend().readAsset).toHaveBeenCalledWith("logo.PNG", expect.any(Number));
    } finally { dispose(); }
  });
  it("tine.icon:: wins over icon::, and a change to either property updates title and Favorites", async () => {
    const page = { id: "pages/Both.md", name: "Both", kind: "page" as const, title: "Both", pre_block: "icon:: 🏁", blocks: [] };
    loadSingle(page);
    vi.spyOn(backend(), "getPage").mockResolvedValue(page);
    vi.spyOn(backend(), "getPageByPath").mockResolvedValue(page);
    seedFavorites(["Both"]);
    mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Both", pageKind: "page" });
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <><Sidebar /><PageView /></>, root);
    const shown = () => [logical(root.querySelector(".page-title-icon") ?? document.createElement("i")).trim(), ...labels(root, "#sidebar-favorites-list .nav-page-label")];
    try {
      await expect.poll(shown).toEqual(["🏁", "🏁 Both"]);
      setPageProperty("Both", "tine.icon", "AB");
      await expect.poll(shown).toEqual(["AB", "AB Both"]);
      setPageProperty("Both", "icon", "EF");
      await settle();
      expect(shown()).toEqual(["AB", "AB Both"]);
      setPageProperty("Both", "tine.icon", null);
      await expect.poll(shown).toEqual(["EF", "EF Both"]);
      setPageProperty("Both", "icon", null);
      await expect.poll(shown).toEqual(["", "⭐ Both"]);
      setPageProperty("Both", "tine.icon", "![](../assets/missing.png)");
      setPageProperty("Both", "icon", "EF");
      await settle();
      // A refused tine.icon image shows no icon; it does not fall back to icon:: (the property that is present wins).
      expect(root.querySelector(".page-title-icon")?.textContent ?? "").toBe("");
    } finally { dispose(); }
  });
  it("AAA's exact header fixture, live header typing, removal and external reload update Favorites and Recent", async () => {
    const page = (pre_block: string | null): PageDto => ({ name: "AAA", kind: "page", title: "AAA", pre_block,
      blocks: [{ id: "aaa-1", raw: "", collapsed: false, children: [] }, { id: "aaa-2", raw: "", collapsed: false, children: [] }] });
    loadSingle(page("icon:: 🚩\n\n"));
    vi.mocked(backend().pageIcons).mockResolvedValue({ AAA: "🚩" });
    seedFavorites(["AAA"]);
    setRecentPages([{ name: "AAA", kind: "page" }]);
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <Sidebar />, root);
    const both = () => [...labels(root, "#sidebar-favorites-list .nav-page-label"), ...labels(root, "#sidebar-recent-list .nav-page-label")];
    try {
      await expect.poll(both).toEqual(["🚩 AAA", "🚩 AAA"]);
      setPageProperty("AAA", "icon", null);
      await expect.poll(both).toEqual(["⭐ AAA", "AAA"]);
      setRaw("aaa-1", "icon:: 🚩");
      await expect.poll(both).toEqual(["🚩 AAA", "🚩 AAA"]);
      reloadPage(page("icon:: AB\n\n"));
      await expect.poll(both).toEqual(["AB AAA", "AB AAA"]);
    } finally { dispose(); }
  });

  it("published changes refresh cached icons for unloaded pages, including removal", async () => {
    vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(1);
    const icons = vi.mocked(backend().pageIcons).mockResolvedValue({ Published: "🚩" });
    seedFavorites(["Published"]);
    setRecentPages([{ name: "Published", kind: "page" }]);
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <Sidebar />, root);
    const both = () => [...labels(root, "#sidebar-favorites-list .nav-page-label"), ...labels(root, "#sidebar-recent-list .nav-page-label")];
    try {
      await expect.poll(both).toEqual(["🚩 Published", "🚩 Published"]);
      icons.mockResolvedValue({ Published: "AB" });
      bumpDataRev();
      await expect.poll(both).toEqual(["AB Published", "AB Published"]);
      icons.mockResolvedValue({});
      bumpDataRev();
      await expect.poll(both).toEqual(["⭐ Published", "Published"]);
    } finally { dispose(); }
  });
  it("Favorites: an iconed page shows its icon instead of the star; others keep the star", async () => {
    seedFavorites(["IconFav", "PlainFav"]);
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <Sidebar />, root);
    await expect.poll(() => labels(root, "#sidebar-favorites-list .nav-page-label")).toEqual(["🏁 IconFav", "⭐ PlainFav"]);
    dispose();
  });

  it("Recent: an iconed page shows its icon before its name; others show the bare name", async () => {
    setRecentPages([{ name: "IconRecent", kind: "page" }, { name: "PlainRecent", kind: "page" }]);
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <Sidebar />, root);
    await expect.poll(() => labels(root, "#sidebar-recent-list .nav-page-label")).toEqual(["🏁 IconRecent", "PlainRecent"]);
    dispose();
  });
});

describe("the All pages sidebar navigation", () => {
  it("no longer enumerates pages or their icons in the sidebar (GH #335)", async () => {
    vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(1);
    const existing = (n: string) => ({ kind: "existing" as const, id: `pages/${n}.md`, others: [] });
    vi.spyOn(backend(), "pageInventory").mockResolvedValue({ rev: "1", entries: [
      { key: "iconall", name: "IconAll", is_journal: false, day: null, target: existing("IconAll") },
      { key: "plainall", name: "PlainAll", is_journal: false, day: null, target: existing("PlainAll") },
    ] });
    installPageIndex();
    await refreshPageIndex();
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <Sidebar />, root);
    const header = root.querySelector<HTMLButtonElement>(".nav-all-pages-open")!;
    header.click();
    expect(root.textContent).not.toContain("IconAll");
    expect(root.textContent).not.toContain("PlainAll");
    dispose();
    resetPageIndex();
  });
});

describe("right sidebar page item carries the page icon", () => {
  const page = (name: string): PageDto => ({ name, kind: "page", title: name, pre_block: name.startsWith("Icon") ? "icon:: 🏁" : null, blocks: [{ id: `${name}-b`, raw: "text", collapsed: false, children: [] }] });

  it("shows the icon before the page name, and nothing extra for a page without one", async () => {
    loadSingle(page("IconSide"));
    loadSingle(page("PlainSide"));
    applySidebarSession({ right: true, items: [
      { kind: "page", name: "IconSide", pageKind: "page" },
      { kind: "page", name: "PlainSide", pageKind: "page" },
    ] });
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <RightSidebar />, root);
    await expect.poll(() => labels(root, ".rs-item-title")).toEqual(["🏁 IconSide", "PlainSide"]);
    dispose();
  });
});
