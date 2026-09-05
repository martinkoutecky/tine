import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { ContextMenu, deletePageMenuLabel, pageMenuAvailability } from "./ContextMenu";
import { initParser } from "../render/parse";
import {
  blockProperty,
  doc,
  extendSelectionTo,
  markDirty,
  pageByName,
  resetStore,
  selectBlock,
  selectedIds,
  setDoc,
  type Node as StoreNode,
} from "../store";
import {
  clearConflict,
  closeContextMenu,
  closeExportModal,
  exportModal,
  markConflict,
  openContextMenu,
  openPageContextMenu,
  setToasts,
  toasts,
} from "../ui";
import { clearTransientLayersForTest, dismissTopTransient } from "../transientLayers";
import { backend } from "../backend";
import { clearClipboardPayload, peekClipboardPayload } from "../clipboard";
import { mainPaneRouter, tabs } from "../router";
import { editingId, endEdit } from "../editorController";

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
    vi.restoreAllMocks();
    clearClipboardPayload();
    resetStore();
    closeContextMenu();
    closeExportModal();
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

  it("context Copy/Cut block each leave a fresh exact private payload", () => {
    load();
    setDoc("byId", "parent", "raw", "Parent\nid:: 11111111-1111-1111-1111-111111111111");
    setDoc("byId", "child", "raw", "Child\ncollapsed:: true\nid:: 22222222-2222-2222-2222-222222222222");
    vi.spyOn(backend(), "writeRich").mockResolvedValue();
    const dispose = mount(() => <ContextMenu />);
    const click = (label: string) => {
      const item = [...document.querySelectorAll<HTMLElement>(".ctx-item")]
        .find((el) => el.textContent?.trim() === label);
      expect(item).toBeDefined();
      item!.click();
    };

    openContextMenu(10, 10, "parent");
    click("Copy block");
    expect(peekClipboardPayload()).toMatchObject({
      op: "copy",
      blocks: [{
        raw: "Parent\nid:: 11111111-1111-1111-1111-111111111111",
        children: [{ raw: "Child\ncollapsed:: true\nid:: 22222222-2222-2222-2222-222222222222" }],
      }],
    });

    document.dispatchEvent(new Event("copy", { bubbles: true }));
    expect(peekClipboardPayload()).toBeNull();

    openContextMenu(10, 10, "parent");
    click("Cut block");
    expect(peekClipboardPayload()).toMatchObject({
      op: "cut",
      sourcePages: [{ name: "P", kind: "page", generation: expect.any(Number) }],
    });
    expect(peekClipboardPayload()?.blocks[0].children[0].raw).toContain("collapsed:: true");
    expect(doc.byId.parent).toBeUndefined();
    dispose();
  });

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
      "copy-export",
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
    expect(menu.querySelectorAll('[role="menuitem"]')).toHaveLength(13);
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
      "copy-page-ref", "copy-export", "copy-page-markdown", "export-pdf",
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
      "copy-page-ref", "copy-export", "copy-page-markdown", "export-pdf",
      "show-in-folder", "open-default-app", "page-properties",
      "carry-unfinished", "delete-journal",
    ]);
    dispose();
  });

  it("opens the shared export modal with the page root forest and preserves page Markdown/PDF actions", () => {
    load();
    const dispose = mount(() => <ContextMenu />);

    openPageContextMenu(10, 10, "P", "page", true);
    expect(menuLabels()).toContain("Copy page as Markdown");
    expect(menuLabels()).toContain("Export to PDF…");
    document.querySelector<HTMLButtonElement>('[data-page-action-id="copy-export"]')!.click();

    expect(exportModal()).toEqual({ ids: ["parent", "leaf"] });
    dispose();
  });

  it("does NOT offer it on a childless bullet (nothing to lay out)", () => {
    load();
    const dispose = mount(() => <ContextMenu />);
    openContextMenu(10, 10, "leaf");
    expect(menuLabels().some((l) => l.startsWith("Show children as"))).toBe(false);
    dispose();
  });

  it("offers Auto beside explicit heading levels and uses the shared transition", () => {
    load();
    const dispose = mount(() => <ContextMenu />);
    openContextMenu(10, 10, "leaf");

    const auto = document.querySelector<HTMLButtonElement>('[title="Automatic heading"]');
    expect(auto).not.toBeNull();
    auto!.click();
    expect(blockProperty("leaf", "heading")).toBe("true");
    dispose();
  });

  it.each([false, true])(
    "applies a heading to the active selection when the pointer block is %s read-only (GH #240)",
    (pointerReadOnly) => {
      setDoc({
        byId: {
          a: { ...node("a", "A", null, []), page: "Selected" },
          b: { ...node("b", "B", null, []), page: "Selected" },
          c: { ...node("c", "C", null, []), page: "Pointer" },
        },
        pages: [
          { name: "Selected", kind: "page", title: "Selected", preBlock: null, roots: ["a", "b"], format: "md", readOnly: false, guide: false },
          { name: "Pointer", kind: "page", title: "Pointer", preBlock: null, roots: ["c"], format: "md", readOnly: pointerReadOnly, guide: false },
        ],
        feed: ["Selected", "Pointer"],
        loaded: true,
      });
      selectBlock("a");
      extendSelectionTo("b");
      const dispose = mount(() => <ContextMenu />);

      openContextMenu(10, 10, "c");
      const h2 = document.querySelector<HTMLButtonElement>('[title="Heading 2"]');
      expect(h2).not.toBeNull();
      h2!.click();

      expect(doc.byId.a.raw).toBe("## A");
      expect(doc.byId.b.raw).toBe("## B");
      expect(doc.byId.c.raw).toBe("C");
      expect(selectedIds()).toEqual(["a", "b"]);
      dispose();
    },
  );

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

  it("offers 'Open in new tab' on an editable block after 'Zoom into block' and opens the block's page in a background tab", () => {
    load();
    setDoc("byId", "parent", "raw", "Parent\nid:: 11111111-1111-1111-1111-111111111111");
    vi.spyOn(backend(), "writeRich").mockResolvedValue();
    const dispose = mount(() => <ContextMenu />);
    openContextMenu(10, 10, "parent");
    const labels = menuLabels();
    const zoomIdx = labels.indexOf("Zoom into block");
    expect(zoomIdx).toBeGreaterThanOrEqual(0);
    const newTabIdx = labels.indexOf("Open in new tab");
    expect(newTabIdx).toBeGreaterThan(zoomIdx);

    const before = tabs().length;
    const item = [...document.querySelectorAll<HTMLElement>(".ctx-item")]
      .find((el) => el.textContent?.trim() === "Open in new tab")!;
    item.click();
    expect(tabs().length).toBe(before + 1);
    const newTab = tabs()[tabs().length - 1];
    expect(newTab.history[0]).toMatchObject({
      kind: "page",
      name: "P",
      pageKind: "page",
      block: "11111111-1111-1111-1111-111111111111",
    });
    dispose();
  });

  it("offers 'Open in new tab' on a read-only block too (matches middle-click parity)", () => {
    load(true);
    setDoc("byId", "parent", "raw", "Parent\nid:: 11111111-1111-1111-1111-111111111111");
    const dispose = mount(() => <ContextMenu />);
    openContextMenu(10, 10, "parent");
    const labels = menuLabels();
    expect(labels).toContain("Open in new tab");
    const zoomIdx = labels.indexOf("Zoom into block");
    const newTabIdx = labels.indexOf("Open in new tab");
    expect(newTabIdx).toBeGreaterThan(zoomIdx);
    dispose();
  });

  // "The file moves to the graph's .tine-trash folder" reads as fully
  // recoverable, and for a saved page it is. A page with unsaved edits — dirty,
  // or parked with an unresolved conflict, which by definition never reached
  // disk — trashes only its STALE file. The user's actual work is destroyed and
  // is in no trash. Say so before they answer. (Direct Files data-safety audit,
  // 2026-08-09, finding 18.)
  async function confirmTextForDelete(): Promise<string> {
    const confirm = vi.spyOn(backend(), "confirm").mockResolvedValue(false);
    const dispose = mount(() => <ContextMenu />);
    openPageContextMenu(10, 10, "P", "page");
    [...document.querySelectorAll<HTMLElement>(".ctx-item")]
      .find((el) => el.textContent?.trim() === "Delete page")!
      .click();
    await vi.waitFor(() => expect(confirm).toHaveBeenCalled());
    dispose();
    return confirm.mock.calls[0][0] as string;
  }

  it("warns that unsaved edits are not in the trash copy", async () => {
    load();
    markDirty("P");

    const text = await confirmTextForDelete();

    expect(text).toContain("unsaved");
    expect(text).toContain(".tine-trash");
    clearConflict("P");
  });

  it("warns for a conflicted page, whose edits provably never reached disk", async () => {
    load();
    markConflict("P");

    const text = await confirmTextForDelete();

    expect(text).toContain("unsaved");
    clearConflict("P");
  });

  it("keeps the plain wording for a page with nothing unsaved", async () => {
    load();

    const text = await confirmTextForDelete();

    expect(text).not.toContain("unsaved");
    expect(text).toContain(".tine-trash");
  });

  it("retires the current route in the durable-delete continuation before purging its page", async () => {
    load();
    mainPaneRouter.resetTabsToJournals();
    mainPaneRouter.openPage("P", "page", { inPlace: true });
    vi.spyOn(backend(), "confirm").mockResolvedValue(true);
    let resolveDelete!: () => void;
    vi.spyOn(backend(), "deletePage").mockImplementation(() => new Promise<void>((resolve) => {
      resolveDelete = resolve;
    }));
    const dispose = mount(() => <ContextMenu />);

    openPageContextMenu(10, 10, "P", "page");
    document.querySelector<HTMLElement>('[data-page-action-id="delete-page"]')!.click();
    await vi.waitFor(() => expect(backend().deletePage).toHaveBeenCalledTimes(1));
    expect(mainPaneRouter.route()).toMatchObject({ kind: "page", name: "P" });
    expect(pageByName("P")).toBeDefined();

    resolveDelete();
    // One continuation is the store's durable response. The menu's chained
    // `.then` is deliberately still pending here: on Android WebView the DOM can
    // paint between those two jobs, exposing an empty black current route.
    await Promise.resolve();

    expect(mainPaneRouter.route()).toEqual({ kind: "journals" });
    expect(pageByName("P")).toBeUndefined();
    dispose();
    mainPaneRouter.resetTabsToJournals();
  });
});

// GH #480. The keyboard route to "a block above this one" is Enter at offset 0,
// which splits. A code block owns its own Enter key (it inserts a newline and
// never splits), so when a code block is the FIRST block of a page there is
// neither a keyboard route nor an earlier block to insert after — the top of the
// page was simply unreachable. `blockActions` now carries the route.
describe("BlockMenu — insert a block above (GH #480)", () => {
  beforeAll(async () => {
    await initParser();
  });
  afterEach(() => {
    vi.restoreAllMocks();
    endEdit("page-navigation");
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

  const CODE = "```js\nconst a = 1;\n```";

  /** A page whose FIRST root is a code block — the reporter's shape exactly. */
  function loadCodeFirst() {
    setDoc({
      byId: {
        code: { id: "code", raw: CODE, collapsed: false, parent: null, page: "P", children: [] },
        after: { id: "after", raw: "plain text", collapsed: false, parent: null, page: "P", children: [] },
      },
      pages: [{ name: "P", kind: "page", title: "P", preBlock: null, roots: ["code", "after"], format: "md", readOnly: false, guide: false }],
      feed: ["P"],
      loaded: true,
    });
  }

  function clickItem(label: string) {
    const item = [...document.querySelectorAll<HTMLElement>(".ctx-item")]
      .find((el) => el.textContent?.trim() === label);
    expect(item, `no "${label}" item in the block menu`).toBeDefined();
    item!.click();
  }

  it("puts an empty block above the first block of a page and moves the caret into it", () => {
    loadCodeFirst();
    const dispose = mount(() => <ContextMenu />);
    openContextMenu(10, 10, "code");
    clickItem("Insert block above");

    const roots = pageByName("P")!.roots;
    expect(roots).toHaveLength(3);
    expect(roots.slice(1)).toEqual(["code", "after"]);
    const inserted = roots[0];
    expect(doc.byId[inserted].raw).toBe("");
    // The code block itself is untouched — this inserts, it does not split.
    expect(doc.byId.code.raw).toBe(CODE);
    expect(editingId()).toBe(inserted);
    dispose();
  });

  it("inserts before a nested block without leaving its parent", () => {
    setDoc({
      byId: {
        parent: { id: "parent", raw: "Parent", collapsed: false, parent: null, page: "P", children: ["kid"] },
        kid: { id: "kid", raw: CODE, collapsed: false, parent: "parent", page: "P", children: [] },
      },
      pages: [{ name: "P", kind: "page", title: "P", preBlock: null, roots: ["parent"], format: "md", readOnly: false, guide: false }],
      feed: ["P"],
      loaded: true,
    });
    const dispose = mount(() => <ContextMenu />);
    openContextMenu(10, 10, "kid");
    clickItem("Insert block above");

    const children = doc.byId.parent.children;
    expect(children).toHaveLength(2);
    expect(children[1]).toBe("kid");
    expect(doc.byId[children[0]].parent).toBe("parent");
    expect(pageByName("P")!.roots).toEqual(["parent"]);
    dispose();
  });

  it("does not offer it on a read-only page", () => {
    loadCodeFirst();
    setDoc("pages", 0, "readOnly", true);
    const dispose = mount(() => <ContextMenu />);
    openContextMenu(10, 10, "code");
    const labels = [...document.querySelectorAll(".ctx-item")].map((e) => e.textContent?.trim() ?? "");
    expect(labels).not.toContain("Insert block above");
    dispose();
  });
});

describe("page file actions on a conflicted page (GH #490)", () => {
  beforeAll(async () => {
    await initParser();
  });
  afterEach(() => {
    vi.restoreAllMocks();
    clearConflict("P");
    closeContextMenu();
    clearTransientLayersForTest();
    document.body.innerHTML = "";
  });

  function mount(node: () => JSX.Element): () => void {
    const root = document.createElement("div");
    document.body.appendChild(root);
    return render(node, root);
  }
  function loadPage() {
    resetStore();
    setDoc({
      byId: { only: { id: "only", raw: "Body", collapsed: false, parent: null, page: "P", children: [] } },
      pages: [{ name: "P", kind: "page", title: "P", preBlock: null, roots: ["only"], format: "md", readOnly: false, guide: false }],
      feed: ["P"],
      loaded: true,
    });
  }
  const clickItem = (label: string) => {
    const item = [...document.querySelectorAll<HTMLElement>(".ctx-item")]
      .find((e) => e.textContent?.trim() === label);
    if (!item) throw new Error(`menu item not found: ${label}`);
    item.click();
  };

  // A page whose conflict view will not load is the reporter's worst case: the
  // ONLY remaining way to see the text is to open the file outside Tine, and
  // Tine used to refuse exactly that. Opening the file changes nothing on disk,
  // so there was nothing for the refusal to protect.
  it("still opens the file on disk, and says the draft is not in it", async () => {
    loadPage();
    markConflict("P");
    const open = vi.spyOn(backend(), "openPageFile").mockResolvedValue(undefined as never);
    setToasts([]);
    const dispose = mount(() => <ContextMenu />);
    openPageContextMenu(10, 10, "P", "page", true);

    clickItem("Open with default app");
    await vi.waitFor(() => expect(open).toHaveBeenCalledTimes(1));
    expect(open.mock.calls[0][3]).toBe(false);
    await vi.waitFor(() =>
      expect(toasts().some((toast) => toast.kind === "info" && toast.message.includes("as it stands on disk"))).toBe(true),
    );
    expect(toasts().every((toast) => toast.kind !== "error")).toBe(true);
    dispose();
  });

  it("still reveals the file in the folder", async () => {
    loadPage();
    markConflict("P");
    const open = vi.spyOn(backend(), "openPageFile").mockResolvedValue(undefined as never);
    setToasts([]);
    const dispose = mount(() => <ContextMenu />);
    openPageContextMenu(10, 10, "P", "page", true);

    clickItem("Show in folder");
    await vi.waitFor(() => expect(open).toHaveBeenCalledTimes(1));
    expect(open.mock.calls[0][3]).toBe(true);
    expect(toasts().every((toast) => toast.kind !== "error")).toBe(true);
    dispose();
  });
});
