import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { ContextMenu, deletePageMenuLabel, pageMenuAvailability } from "./ContextMenu";
import { initParser } from "../render/parse";
import { blockProperty, resetStore, setDoc, type Node as StoreNode } from "../store";
import { closeContextMenu, openContextMenu, openPageContextMenu } from "../ui";

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
