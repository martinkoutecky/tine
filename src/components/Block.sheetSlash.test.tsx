import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { applySheetViewSlashAction } from "./Block";
import { initParser } from "../render/parse";
import { blockProperty, doc, resetStore, setDoc, undo, type FeedPage, type Node } from "../store";
import { editingId, startEditing } from "../editorController";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  resetStore();
});

function page(roots: string[]): FeedPage {
  return {
    name: "Sheet",
    kind: "page",
    title: "Sheet",
    preBlock: null,
    roots,
    format: "md",
    readOnly: false,
  };
}

function node(id: string, raw: string, parent: string | null, children: string[] = []): Node {
  return { id, raw, collapsed: false, parent, page: "Sheet", children };
}

describe("sheet slash commands", () => {
  it("/Grid on a childless block writes the view, seeds one row/cell, and edits the new cell", () => {
    setDoc({
      byId: {
        host: node("host", "Host", null),
      },
      pages: [page(["host"])],
      feed: ["Sheet"],
      loaded: true,
    });
    startEditing("host", 4);

    const cellId = applySheetViewSlashAction("host", "grid");

    expect(blockProperty("host", "tine.view")).toBe("grid");
    expect(doc.byId.host.children).toHaveLength(1);
    const rowId = doc.byId.host.children[0];
    expect(doc.byId[rowId].raw).toBe("");
    expect(doc.byId[rowId].children).toEqual([cellId]);
    expect(cellId).not.toBeNull();
    expect(doc.byId[cellId!].raw).toBe("");
    expect(editingId()).toBe(cellId);

    undo();

    expect(doc.byId.host.raw).toBe("Host");
    expect(doc.byId.host.children).toEqual([]);
    expect(blockProperty("host", "tine.view")).toBeNull();
    expect(doc.byId[rowId]).toBeUndefined();
    expect(doc.byId[cellId!]).toBeUndefined();
    expect(editingId()).toBeNull();
  });

  it("/Grid on a block with children keeps the existing children as rows and seeds nothing", () => {
    setDoc({
      byId: {
        host: node("host", "Host", null, ["row"]),
        row: node("row", "Existing row", "host", ["cell"]),
        cell: node("cell", "Existing cell", "row"),
      },
      pages: [page(["host"])],
      feed: ["Sheet"],
      loaded: true,
    });
    startEditing("host", 4);

    const cellId = applySheetViewSlashAction("host", "grid");

    expect(cellId).toBeNull();
    expect(blockProperty("host", "tine.view")).toBe("grid");
    expect(doc.byId.host.children).toEqual(["row"]);
    expect(doc.byId.row.children).toEqual(["cell"]);
    expect(editingId()).toBeNull();

    undo();

    expect(doc.byId.host.raw).toBe("Host");
    expect(doc.byId.host.children).toEqual(["row"]);
    expect(blockProperty("host", "tine.view")).toBeNull();
    expect(doc.byId.row.raw).toBe("Existing row");
    expect(doc.byId.cell.raw).toBe("Existing cell");
  });
});
