import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { ContextMenu, deletePageMenuLabel, pageMenuAvailability } from "./ContextMenu";
import { initParser } from "../render/parse";
import { blockProperty, pageByName, resetStore } from "../document";
import { editingId, endEdit } from "../editorController";
import { type Node as StoreNode } from "../document/model";
import { doc, setDoc } from "../document/model";
import { closeContextMenu, closeExportModal, closePageProps, exportModal, openContextMenu, openPageContextMenu, pagePropsPanel } from "../ui";
import { clearTransientLayersForTest, dismissTopTransient } from "../transientLayers";
import { backend } from "../backend";
import { clearClipboardPayload, peekClipboardPayload } from "../clipboard";
import { setToasts, toasts } from "../toasts";
import { focusedRouter } from "../panes";
import { clearConflict, markConflict } from "../document/save/engine";

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
    setToasts([]);
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

  it("offers a durable new-tab destination for writable and read-only blocks", () => {
    for (const readOnly of [false, true]) {
      load(readOnly);
      const dispose = mount(() => <ContextMenu />);
      openContextMenu(10, 10, "leaf");
      expect(menuLabels()).toContain("Open in new tab");
      dispose();
      closeContextMenu();
      resetStore();
    }
  });

  // The long-left swipe opens this menu on the selected block as the analogue of
  // OG's mobile action bar (frontend/mobile/action_bar.cljs): Copy, Cut, Delete,
  // Copy ref and (iPad) Right sidebar. Card (SRS) and Copy url have no Tine
  // counterpart by decision (docs/BACKLOG.md: no flashcards; Tine registers no
  // URL scheme), so they are deliberately absent here.
  it("offers every OG action-bar action Tine can honour (Copy, Cut, Delete, Copy ref, Right sidebar)", () => {
    load();
    const dispose = mount(() => <ContextMenu />);
    openContextMenu(10, 10, "leaf");
    expect(menuLabels()).toEqual(
      expect.arrayContaining(["Copy block", "Cut block", "Delete block", "Copy block ref", "Open in sidebar"]),
    );
    dispose();
  });

  it("offers block Properties… only on a writable block, opening the block scope (GH #164)", () => {
    load(true);
    const dispose = mount(() => <ContextMenu />);
    openContextMenu(10, 10, "leaf");
    expect(menuLabels()).not.toContain("Properties…");
    closeContextMenu();
    resetStore();
    load();
    openContextMenu(30, 40, "leaf");
    [...document.querySelectorAll<HTMLElement>(".ctx-item")].find((el) => el.textContent?.trim() === "Properties…")!.click();
    expect(pagePropsPanel()).toMatchObject({ scope: { kind: "block", id: "leaf" }, x: 30, y: 40 });
    closePageProps();
    dispose();
  });

  it("refuses Make a template when its name inventory cannot be read", async () => {
    load();
    vi.spyOn(backend(), "listTemplates").mockRejectedValue(new Error("io:PermissionDenied"));
    const dispose = mount(() => <ContextMenu />);
    openContextMenu(10, 10, "leaf");
    [...document.querySelectorAll<HTMLElement>(".ctx-item")].find((item) => item.textContent?.includes("Make a template"))!.click();
    const input = document.querySelector<HTMLInputElement>(".ctx-template-name")!;
    input.value = "New template";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    document.querySelector<HTMLElement>(".ctx-template-submit")!.click();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(blockProperty("leaf", "template")).toBeNull();
    expect(toasts().some((t) => t.kind === "error")).toBe(true);
    dispose();
  });

  it("rechecks writability after reading names before Make a template", async () => {
    load();
    let finish!: (templates: []) => void;
    vi.spyOn(backend(), "listTemplates").mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const dispose = mount(() => <ContextMenu />);
    openContextMenu(10, 10, "leaf");
    [...document.querySelectorAll<HTMLElement>(".ctx-item")].find((item) => item.textContent?.includes("Make a template"))!.click();
    const input = document.querySelector<HTMLInputElement>(".ctx-template-name")!;
    input.value = "Read only";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    document.querySelector<HTMLElement>(".ctx-template-submit")!.click();
    setDoc("pages", 0, "readOnly", true);
    finish([]);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(blockProperty("leaf", "template")).toBeNull();
    expect(toasts().some((t) => t.kind === "success")).toBe(false);
    expect(toasts().some((t) => t.kind === "error")).toBe(true);
    dispose();
  });

  it("does not mark a colliding block in the new graph as a template", async () => {
    load();
    let finish!: (templates: Awaited<ReturnType<ReturnType<typeof backend>["listTemplates"]>>) => void;
    vi.spyOn(backend(), "listTemplates").mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const dispose = mount(() => <ContextMenu />);
    openContextMenu(10, 10, "leaf");
    const make = [...document.querySelectorAll<HTMLElement>(".ctx-item")].find((item) => item.textContent?.includes("Make a template"));
    expect(make).toBeDefined();
    make!.click();
    const input = document.querySelector<HTMLInputElement>(".ctx-template-name")!;
    input.value = "Old graph template";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    document.querySelector<HTMLElement>(".ctx-template-submit")!.click();
    resetStore();
    load();
    finish([]);
    await Promise.resolve();
    await Promise.resolve();
    expect(blockProperty("leaf", "template")).toBeNull();
    dispose();
  });

  it("does not delete a colliding page after an old graph confirmation", async () => {
    load();
    let finish!: (confirmed: boolean) => void;
    vi.spyOn(backend(), "confirm").mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const remove = vi.spyOn(backend(), "deletePage").mockResolvedValue(undefined as never);
    const dispose = mount(() => <ContextMenu />);
    openPageContextMenu(10, 10, "P", "page", true);
    const action = [...document.querySelectorAll<HTMLElement>(".ctx-item")]
      .find((item) => item.textContent?.includes("Delete page"));
    expect(action).toBeDefined();
    action!.click();
    resetStore();
    load();
    finish(true);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(remove).not.toHaveBeenCalled();
    dispose();
  });

  it.each([true, false])("pins the page file before Delete confirmation (explicit path %s)", async (explicit) => {
    load();
    setDoc("pages", 0, "id", "pages/one.md");
    let finish!: (confirmed: boolean) => void;
    vi.spyOn(backend(), "confirm").mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const remove = vi.spyOn(backend(), "deletePage").mockResolvedValue();
    const dispose = mount(() => <ContextMenu />);
    openPageContextMenu(10, 10, { name: "P", pageKind: "page", ...(explicit ? { path: "pages/one.md" } : {}) }, true);
    [...document.querySelectorAll<HTMLElement>(".ctx-item")].find((item) => item.textContent?.includes("Delete page"))!.click();
    // Same graph, new unique title claimant while native confirm is unanswered.
    setDoc("pages", 0, "id", "pages/two.md");
    finish(true);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(remove).not.toHaveBeenCalled();
    expect(pageByName("P")?.id).toBe("pages/two.md");
    dispose();
  });

  it("reopening a page menu replaces the previous file target", async () => {
    load(); setDoc("pages", 0, "id", "pages/one.md");
    const remove = vi.spyOn(backend(), "deletePage").mockResolvedValue();
    vi.spyOn(backend(), "confirm").mockResolvedValue(true);
    const dispose = mount(() => <ContextMenu />);
    openPageContextMenu(10, 10, { name: "P", pageKind: "page", path: "pages/one.md" }, true);
    setDoc("pages", 0, "id", "pages/two.md");
    openPageContextMenu(10, 10, { name: "P", pageKind: "page", path: "pages/two.md" }, true);
    const action = document.querySelector<HTMLElement>('[data-page-action-id="delete-page"]');
    expect(action).not.toBeNull();
    action!.click();
    await vi.waitFor(() => expect(remove).toHaveBeenCalledWith("P", "page", "pages/two.md"));
    dispose();
  });

  it("context Copy/Cut block each leave a fresh exact private payload", async () => {
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
    await vi.waitFor(() => expect(doc.byId.parent).toBeUndefined());
    expect(peekClipboardPayload()).toMatchObject({
      op: "cut",
      sourcePages: [{ name: "P", kind: "page", generation: expect.any(Number) }],
    });
    expect(peekClipboardPayload()?.blocks[0].children[0].raw).toContain("collapsed:: true");
    expect(doc.byId.parent).toBeUndefined();
    dispose();
  });

  it("keeps a block when the clipboard rejects Cut", async () => {
    load();
    vi.spyOn(backend(), "writeRich").mockRejectedValue(new Error("clipboard denied"));
    const dispose = mount(() => <ContextMenu />);
    openContextMenu(10, 10, "parent");
    const cut = [...document.querySelectorAll<HTMLElement>(".ctx-item")]
      .find((el) => el.textContent?.trim() === "Cut block");
    cut!.click();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(doc.byId.parent).toBeDefined();
    expect(peekClipboardPayload()).toBeNull();
    dispose();
  });

  it("keeps an edited block and its children when Cut resolves after the edit", async () => {
    load();
    let finish!: () => void;
    vi.spyOn(backend(), "writeRich").mockReturnValue(new Promise<void>((resolve) => { finish = resolve; }));
    const dispose = mount(() => <ContextMenu />);
    openContextMenu(10, 10, "parent");
    [...document.querySelectorAll<HTMLElement>(".ctx-item")]
      .find((el) => el.textContent?.trim() === "Cut block")!.click();
    setDoc("byId", "child", "raw", "Edited during clipboard write");
    finish();
    await vi.waitFor(() => expect(peekClipboardPayload()?.op).toBe("copy"));
    expect(doc.byId.parent).toBeDefined();
    expect(doc.byId.child.raw).toBe("Edited during clipboard write");
    dispose();
  });

  it("reports block Copy only after a successful clipboard write", async () => {
    load();
    let reject!: (error: Error) => void;
    vi.spyOn(backend(), "writeRich").mockReturnValue(new Promise<void>((_, fail) => { reject = fail; }));
    const dispose = mount(() => <ContextMenu />);
    openContextMenu(10, 10, "parent");
    [...document.querySelectorAll<HTMLElement>(".ctx-item")]
      .find((el) => el.textContent?.trim() === "Copy block")!.click();
    expect(toasts().some((toast) => toast.message === "Copied block")).toBe(false);
    reject(new Error("clipboard denied"));
    await vi.waitFor(() => expect(toasts().some((toast) => toast.kind === "error")).toBe(true));
    expect(toasts().some((toast) => toast.message === "Copied block")).toBe(false);
    dispose();
  });

  it("reports page Markdown Copy failure when the page read returns nothing", async () => {
    load();
    vi.spyOn(backend(), "getPage").mockResolvedValue(null);
    const write = vi.spyOn(backend(), "writeText");
    const dispose = mount(() => <ContextMenu />);
    openPageContextMenu(10, 10, "P", "page");
    document.querySelector<HTMLButtonElement>('[data-page-action-id="copy-page-markdown"]')!.click();
    await vi.waitFor(() => expect(toasts().some((toast) => toast.kind === "error")).toBe(true));
    expect(write).not.toHaveBeenCalled();
    expect(toasts().some((toast) => toast.message === "Copied page as Markdown")).toBe(false);
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

  // G3 finding 2 (og 12b follow-up): the rename's own refresh bumps the graph
  // epoch, which retired the owner the menu captured before the rename, so a
  // successful rename or merge from the menu neither opened the page nor said so.
  it.each(["renamed", "merged"] as const)("opens the page and confirms after a menu rename that %s", async (outcome) => {
    load();
    const dispose = mount(() => <ContextMenu />);
    openPageContextMenu(10, 10, "P", "page", true);
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    vi.spyOn(backend(), "resolvePage").mockImplementation(async (name) => name === "Q"
      ? (outcome === "merged" ? { kind: "existing", id: "pages/Q.md", others: [] } : { kind: "absent", id: "pages/Q.md" })
      : { kind: "existing", id: "pages/P.md", others: [] });
    vi.spyOn(backend(), "confirm").mockResolvedValue(true);
    const rename = vi.spyOn(backend(), "renamePage").mockResolvedValue({ outcome, touched: [{ path: "pages/P.md", moved: true }] });
    document.querySelector<HTMLButtonElement>('[data-page-action-id="rename-page"]')!.click();
    await Promise.resolve();
    const input = document.querySelector<HTMLInputElement>(".ctx-rename-name")!;
    input.value = "Q";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(toasts().map((toast) => toast.message)).toContain(outcome === "merged" ? "Merged into “Q”" : "Renamed to “Q”"));
    expect(rename).toHaveBeenCalledOnce();
    expect(focusedRouter().route()).toMatchObject({ kind: "page", name: "Q" });
    dispose();
  });

  it("keeps the route when the user moved on during a menu rename", async () => {
    load();
    const dispose = mount(() => <ContextMenu />);
    openPageContextMenu(10, 10, "P", "page", true);
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    vi.spyOn(backend(), "resolvePage").mockResolvedValue({ kind: "absent", id: "pages/Q.md" });
    let finish!: (value: { outcome: "renamed"; touched: [] }) => void;
    vi.spyOn(backend(), "renamePage").mockImplementation(() => new Promise((resolve) => { finish = resolve; }));
    document.querySelector<HTMLButtonElement>('[data-page-action-id="rename-page"]')!.click();
    await Promise.resolve();
    const input = document.querySelector<HTMLInputElement>(".ctx-rename-name")!;
    input.value = "Q";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(finish).toBeDefined());
    focusedRouter().openPage("Elsewhere", "page");
    finish({ outcome: "renamed", touched: [] });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(focusedRouter().route()).toMatchObject({ kind: "page", name: "Elsewhere" });
    expect(toasts().map((toast) => toast.message)).not.toContain("Renamed to “Q”");
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

// og I1d (port of master 6f8531344, GH #490 second half): opening or revealing
// a conflicted page's file changes nothing on disk and is the recovery path a
// stuck conflict needs, so it is not refused; the draft is not flushed.
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
      pages: [{ id: "pages/P.md", name: "P", kind: "page", title: "P", preBlock: null, roots: ["only"], format: "md", readOnly: false, guide: false }],
      feed: ["P"],
      loaded: true,
    } as never);
  }
  const clickItem = (label: string) => {
    const item = [...document.querySelectorAll<HTMLElement>(".ctx-item")]
      .find((e) => e.textContent?.trim() === label);
    if (!item) throw new Error(`menu item not found: ${label}`);
    item.click();
  };

  for (const [label, reveal] of [["Open with default app", false], ["Show in folder", true]] as const) {
    it(`${label}: opens the file as it stands on disk and says the draft is not in it`, async () => {
      loadPage();
      markConflict("P");
      const open = vi.spyOn(backend(), "openPageFile").mockResolvedValue(undefined as never);
      const save = vi.spyOn(backend(), "savePages");
      setToasts([]);
      const dispose = mount(() => <ContextMenu />);
      openPageContextMenu(10, 10, "P", "page", true);

      clickItem(label);
      await vi.waitFor(() => expect(open).toHaveBeenCalledTimes(1));
      expect(open.mock.calls[0][3]).toBe(reveal);
      expect(save).not.toHaveBeenCalled();
      await vi.waitFor(() =>
        expect(toasts().some((toast) => toast.kind === "info" && toast.message.includes("as it stands on disk"))).toBe(true),
      );
      expect(toasts().every((toast) => toast.kind !== "error")).toBe(true);
      dispose();
    });
  }
});
