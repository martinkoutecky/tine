import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { ContextMenu, deletePageMenuLabel, pageMenuAvailability } from "./ContextMenu";
import { initParser } from "../render/parse";
import { blockProperty, resetStore, setDoc, type Node as StoreNode } from "../store";
import { closeContextMenu, openContextMenu, openPageContextMenu } from "../ui";
import { clearTransientLayersForTest, dismissTopTransient } from "../transientLayers";

describe("PageMenu page-kind availability", () => {
  it("keeps rename page-only but exposes delete for pages and journals", () => {
    expect(pageMenuAvailability("page")).toEqual({ rename: true, delete: true });
    expect(pageMenuAvailability("journal")).toEqual({ rename: false, delete: true });
  });

  it("labels the delete action by page kind", () => {
    expect(deletePageMenuLabel("page")).toBe("Delete page");
    expect(deletePageMenuLabel("journal")).toBe("Delete journal");
  });
});

describe("BlockMenu — convert an outline into a grid (Show children as →)", () => {
  beforeAll(async () => {
    await initParser();
  });
  afterEach(() => {
    resetStore();
    closeContextMenu();
    clearTransientLayersForTest();
    document.body.innerHTML = "";
  });

  function mount(node: () => JSX.Element): () => void {
    const root = document.createElement("div");
    document.body.appendChild(root);
    return render(node, root);
  }
  const node = (id: string, raw: string, parent: string | null, children: string[]): StoreNode => ({
    id, raw, collapsed: false, parent, page: "P", children,
  });
  function load(readOnly = false) {
    setDoc({
      byId: {
        parent: node("parent", "Parent", null, ["child"]),
        child: node("child", "Child", "parent", []),
        leaf: node("leaf", "Leaf", null, []),
      },
      pages: [{ name: "P", kind: "page", title: "P", preBlock: null, roots: ["parent", "leaf"], format: "md", readOnly, guide: false }],
      feed: ["P"],
      loaded: true,
    });
  }
  const menuLabels = () => [...document.querySelectorAll(".ctx-item")].map((e) => e.textContent?.trim() ?? "");

  it("offers 'Show children as →' on a bullet WITH children and flips tine.view to grid", () => {
    load();
    const dispose = mount(() => <ContextMenu />);
    openContextMenu(10, 10, "parent");
    expect(menuLabels().some((l) => l.startsWith("Show children as"))).toBe(true);

    const grid = [...document.querySelectorAll(".ctx-submenu-menu .ctx-item")].find((e) =>
      e.textContent?.includes("Grid")
    ) as HTMLElement | undefined;
    grid!.click();
    expect(blockProperty("parent", "tine.view")).toBe("grid");
    dispose();
  });

  it("offers exact-file actions only when invoked from a real page title", () => {
    load();
    const dispose = mount(() => <ContextMenu />);

    openPageContextMenu(10, 10, "P", "page");
    expect(menuLabels()).not.toContain("Show in folder");
    closeContextMenu();

    openPageContextMenu(10, 10, "P", "page", true);
    expect(menuLabels()).toContain("Show in folder");
    expect(menuLabels()).toContain("Open with default app");
    dispose();
  });

  it("exposes stable semantic page actions and focuses the first item", async () => {
    load();
    const trigger = document.createElement("button");
    trigger.dataset.pageActionsTrigger = "";
    document.body.appendChild(trigger);
    const dispose = mount(() => <ContextMenu />);

    openPageContextMenu(10, 10, "P", "page", true, trigger);
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    await Promise.resolve();

    const menu = document.querySelector<HTMLElement>('.ctx-menu[role="menu"]');
    expect(menu?.getAttribute("aria-label")).toBe("Page actions");
    const ids = [...document.querySelectorAll<HTMLElement>('[role="menuitem"][data-page-action-id]')]
      .map((item) => item.dataset.pageActionId);
    expect(ids).toEqual([
      "open",
      "open-sidebar",
      "open-new-tab",
      "favorite-toggle",
      "copy-page-ref",
      "copy-page-markdown",
      "export-pdf",
      "show-in-folder",
      "open-default-app",
      "page-properties",
      "rename-page",
      "delete-page",
    ]);
    expect(document.activeElement).toBe(document.querySelector('[data-page-action-id="open"]'));
    dispose();
  });

  it("wraps page-menu arrow navigation and honors Home and End", async () => {
    load();
    const dispose = mount(() => <ContextMenu />);
    openPageContextMenu(10, 10, "P", "page", true);
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    const menu = document.querySelector<HTMLElement>('.ctx-menu[role="menu"]')!;
    const activeId = () => (document.activeElement as HTMLElement | null)?.dataset.pageActionId;
    const press = (key: string) => document.activeElement?.dispatchEvent(
      new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }),
    );

    expect(activeId()).toBe("open");
    press("ArrowUp");
    expect(activeId()).toBe("delete-page");
    press("ArrowDown");
    expect(activeId()).toBe("open");
    press("End");
    expect(activeId()).toBe("delete-page");
    press("Home");
    expect(activeId()).toBe("open");
    expect(menu.querySelectorAll('[role="menuitem"]')).toHaveLength(12);
    dispose();
  });

  it("uses two Escape rungs for inline rename before restoring the ellipsis", async () => {
    load();
    const trigger = document.createElement("button");
    trigger.dataset.pageActionsTrigger = "";
    document.body.appendChild(trigger);
    const dispose = mount(() => <ContextMenu />);
    openPageContextMenu(10, 10, "P", "page", true, trigger);
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));

    document.querySelector<HTMLButtonElement>('[data-page-action-id="rename-page"]')!.click();
    await Promise.resolve();
    expect(document.activeElement).toBe(document.querySelector(".ctx-rename-name"));

    expect(dismissTopTransient("escape")).toBe(true);
    await Promise.resolve();
    expect(document.querySelector('.ctx-menu[role="menu"]')).not.toBeNull();
    expect(document.activeElement).toBe(document.querySelector('[data-page-action-id="rename-page"]'));

    expect(dismissTopTransient("escape")).toBe(true);
    await Promise.resolve();
    expect(document.querySelector('.ctx-menu[role="menu"]')).toBeNull();
    expect(document.activeElement).toBe(trigger);
    dispose();
  });

  it("restores the ellipsis after outside dismissal", async () => {
    load();
    const trigger = document.createElement("button");
    trigger.dataset.pageActionsTrigger = "";
    document.body.appendChild(trigger);
    const dispose = mount(() => <ContextMenu />);
    openPageContextMenu(10, 10, "P", "page", true, trigger);
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    document.querySelector<HTMLElement>(".ctx-overlay")!.click();
    await Promise.resolve();
    expect(document.querySelector('.ctx-menu[role="menu"]')).toBeNull();
    expect(document.activeElement).toBe(trigger);
    dispose();
  });

  it("preserves mutable/read-only page and journal action availability", async () => {
    load(true);
    const dispose = mount(() => <ContextMenu />);
    const ids = () => [...document.querySelectorAll<HTMLElement>("[data-page-action-id]")]
      .map((item) => item.dataset.pageActionId);

    openPageContextMenu(10, 10, "P", "page", true);
    expect(ids()).toEqual([
      "open", "open-sidebar", "open-new-tab", "favorite-toggle",
      "copy-page-ref", "copy-page-markdown", "export-pdf",
      "show-in-folder", "open-default-app",
    ]);
    closeContextMenu();

    setDoc("pages", 0, "readOnly", false);
    setDoc("pages", 0, "name", "2000-01-01");
    setDoc("pages", 0, "title", "2000-01-01");
    setDoc("pages", 0, "kind", "journal");
    openPageContextMenu(10, 10, "2000-01-01", "journal", true);
    expect(ids()).toEqual([
      "open", "open-sidebar", "open-new-tab", "favorite-toggle",
      "copy-page-ref", "copy-page-markdown", "export-pdf",
      "show-in-folder", "open-default-app", "page-properties",
      "carry-unfinished", "delete-journal",
    ]);
    dispose();
  });

  it("does NOT offer it on a childless bullet (nothing to lay out)", () => {
    load();
    const dispose = mount(() => <ContextMenu />);
    openContextMenu(10, 10, "leaf");
    expect(menuLabels().some((l) => l.startsWith("Show children as"))).toBe(false);
    dispose();
  });

  it("offers only view/copy actions on a read-only page", () => {
    load(true);
    const dispose = mount(() => <ContextMenu />);
    openContextMenu(10, 10, "parent");
    const labels = menuLabels();
    expect(labels).toContain("Zoom into block");
    expect(labels).toContain("Copy block");
    expect(labels).not.toContain("Delete block");
    expect(labels).not.toContain("Collapse all");
    expect(labels).not.toContain("Numbered list");
    expect(document.querySelector(".ctx-headings")).toBeNull();
    dispose();
  });
});
