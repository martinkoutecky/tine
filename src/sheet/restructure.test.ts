import { beforeAll, beforeEach, describe, expect, it } from "vitest";
import { __setBackendForTest } from "../backend";
import { managedStorageRuntime } from "../managedStorageRuntime";
import { mockBackend } from "../mock";
import { initParser } from "../render/parse";
import { doc, pageToDto, resetStore, setDoc, undo, type FeedPage, type Node as StoreNode } from "../store";
import { flatten, hierarchify } from "./restructure";

beforeAll(() => initParser());

beforeEach(() => {
  resetStore();
  managedStorageRuntime.bind(1, { binding_generation: 1, authority: "direct" });
  __setBackendForTest(null);
});

function bindManaged(preflight: "accepted" | "refused") {
  managedStorageRuntime.bind(51, {
    binding_generation: 51,
    authority: "managed_writable",
    application_save_page_blocks: 511,
    application_page_request_text_bytes: 1024 * 1024,
    application_page_max_depth: 128,
  });
  const base = mockBackend();
  let calls = 0;
  __setBackendForTest({
    ...base,
    preflightManagedPageMutation: async (candidate, baseRevision, bindingGeneration) => {
      calls++;
      return preflight === "accepted"
        ? {
            status: "accepted",
            binding_generation: bindingGeneration,
            page_name: candidate.name,
            page_path: candidate.path ?? "",
            base_revision: baseRevision,
          }
        : { status: "refused" };
    },
  });
  return () => calls;
}

async function settleManagedPlan() {
  await Promise.resolve();
  await Promise.resolve();
}

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

  it("routes managed hierarchify and flatten through one atomic preflight each", async () => {
    loadTable();
    const before = pageToDto("Sheet");
    const calls = bindManaged("accepted");

    expect(hierarchify("table", "state")).toBe(true);
    expect(pageToDto("Sheet")).toEqual(before);
    await settleManagedPlan();
    expect(doc.byId.table.children).toHaveLength(2);

    expect(flatten("table")).toBe(true);
    await settleManagedPlan();
    expect(pageToDto("Sheet")).toEqual(before);
    expect(calls()).toBe(2);
  });

  it("leaves restructure entirely unpublished when managed preflight refuses", async () => {
    loadTable();
    const before = pageToDto("Sheet");
    const calls = bindManaged("refused");

    expect(hierarchify("table", "priority")).toBe(true);
    await settleManagedPlan();

    expect(calls()).toBe(1);
    expect(pageToDto("Sheet")).toEqual(before);
  });
});
