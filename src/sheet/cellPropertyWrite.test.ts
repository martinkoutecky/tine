// A cell's property write (`writeField` → `setCellProperty`) refuses when the
// row holds another spelling of the column's key; every non-cell caller of
// `setBlockProperty` keeps its case-folding write, result and failure path.
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import * as parser from "../render/parse";
import { resetStore, setBlockProperty, withUndoUnit } from "../document";
import { undoTopTag } from "../document/history";
import { doc, setDoc, type FeedPage, type Node } from "../document/model";
import { setToasts, toasts } from "../toasts";
import { writeField, writeFieldVisibly } from "./fields";

const saveFixture = vi.hoisted(() => ({ actual: null as Record<string, unknown> | null, markDirty: vi.fn() }));
vi.mock("../document/host/wiring", () => new Proxy({}, {
  get: (_target, key) => key === "then" ? undefined : key === "markDirty" ? saveFixture.markDirty
    : saveFixture.actual?.[key as string],
  has: () => true,
}));

beforeAll(async () => {
  saveFixture.actual = await vi.importActual("../document/host/wiring");
  await parser.initParser();
});

afterEach(() => {
  resetStore();
  setToasts([]);
  saveFixture.markDirty.mockClear();
  vi.restoreAllMocks();
});

function load(raw: string, format: "md" | "org" = "md") {
  const page: FeedPage = { name: "Sheet", kind: "page", title: "Sheet", preBlock: null, roots: ["r"], format, readOnly: false, guide: false };
  const node: Node = { id: "r", raw, collapsed: false, parent: null, page: "Sheet", children: [] };
  setDoc({ byId: { r: node }, pages: [page], feed: ["Sheet"], loaded: true });
}

const errors = () => toasts().filter((t) => t.kind === "error").map((t) => t.message);

describe("writeField on a property column", () => {
  it.each([
    ["Row\nready:: false", "prop:Ready", "yes", "ready"],
    ["Row\nready:: false", "prop:Ready", "", "ready"],
    ["Row\nReady:: yes\nready:: true", "prop:ready", "false", "Ready"],
    ["Row\nready:: true\nReady:: yes", "prop:Ready", "", "ready"],
    ["Row\na-b:: false", "prop:a_b", "true", "a-b"],
    ["Row\r\nShip_Ped:: true", "prop:ship-ped", "false", "Ship_Ped"],
  ])("%j: %s := %j is refused visibly and writes nothing", (raw, field, value, other) => {
    load(raw);
    const undoBefore = undoTopTag();
    expect(writeField("r", field as `prop:${string}`, value)).toBe(false);
    expect(errors()).toEqual([]);
    expect(writeFieldVisibly("r", field as `prop:${string}`, value)).toBe(false);
    expect(doc.byId.r.raw).toBe(raw);
    expect(undoTopTag()).toBe(undoBefore);
    expect(saveFixture.markDirty).not.toHaveBeenCalled();
    expect(errors()).toEqual([`Not changed: this row already has “${other}”, another spelling of this property.`]);
  });

  it.each([
    ["Row\nready:: false", "prop:ready", "true", "Row\nready:: true"],
    ["Row", "prop:Ready", "yes", "Row\nReady:: yes"],
    ["Row\nready:: false", "prop:ready", "", "Row"],
    ["Row\r\nready:: false", "prop:ready", "true", "Row\r\nready:: true"],
  ])("%j: %s := %j writes develop's bytes", (raw, field, value, expected) => {
    load(raw);
    expect(writeFieldVisibly("r", field as `prop:${string}`, value)).toBe(true);
    expect(doc.byId.r.raw).toBe(expected);
    expect(saveFixture.markDirty).toHaveBeenCalledTimes(1);
    expect(errors()).toEqual([]);
  });

  it("costs one editBlock per cell write, refused or not", () => {
    const edit = vi.spyOn(parser, "editBlock");
    load("Row\nready:: false");
    writeFieldVisibly("r", "prop:Ready", "yes");
    expect(edit).toHaveBeenCalledTimes(1);
    writeFieldVisibly("r", "prop:ready", "true");
    expect(edit).toHaveBeenCalledTimes(2);
  });
});

describe("setBlockProperty (non-cell callers) is unchanged", () => {
  it("still folds ASCII case and removes or rewrites the file's spelling", () => {
    load("Row\ncollapsed:: true");
    expect(setBlockProperty("r", "Collapsed", null)).toBeUndefined();
    expect(doc.byId.r.raw).toBe("Row");
    load("Row\nTine.Resolved:: true");
    setBlockProperty("r", "tine.resolved", null);
    expect(doc.byId.r.raw).toBe("Row");
    load("Row\nOwner:: a");
    setBlockProperty("r", "owner", "b");
    expect(doc.byId.r.raw).toBe("Row\nOwner:: b");
  });

  it("a failed write inside an undo unit keeps develop's history and dirty state", () => {
    // The core adds to an Org drawer only before a whole `:END:` line. On
    // develop the unit completes (the callback returns undefined) and marks
    // nothing dirty; a false result would roll back and mark the page.
    const raw = "#+BEGIN_QUERY\n(and)\n#+END_QUERY\n:PROPERTIES:\n:ready: false\n:END: trailing";
    load(raw, "org");
    const result = withUndoUnit("query:breadcrumbs", ["Sheet"], () => setBlockProperty("r", "tine.breadcrumbs", "false"));
    expect(result).toBeUndefined();
    expect(doc.byId.r.raw).toBe(raw);
    expect(undoTopTag()).toBe("query:breadcrumbs");
    expect(saveFixture.markDirty).not.toHaveBeenCalled();
    expect(errors()).toEqual(["Properties drawer has no parser-owned closer"]);
  });
});
