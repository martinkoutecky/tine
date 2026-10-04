import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { ContextMenu } from "./ContextMenu";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { selectBlock, extendSelectionTo, resetStore, pageByName, undo, pasteClipboardPayload, selectedIds, blockProperty } from "../document";
import { clearClipboardSlot, peekClipboardSlot } from "../clipboard";
import { openContextMenu, closeContextMenu, closeExportModal, exportModal } from "../ui";
import { setGraphMeta } from "../graphSession";
import { clearTransientLayersForTest } from "../transientLayers";
import type { BlockDto } from "../types";

const A = "11111111-1111-4111-8111-111111111111";
const B = "22222222-2222-4222-8222-222222222222";
const child = "33333333-3333-4333-8333-333333333333";
const block = (id: string, raw: string, children: BlockDto[] = [], collapsed = false): BlockDto => ({ id, raw, children, collapsed });
let dispose: (() => void) | undefined;
beforeAll(() => initParser());
beforeEach(() => {
  vi.spyOn(backend(), "writeRich").mockResolvedValue();
  vi.spyOn(backend(), "writeText").mockResolvedValue();
  vi.spyOn(backend(), "savePages").mockResolvedValue({ ok: ["revision"] });
  vi.spyOn(backend(), "resolveBlocks").mockImplementation(async (ids) => ids.map(() => null));
});
afterEach(() => {
  dispose?.(); dispose = undefined;
  closeContextMenu(); closeExportModal(); clearClipboardSlot(); resetStore();
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
  it("copies every selected block without duplicating a selected child and labels plural actions", async () => {
    load(); open(B);
    const labels = [...document.querySelectorAll(".ctx-item")].map(el => el.textContent?.trim());
    expect(labels).toEqual(expect.arrayContaining(["Copy blocks", "Cut blocks", "Copy block refs", "Copy block embeds", "Delete blocks"]));
    click("Copy block");
    await vi.waitFor(() => expect(backend().writeRich).toHaveBeenCalled());
    expect(vi.mocked(backend().writeRich).mock.calls[0][0]).toBe("- One\n\t- Child\n- Two");
    expect(peekClipboardSlot()!.blocks.map(b => b.raw.split("\n")[0])).toEqual(["One", "Two"]);
    expect(peekClipboardSlot()!.blocks[0].children).toHaveLength(1);
    expect(raws()).toEqual(["One", "Two", ""]);
  });
  it("cuts the complete selected forest, including hidden children, and pastes the same tree", async () => {
    load(true); open(); click("Cut block");
    await vi.waitFor(() => expect(raws()).toEqual([""]));
    expect(vi.mocked(backend().writeRich).mock.calls[0][0]).toBe("- One\n\t- Child\n- Two");
    await pasteClipboardPayload("host", peekClipboardSlot()!);
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
    load(); open(); click(action);
    await vi.waitFor(() => expect(backend().writeText).toHaveBeenCalledTimes(1));
    const text = vi.mocked(backend().writeText).mock.calls[0][0];
    for (const id of [A, child, B]) expect(text).toContain(action.endsWith("embed") ? `{{embed ((${id}))}}` : `((${id}))`);
  });
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
