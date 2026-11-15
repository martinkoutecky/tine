import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { ContextMenu } from "./ContextMenu";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { selectBlock, extendSelectionTo, resetStore, pageByName, undo, pasteClipboardPayload, selectedIds, setBlockProperty, blockProperty, loadFeed } from "../document";
import { clearClipboardPayload, peekClipboardPayload } from "../clipboard";
import { openContextMenu, closeContextMenu, closeExportModal, exportModal } from "../ui";
import { setGraphMeta } from "../graphSession";
import { clearTransientLayersForTest } from "../transientLayers";
import type { BlockDto } from "../types";
import { bindTestHost } from "../document/host/wiring.test.support";
import { removeAuthorTarget } from "../blockRefActions";

const A = "11111111-1111-4111-8111-111111111111";
const B = "22222222-2222-4222-8222-222222222222";
const child = "33333333-3333-4333-8333-333333333333";
const block = (id: string, raw: string, children: BlockDto[] = [], collapsed = false): BlockDto => ({ id, raw, children, collapsed });
let dispose: (() => void) | undefined;
beforeAll(() => initParser());
beforeEach(async () => {
  // Edits reach the page host once the window is bound (graph load).
  await bindTestHost();
  vi.spyOn(backend(), "writeRich").mockResolvedValue();
  vi.spyOn(backend(), "writeText").mockResolvedValue();
  vi.spyOn(backend(), "resolveBlocks").mockImplementation(async (ids) => ids.map(() => null));
});
afterEach(() => {
  dispose?.(); dispose = undefined;
  closeContextMenu(); closeExportModal(); clearClipboardPayload(); resetStore();
  setGraphMeta(null); clearTransientLayersForTest(); vi.restoreAllMocks();
  document.body.innerHTML = "";
});
function load(collapsed = false, readOnly = false) {
  loadSingle({ name: "P", kind: "page", title: "P", format: "md", pre_block: null, read_only: readOnly,
    blocks: [block(A, `One\nid:: ${A}`, [block(child, `Child\nid:: ${child}`)], collapsed), block(B, `Two\nid:: ${B}`), block("host", "")] });
  setGraphMeta({ root: "/fixture" } as any);
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <ContextMenu />, root);
  selectBlock(A); extendSelectionTo(B);
}
function open(id = A) { openContextMenu(10, 10, id); }
function click(stem: string) {
  const item = [...document.querySelectorAll<HTMLElement>(".ctx-item")].find(el => (el.textContent?.trim() === stem || el.textContent?.trim() === `${stem}s` || (stem === "Copy / export" && el.textContent?.trim() === "Copy / export as…")));
  expect(item, stem).toBeDefined(); item!.click();
}
function raws() { return pageByName("P")!.roots.map(id => doc.byId[id].raw.split("\n")[0]); }

describe("selection context menu (GH #591)", () => {
  it("Remove author is offered on a mixed read-only selection and skips the read-only page", () => {
    load();
    const page = (name: string, id: string, read_only: boolean) => ({ name, kind: "page" as const, title: name, format: "md" as const,
      pre_block: null, read_only, blocks: [{ ...block(id, "Words\ntine.author:: Ada Test"), properties: [["tine.author", "Ada Test"]] as [string, string][] }] });
    loadFeed([page("P", A, false), page("Q", B, true)]);
    selectBlock(A); extendSelectionTo(B); open(B); click("Remove author");
    expect(doc.byId[A].raw).toBe("Words");
    expect(doc.byId[B].raw).toBe("Words\ntine.author:: Ada Test");
    undo();
    expect(doc.byId[A].raw).toBe("Words\ntine.author:: Ada Test");
  });
  it("Remove author clears the selected forest including folded children, with one undo (GH #918)", () => {
    load(true);
    for (const id of [A, child]) setBlockProperty(id, "tine.author", "Synthetic author");
    const before = [A, child, B].map(id => doc.byId[id].raw);
    open(B); click("Remove author");
    for (const id of [A, child, B]) expect(blockProperty(id, "tine.author")).toBeNull();
    expect(doc.byId[B].raw).toBe(before[2]);
    undo();
    expect([A, child, B].map(id => doc.byId[id].raw)).toEqual(before);
  });
  it("one selected parent keeps the same Remove author target in the menu and palette (GH #941)", () => {
    load(); setBlockProperty(A, "tine.author", "Synthetic author"); selectBlock(A); open();
    expect(removeAuthorTarget()).toBe(A);
    click("Remove author");
    expect(blockProperty(A, "tine.author")).toBeNull();
    expect(doc.byId[A].children).toEqual([child]);
  });
  it("copies every selected block without duplicating a selected child and labels plural actions", async () => {
    load(); open(B);
    const labels = [...document.querySelectorAll(".ctx-item")].map(el => el.textContent?.trim());
    expect(labels).toEqual(expect.arrayContaining(["Copy blocks", "Cut blocks", "Copy block refs", "Copy block embeds", "Delete blocks"]));
    click("Copy block");
    await vi.waitFor(() => expect(backend().writeRich).toHaveBeenCalled());
    expect(vi.mocked(backend().writeRich).mock.calls[0][0]).toBe("- One\n\t- Child\n- Two");
    expect(peekClipboardPayload()!.blocks.map(b => b.raw.split("\n")[0])).toEqual(["One", "Two"]);
    expect(peekClipboardPayload()!.blocks[0].children).toHaveLength(1);
    expect(raws()).toEqual(["One", "Two", ""]);
  });
  it("cuts the complete selected forest, including hidden children, and pastes the same tree", async () => {
    load(true); open(); click("Cut block");
    await vi.waitFor(() => expect(raws()).toEqual([""]));
    expect(vi.mocked(backend().writeRich).mock.calls[0][0]).toBe("- One\n\t- Child\n- Two");
    await pasteClipboardPayload("host", peekClipboardPayload()!);
    expect(raws()).toEqual(["One", "Two"]);
    expect(doc.byId[A].children).toEqual([child]);
    expect(doc.byId[child].raw).toBe(`Child\nid:: ${child}`);
  });
  it.each(["Cut block", "Delete block"])("%s removes exactly the selection in one undo step", async (action) => {
    load(); open(); click(action);
    await vi.waitFor(() => expect(raws()).toEqual([""]));
    undo();
    expect(raws()).toEqual(["One", "Two", ""]);
    expect(doc.byId[A].children).toEqual([child]);
    undo();
    expect(raws()).toEqual(["One", "Two", ""]);
  });
  it.each(["Copy block ref", "Copy block embed"])("%s publishes all selected references after their IDs are saved", async (action) => {
    load(); open();
    // Top-level rows, as in OG's selection menu (GH #725), not behind a submenu.
    const row = [...document.querySelectorAll<HTMLElement>(".ctx-item")].find(el => el.textContent?.trim() === `${action}s`);
    expect(row).toBeDefined();
    expect(row!.closest(".ctx-submenu-menu")).toBeNull();
    click(action);
    await vi.waitFor(() => expect(backend().writeText).toHaveBeenCalledTimes(1));
    const text = vi.mocked(backend().writeText).mock.calls[0][0];
    for (const id of [A, child, B]) expect(text).toContain(action.endsWith("embed") ? `{{embed ((${id}))}}` : `((${id}))`);
  });
  for (const collapsed of [false, true]) {
    it.each(["Copy block ref", "Copy block embed"])(`%s on one selected parent includes its child, collapsed=${collapsed} (GH #941)`, async action => {
      load(collapsed); selectBlock(A); open(); click(action);
      await vi.waitFor(() => expect(backend().writeText).toHaveBeenCalledTimes(1));
      const expected = [A, child].map(id => action.endsWith("embed") ? `{{embed ((${id}))}}` : `- ((${id}))`).join("\n");
      expect(vi.mocked(backend().writeText).mock.calls[0][0]).toBe(expected);
    });
    it(`export of one selected parent carries its child, collapsed=${collapsed} (GH #941)`, () => {
      load(collapsed); selectBlock(A); open(); click("Copy / export");
      expect(exportModal()).toEqual({ ids: [A, child] });
    });
  }
  it("leaves every source intact when clipboard writing fails", async () => {
    load(); vi.mocked(backend().writeRich).mockRejectedValue(new Error("clipboard busy"));
    open(); click("Cut block");
    await vi.waitFor(() => expect(backend().writeRich).toHaveBeenCalled());
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(raws()).toEqual(["One", "Two", ""]);
  });
  it("does not delete a new selection while a cut waits on the clipboard", async () => {
    load(); let release!: () => void;
    vi.mocked(backend().writeRich).mockReturnValue(new Promise<void>(resolve => { release = resolve; }));
    open(); click("Cut block"); selectBlock(B); release();
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(raws()).toEqual(["One", "Two", ""]);
    expect(selectedIds()).toEqual([B]);
  });
  it("copies a read-only selection while omitting destructive actions", async () => {
    load(false, true); open();
    const labels = [...document.querySelectorAll(".ctx-item")].map(el => el.textContent?.trim());
    expect(labels).toContain("Copy blocks");
    expect(labels).not.toContain("Remove author");
    expect(labels).not.toContain("Cut blocks"); expect(labels).not.toContain("Delete blocks");
    click("Copy block");
    await vi.waitFor(() => expect(backend().writeRich).toHaveBeenCalled());
    expect(vi.mocked(backend().writeRich).mock.calls[0][0]).toBe("- One\n\t- Child\n- Two");
  });
  it("acts on a pointer outside the selection and keeps singular labels", async () => {
    load(); open("host"); click("Delete block");
    expect(raws()).toEqual(["One", "Two"]);
  });
  it("keeps copy/export selection-aware", () => {
    load(); open(B); click("Copy / export");
    expect(exportModal()).toEqual({ ids: [A, child, B] });
  });
  it("collapses every selected parent subtree", () => {
    load();
    loadSingle({ name: "P", kind: "page", title: "P", format: "md", pre_block: null,
      blocks: [block(A, "One", [block(child, "Child")]), block(B, "Two", [block("kid2", "Second child")])] });
    selectBlock(A); extendSelectionTo(B); open(); click("Collapse all");
    expect(doc.byId[A].collapsed).toBe(true);
    expect(doc.byId[B].collapsed).toBe(true);
  });
  it.each(["Expand all", "Numbered list"])("%s acts on every selected block", (action) => {
    load(action === "Expand all"); open(); click(action);
    for (const id of [A, child, B]) {
      if (action === "Numbered list") expect(blockProperty(id, "logseq.order-list-type")).toBe("number");
      else expect(doc.byId[id].collapsed).toBe(action === "Collapse all");
    }
  });
});
