import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { editingId, endEdit } from "../editorController";
import { initParser } from "../render/parse";
import { doc, loadSingle, resetStore } from "../store";
import type { PageDto } from "../types";
import { applySidebarSession, rightSidebar, setRightSidebar } from "../ui";
import { RightSidebar } from "./RightSidebar";

const page: PageDto = {
  name: "Sidebar test",
  kind: "page",
  title: "Sidebar test",
  pre_block: null,
  blocks: [
    {
      id: "sidebar-root",
      raw: "Editable sidebar text",
      collapsed: false,
      children: [{ id: "sidebar-child", raw: "Sidebar child", collapsed: false, children: [] }],
    },
    { id: "sidebar-second", raw: "Second block", collapsed: false, children: [] },
  ],
};

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  endEdit("page-navigation");
  setRightSidebar([]);
  applySidebarSession({ right: false, items: [] });
  resetStore();
  document.body.innerHTML = "";
  localStorage.clear();
  vi.restoreAllMocks();
});

function mount(items = [
  { kind: "page" as const, name: page.name, pageKind: "page" as const },
  { kind: "block" as const, uuid: "sidebar-second", page: page.name, pageKind: "page" as const },
]) {
  loadSingle(page);
  applySidebarSession({ right: true, items });
  vi.spyOn(backend(), "getBacklinks").mockResolvedValue([]);
  vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([]);
  vi.spyOn(backend(), "getBlockRefCounts").mockResolvedValue({});
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => <RightSidebar />, root);
  return { root, dispose };
}

describe("right sidebar collection disclosures", () => {
  it("adopts the canonical page name for a restored mixed-case sidebar item", async () => {
    const canonical = { ...page, name: "page1", title: "page1" };
    applySidebarSession({
      right: true,
      items: [{ kind: "page", name: "Page1", pageKind: "page" }],
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue(canonical);
    vi.spyOn(backend(), "getBacklinks").mockResolvedValue([]);
    vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([]);
    vi.spyOn(backend(), "getBlockRefCounts").mockResolvedValue({});
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <RightSidebar />, root);

    try {
      await vi.waitFor(() => {
        expect(rightSidebar()[0]).toMatchObject({ kind: "page", name: "page1" });
        expect(root.textContent).toContain("Editable sidebar text");
      });
    } finally {
      dispose();
    }
  });

  it("keeps a real sidebar Block disclosure separate from the sidebar item's disclosure", async () => {
    const { root, dispose } = mount([
      { kind: "page", name: page.name, pageKind: "page" },
    ]);
    try {
      const parentToggle = await vi.waitFor(() => {
        const found = root.querySelector<HTMLElement>(
          '[data-block-id="sidebar-root"] > .block-main .collapse-toggle.has-children'
        );
        expect(found).not.toBeNull();
        expect(root.querySelector('[data-block-id="sidebar-child"] > .block-main .collapse-toggle:not(.has-children)')).not.toBeNull();
        expect(root.querySelectorAll('[data-block-id="sidebar-root"] > .block-main .collapse-toggle.has-children')).toHaveLength(1);
        return found!;
      });
      const itemToggle = root.querySelector<HTMLButtonElement>("[data-right-sidebar-item-toggle]")!;
      expect(itemToggle.getAttribute("aria-expanded")).toBe("true");
      expect(root.querySelector(".rs-item-body")).not.toBeNull();

      parentToggle.click();
      await vi.waitFor(() => expect(root.querySelector('[data-block-id="sidebar-child"]')).toBeNull());
      expect(root.querySelector(".rs-item-body")).not.toBeNull();
      expect(itemToggle.getAttribute("aria-expanded")).toBe("true");

      root.querySelector<HTMLElement>('[data-block-id="sidebar-root"] > .block-main .collapse-toggle.has-children')!.click();
      await vi.waitFor(() => expect(root.querySelector('[data-block-id="sidebar-child"]')).not.toBeNull());
    } finally {
      dispose();
    }
  });

  it("collapses items independently and offers collapse, expand, and close all", async () => {
    const { root, dispose } = mount();
    try {
      const toggles = await vi.waitFor(() => {
        const found = root.querySelectorAll<HTMLButtonElement>("[data-right-sidebar-item-toggle]");
        expect(found).toHaveLength(2);
        return [...found];
      });
      expect(toggles[0].getAttribute("aria-expanded")).toBe("true");
      expect(toggles[0].getAttribute("aria-controls")).toBeTruthy();
      expect(root.querySelectorAll(".rs-item-body")).toHaveLength(2);

      toggles[0].focus();
      toggles[0].click();
      await Promise.resolve();
      expect(root.querySelector("[data-right-sidebar-item-toggle]")?.getAttribute("aria-expanded")).toBe("false");
      expect(document.activeElement).toBe(root.querySelector("[data-right-sidebar-item-toggle]"));
      expect(root.querySelectorAll(".rs-item-body")).toHaveLength(1);

      root.querySelector<HTMLButtonElement>("[data-right-sidebar-actions]")!.click();
      root.querySelector<HTMLButtonElement>('[data-right-sidebar-action="collapse-all"]')!.click();
      expect(root.querySelectorAll(".rs-item-body")).toHaveLength(0);
      expect(rightSidebar().every((item) => item.collapsed)).toBe(true);

      root.querySelector<HTMLButtonElement>("[data-right-sidebar-actions]")!.click();
      root.querySelector<HTMLButtonElement>('[data-right-sidebar-action="expand-all"]')!.click();
      expect(root.querySelectorAll(".rs-item-body")).toHaveLength(2);
      expect(rightSidebar().every((item) => !item.collapsed)).toBe(true);

      root.querySelector<HTMLButtonElement>("[data-right-sidebar-actions]")!.click();
      root.querySelector<HTMLButtonElement>('[data-right-sidebar-action="close-all"]')!.click();
      expect(rightSidebar()).toEqual([]);
      expect(root.textContent).toContain("Nothing open");
    } finally {
      dispose();
    }
  });

  it("commits and exits an active sidebar editor before its body unmounts", async () => {
    const { root, dispose } = mount([
      { kind: "block", uuid: "sidebar-root", page: page.name, pageKind: "page" },
    ]);
    try {
      const content = await vi.waitFor(() => {
        const found = root.querySelector<HTMLElement>(".rs-item-body .block-content-wrapper");
        expect(found).not.toBeNull();
        return found!;
      });
      content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
      content.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
      content.click();
      const editor = await vi.waitFor(() => {
        const found = root.querySelector<HTMLTextAreaElement>(".rs-item-body textarea.block-editor");
        expect(found).not.toBeNull();
        return found!;
      });
      editor.value = "Committed before collapse";
      editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText" }));
      expect(editingId()).toBe("sidebar-root");

      root.querySelector<HTMLButtonElement>("[data-right-sidebar-item-toggle]")!.click();

      expect(root.querySelector(".rs-item-body")).toBeNull();
      expect(editingId()).toBeNull();
      expect(doc.byId["sidebar-root"].raw).toBe("Committed before collapse");
    } finally {
      dispose();
    }
  });
});
