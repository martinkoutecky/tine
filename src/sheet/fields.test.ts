import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { initParser } from "../render/parse";
import { doc, resetStore, setDoc, undo, type FeedPage, type Node } from "../store";
import { setWorkflow } from "../ui";
import { cycleField, fieldIdsForBlocks, fieldLabel, groupKeysForBlock, isFieldId, readField, writeField, writeTagDelta } from "./fields";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  resetStore();
  setWorkflow("todo");
});

function page(roots: string[], opts: Partial<Pick<FeedPage, "format" | "readOnly">> = {}): FeedPage {
  return {
    name: "Sheet",
    kind: "page",
    title: "Sheet",
    preBlock: null,
    roots,
    format: opts.format ?? "md",
    readOnly: opts.readOnly ?? false,
    guide: false,
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

  it("keeps field (column) order stable when a cell value is edited (GH #216)", () => {
    setDoc({
      byId: { r: node("r", "row\nfirst:: 23\nsecond:: 46\nthird:: 69") },
      pages: [page(["r"])],
      feed: ["Sheet"],
      loaded: true,
    });
    const before = fieldIdsForBlocks(["r"]);
    expect(before).toEqual(["prop:first", "prop:second", "prop:third"]);
    // Editing the middle cell must not reorder the columns.
    expect(writeField("r", "prop:second", "90")).toBe(true);
    expect(readField("r", "prop:second")).toEqual({ text: "90", raw: "90" });
    expect(fieldIdsForBlocks(["r"])).toEqual(before);
  });

  it("reads field values through facets", () => {
    loadRows();

    expect(readField("a", "state")).toEqual({ text: "TODO", raw: "TODO" });
    expect(readField("a", "priority")).toEqual({ text: "[#A]", raw: "A" });
    expect(readField("a", "tags")).toEqual({ text: "#sheets", raw: "sheets" });
    expect(readField("a", "prop:owner")).toEqual({ text: "Martin", raw: "Martin" });
  });

  it("treats formula fields as read-only derived fields", () => {
    loadRows();

    expect(isFieldId("formula:total")).toBe(true);
    expect(fieldLabel("formula:total")).toBe("total");
    expect(readField("a", "formula:total")).toBeNull();
    expect(writeField("a", "formula:total", "12")).toBe(false);
    expect(doc.byId.a.raw).toContain("owner:: Martin");
  });

  it("groups formula fields into boolean, none, and error buckets", () => {
    setDoc({
      byId: {
        a: node("a", "Big\npoints:: 3"),
        b: node("b", "Small\npoints:: 1"),
        c: node("c", "Missing"),
      },
      pages: [page(["a", "b", "c"])],
      feed: ["Sheet"],
      loaded: true,
    });
    const formulas = new Map([
      ["big", "points > 2"],
      ["maybe", "if(points > 2, null, points)"],
      ["broken", "points + true"],
    ]);
    const now = new Date(Date.UTC(2026, 6, 7));

    expect(groupKeysForBlock("a", "formula:big", { formulas, now })).toEqual(["true"]);
    expect(groupKeysForBlock("b", "formula:big", { formulas, now })).toEqual(["false"]);
    expect(groupKeysForBlock("a", "formula:maybe", { formulas, now })).toEqual([null]);
    expect(groupKeysForBlock("c", "formula:broken", { formulas, now })).toEqual(["(error)"]);
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

describe("writeField review fixes", () => {
  it("state cycle writes exactly ONE clock transition (no double timetracking)", () => {
    setDoc({
      byId: { a: node("a", "TODO write the intro") },
      pages: [page(["a"])],
      feed: ["Sheet"],
      loaded: true,
    });
    expect(writeField("a", "state", "DOING")).toBe(true);
    const raw = doc.byId.a.raw;
    expect(raw.startsWith("DOING ")).toBe(true);
    const clocks = (raw.match(/CLOCK:/g) || []).length;
    // cycleMarkerSmart bakes the transition in; setRaw must not add a second.
    expect(clocks).toBeLessThanOrEqual(1);
    expect((raw.match(/:LOGBOOK:/g) || []).length).toBeLessThanOrEqual(1);
  });

  it("refuses to write on a read-only page (org round-trip gate)", () => {
    setDoc({
      byId: { a: node("a", "TODO t") },
      pages: [{ ...page(["a"]), readOnly: true }],
      feed: ["Sheet"],
      loaded: true,
    });
    const before = doc.byId.a.raw;
    expect(writeField("a", "state", "DOING")).toBe(false);
    expect(writeField("a", "prop:x", "1")).toBe(false);
    expect(doc.byId.a.raw).toBe(before);
  });
});

describe("writeTagDelta", () => {
  function loadOne(raw: string, opts: Partial<Pick<FeedPage, "format" | "readOnly">> = {}) {
    setDoc({
      byId: { a: node("a", raw) },
      pages: [page(["a"], opts)],
      feed: ["Sheet"],
      loaded: true,
    });
  }

  it("adds simple and multi-word tags at the end of the first line", () => {
    loadOne("Title");
    expect(writeTagDelta("a", { add: "alpha" })).toBe(true);
    expect(doc.byId.a.raw).toBe("Title #alpha");

    loadOne("Title\nbody");
    expect(writeTagDelta("a", { add: "multi word" })).toBe(true);
    expect(doc.byId.a.raw).toBe("Title #[[multi word]]\nbody");
  });

  it("does not rewrite when the tag already exists case-insensitively", () => {
    loadOne("Title #Alpha");
    expect(writeTagDelta("a", { add: "alpha" })).toBe(true);
    expect(doc.byId.a.raw).toBe("Title #Alpha");
  });

  it("appends with exactly one separator after trimming first-line trailing spaces", () => {
    loadOne("Title   \nbody");
    expect(writeTagDelta("a", { add: "alpha" })).toBe(true);
    expect(doc.byId.a.raw).toBe("Title #alpha\nbody");
  });

  it("removes first-line tags at the start, middle, and end with whitespace normalization", () => {
    loadOne("#alpha Start");
    expect(writeTagDelta("a", { remove: "alpha" })).toBe(true);
    expect(doc.byId.a.raw).toBe("Start");

    loadOne("Start  #alpha  end");
    expect(writeTagDelta("a", { remove: "alpha" })).toBe(true);
    expect(doc.byId.a.raw).toBe("Start end");

    loadOne("Start #alpha  ");
    expect(writeTagDelta("a", { remove: "alpha" })).toBe(true);
    expect(doc.byId.a.raw).toBe("Start");
  });

  it("removes every first-line occurrence of a duplicated tag", () => {
    // review finding: a single cut returned true with the tag still present
    loadOne("#a foo #a bar");
    expect(writeTagDelta("a", { remove: "a" })).toBe(true);
    expect(doc.byId.a.raw).toBe("foo bar");

    loadOne("x #b #[[b]] y");
    expect(writeTagDelta("a", { remove: "b" })).toBe(true);
    expect(doc.byId.a.raw).toBe("x y");
  });

  it("refuses to remove a tag that exists only below the first line", () => {
    loadOne("Title\nBody #alpha");
    expect(writeTagDelta("a", { remove: "alpha" })).toBe(false);
    expect(doc.byId.a.raw).toBe("Title\nBody #alpha");
  });

  it("leaves a code-span lookalike untouched", () => {
    loadOne("Title `#alpha`");
    expect(writeTagDelta("a", { remove: "alpha" })).toBe(false);
    expect(doc.byId.a.raw).toBe("Title `#alpha`");
  });

  it("refuses org-format write-back", () => {
    loadOne("Title :alpha:", { format: "org" });
    expect(writeTagDelta("a", { remove: "alpha" })).toBe(false);
    expect(writeTagDelta("a", { add: "beta" })).toBe(false);
    expect(doc.byId.a.raw).toBe("Title :alpha:");
  });

  it("moves by removing then adding as one undoable gesture", () => {
    loadOne("Read #old");
    const before = doc.byId.a.raw;

    expect(writeTagDelta("a", { remove: "old", add: "new" })).toBe(true);
    expect(doc.byId.a.raw).toBe("Read #new");

    undo();
    expect(doc.byId.a.raw).toBe(before);
  });

  it("removes both bare and bracketed source forms", () => {
    loadOne("Read #tag");
    expect(writeTagDelta("a", { remove: "tag" })).toBe(true);
    expect(doc.byId.a.raw).toBe("Read");

    loadOne("Read #[[tag]]");
    expect(writeTagDelta("a", { remove: "tag" })).toBe(true);
    expect(doc.byId.a.raw).toBe("Read");
  });
});
