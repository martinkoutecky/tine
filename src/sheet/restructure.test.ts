import { beforeAll, beforeEach, describe, expect, it } from "vitest";
import { initParser } from "../render/parse";
import { doc, pageToDto, resetStore, setDoc, undo, type FeedPage, type Node as StoreNode } from "../store";
import { flatten, hierarchify } from "./restructure";

beforeAll(() => initParser());

beforeEach(() => {
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
    guide: false,
  };
}

function node(id: string, raw: string, parent: string | null, children: string[] = []): StoreNode {
  return { id, raw, collapsed: false, parent, page: "Sheet", children };
}

function loadTable() {
  setDoc({
    byId: {
      table: node("table", "Table\ntine.view:: table", null, ["r1", "r2", "r3"]),
      r1: node("r1", "TODO [#A] First\nowner:: Martin", "table"),
      r2: node("r2", "TODO Second\nowner:: Codex", "table"),
      r3: node("r3", "DONE Third\nowner:: Codex", "table"),
    },
    pages: [page(["table"])],
    feed: ["Sheet"],
    loaded: true,
  });
}

describe("sheet restructure", () => {
  it("hierarchifies by state, flattens back to identity, and each direction undoes as one unit", () => {
    loadTable();
    const before = pageToDto("Sheet");

    expect(hierarchify("table", "state")).toBe(true);
    const groups = doc.byId.table.children;
    expect(groups).toHaveLength(2);
    expect(groups.map((id) => doc.byId[id].raw)).toEqual(["TODO", "DONE"]);
    expect(doc.byId[groups[0]].children).toEqual(["r1", "r2"]);
    expect(doc.byId[groups[1]].children).toEqual(["r3"]);

    undo();
    expect(pageToDto("Sheet")).toEqual(before);

    expect(hierarchify("table", "state")).toBe(true);
    const groupedAgain = pageToDto("Sheet");
    expect(flatten("table")).toBe(true);
    expect(pageToDto("Sheet")).toEqual(before);

    undo();
    expect(pageToDto("Sheet")).toEqual(groupedAgain);
  });

  it("flatten writes a round-tripping group property to rows that lack it", () => {
    setDoc({
      byId: {
        table: node("table", "Table\ntine.view:: table\ntine.group-by:: prop:owner", null, ["g1", "loose"]),
        g1: node("g1", "Martin", "table", ["r1", "r2"]),
        r1: node("r1", "Needs owner", "g1"),
        r2: node("r2", "Already owned\nowner:: Martin", "g1"),
        loose: node("loose", "Loose row", "table"),
      },
      pages: [page(["table"])],
      feed: ["Sheet"],
      loaded: true,
    });
    const before = pageToDto("Sheet");

    expect(flatten("table")).toBe(true);

    expect(doc.byId.table.children).toEqual(["r1", "r2", "loose"]);
    expect(doc.byId.g1).toBeUndefined();
    expect(doc.byId.r1.raw).toBe("Needs owner\nowner:: Martin");
    expect(doc.byId.r2.raw).toBe("Already owned\nowner:: Martin");

    undo();
    expect(pageToDto("Sheet")).toEqual(before);
  });

  it("no-ops cleanly when there is nothing to restructure", () => {
    loadTable();
    const before = pageToDto("Sheet");

    expect(flatten("table")).toBe(false);
    expect(hierarchify("missing", "state")).toBe(false);

    expect(pageToDto("Sheet")).toEqual(before);
  });
});
