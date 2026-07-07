import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { initParser } from "../render/parse";
import { doc, resetStore, setDoc, type FeedPage, type Node } from "../store";
import { setWorkflow } from "../ui";
import { cycleField, fieldIdsForBlocks, readField, writeField } from "./fields";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  resetStore();
  setWorkflow("todo");
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

function node(id: string, raw: string, parent: string | null = null, children: string[] = []): Node {
  return { id, raw, collapsed: false, parent, page: "Sheet", children };
}

function loadRows() {
  setDoc({
    byId: {
      a: node("a", "TODO [#A] Ship #sheets\nSCHEDULED: <2026-07-08 Wed>\nowner:: Martin"),
      b: node("b", "DONE Verify\nDEADLINE: <2026-07-09 Thu>\nestimate:: 2h"),
      c: node("c", "Plain\nowner:: Codex"),
    },
    pages: [page(["a", "b", "c"])],
    feed: ["Sheet"],
    loaded: true,
  });
}

describe("sheet fields", () => {
  it("discovers observed fields in stable facet order", () => {
    loadRows();

    expect(fieldIdsForBlocks(["a", "b", "c"], { includePage: true })).toEqual([
      "state",
      "priority",
      "scheduled",
      "deadline",
      "tags",
      "prop:owner",
      "prop:estimate",
      "page",
    ]);
  });

  it("reads field values through facets", () => {
    loadRows();

    expect(readField("a", "state")).toEqual({ text: "TODO", raw: "TODO" });
    expect(readField("a", "priority")).toEqual({ text: "[#A]", raw: "A" });
    expect(readField("a", "tags")).toEqual({ text: "#sheets", raw: "sheets" });
    expect(readField("a", "prop:owner")).toEqual({ text: "Martin", raw: "Martin" });
  });

  it("writes state through marker machinery and cycles with the configured workflow", () => {
    loadRows();

    expect(writeField("a", "state", "DOING")).toBe(true);
    expect(doc.byId.a.raw.split("\n")[0]).toBe("DOING [#A] Ship #sheets");

    expect(cycleField("a", "state")).toBe(true);
    expect(doc.byId.a.raw.split("\n")[0]).toBe("DONE [#A] Ship #sheets");
  });

  it("writes and removes properties in the canonical position after the first line", () => {
    setDoc({
      byId: {
        a: node("a", "Title\nbody line\nother:: keep"),
      },
      pages: [page(["a"])],
      feed: ["Sheet"],
      loaded: true,
    });

    expect(writeField("a", "prop:owner", "Martin")).toBe(true);
    // The new key lands in the canonical head; the legacy trailing property is
    // NOT hoisted (setBlockProperty never reorders lines it wasn't asked to
    // touch — hoisting body-region lines is a fence hazard).
    expect(doc.byId.a.raw).toBe("Title\nowner:: Martin\nbody line\nother:: keep");

    expect(writeField("a", "prop:owner", "")).toBe(true);
    expect(doc.byId.a.raw).toBe("Title\nbody line\nother:: keep");
  });
});
