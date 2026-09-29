// A pre-split bare column list on a query block is the column SELECTION, not a
// declared schema (master P5A, og OG-E family 3): reading it as a schema marked
// every column stray, and declaring a schema then replaced the note's list. The
// list moves to `tine.columns` in the same undo unit as the declaration.
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { ContextMenu } from "./ContextMenu";
import { SheetTable } from "./SheetTable";
import { initParser } from "../render/parse";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { blockProperty, resetStore, undo } from "../document";
import { doc, setDoc, type FeedPage, type Node as StoreNode } from "../document/model";
import type { BlockDto, RefGroup } from "../types";
import type { ViewSettings } from "../editor/queryIr";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  vi.restoreAllMocks();
  resetSharedQueryResultsForTests();
  resetStore();
  localStorage.clear();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}


function page(roots: string[]): FeedPage {
  return {
    name: "Sheet", kind: "page", title: "Sheet", preBlock: null,
    roots, format: "md", readOnly: false, guide: false,
  };
}

function node(id: string, raw: string): StoreNode {
  return { id, raw, collapsed: false, parent: null, page: "Sheet", children: [] };
}

/** Result rows as the BACKEND ships them: a `BlockDto` with its properties
 *  already computed off the Rust lsdoc projection, which is what the query table
 *  renders from. Not a hand-built row record. */
function resultBlock(id: string, raw: string, properties: [string, string][]): BlockDto {
  return { id, raw, collapsed: false, children: [], properties };
}

function resultGroups(): RefGroup[] {
  return [{
    page: "Tracker",
    kind: "page",
    blocks: [
      resultBlock("r1", "Refresh Guide examples", [["cost", "10"], ["severity", "high"], ["owner", "Avery"]]),
      resultBlock("r2", "Publish the demo", [["cost", "4"], ["severity", "low"], ["owner", "Jules"]]),
    ],
  }];
}


const columnsView = (columns: string[]) => ({
  view: { columns } as ViewSettings,
  apply: () => {},
});

function mountTable(raw: string, columns: string[], schemaPage?: string) {
  const pages = [page(["query"])];
  if (schemaPage) pages.push({ ...page([]), name: schemaPage, title: schemaPage, preBlock: "tine.fields:: severity=text" });
  setDoc({ byId: { query: node("query", raw) }, pages, feed: ["Sheet"], loaded: true });
  return mount(() => (
    <>
      <SheetTable ownerId="query" rowSource="query" schemaPage={schemaPage} groups={resultGroups()} queryDisplay={columnsView(columns)} />
      <ContextMenu />
    </>
  ));
}

/** Declare a schema through the field header's own context menu, the route the user takes. */
function declareThroughHeader(root: HTMLElement, label: string): void {
  const header = [...root.querySelectorAll<HTMLElement>(".sheet-field-header")]
    .find((el) => (el.textContent ?? "").trim() === label);
  expect(header, `a header for ${label}`).toBeTruthy();
  header!.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 20, clientY: 30 }));
  const item = [...document.querySelectorAll<HTMLElement>(".ctx-item")]
    .find((el) => (el.textContent ?? "").trim() === "Declare field (text)");
  expect(item, "a Declare field action").toBeTruthy();
  item!.click();
}

describe("a pre-split bare tine.fields list on a query block", () => {
  it("is not read as a declared (empty) schema", () => {
    const { root, dispose } = mountTable("{{query (task TODO)}}\ntine.fields:: cost;severity", ["cost", "severity"]);
    try {
      expect(root.querySelectorAll(".sheet-col-stray")).toHaveLength(0);
    } finally { dispose(); }
  });

  it("keeps a typed schema readable as a schema", () => {
    const { root, dispose } = mountTable("{{query (task TODO)}}\ntine.fields:: cost=number", ["cost", "severity"]);
    try {
      expect(root.querySelectorAll(".sheet-col-stray").length).toBeGreaterThan(0);
    } finally { dispose(); }
  });

  it("keeps an ordinary children table's bare list as its schema reading", () => {
    const owner = node("query", "Rows\ntine.fields:: cost");
    owner.children = ["child"];
    const child = node("child", "Work\ncost:: 5");
    child.parent = "query";
    setDoc({ byId: { query: owner, child }, pages: [page(["query"])], feed: ["Sheet"], loaded: true });
    const { root, dispose } = mount(() => <SheetTable ownerId="query" rowSource="children" />);
    try {
      expect(root.querySelectorAll(".sheet-col-stray").length).toBeGreaterThan(0);
      expect(blockProperty("query", "tine.columns")).toBeNull();
    } finally { dispose(); }
  });

  it("moves to tine.columns when a schema is declared over it, in one undo unit", () => {
    const { root, dispose } = mountTable("{{query (task TODO)}}\ntine.fields:: cost;severity", ["cost", "severity"]);
    try {
      const before = doc.byId.query.raw;
      declareThroughHeader(root, "cost");
      expect(blockProperty("query", "tine.fields")).toContain("cost=text");
      expect(blockProperty("query", "tine.columns")).toBe("cost;severity");
      undo();
      expect(doc.byId.query.raw).toBe(before);
    } finally { dispose(); }
  });

  it("restores both pages on undo when a page-level schema is declared over it", () => {
    const raw = "{{query (task TODO)}}\ntine.fields:: cost;severity";
    const { root, dispose } = mountTable(raw, ["cost", "severity"], "Schema");
    try {
      declareThroughHeader(root, "cost");
      expect(blockProperty("query", "tine.columns")).toBe("cost;severity");
      undo();
      expect(doc.byId.query.raw).toBe(raw);
      expect(doc.pages.find((p) => p.name === "Schema")?.preBlock).toBe("tine.fields:: severity=text");
    } finally { dispose(); }
  });

  it("never overwrites a PRESENT tine.columns with a rescue", () => {
    const { root, dispose } = mountTable("{{query (task TODO)}}\ntine.columns:: owner\ntine.fields:: cost;severity", ["owner"]);
    try {
      declareThroughHeader(root, "owner");
      expect(blockProperty("query", "tine.columns")).toBe("owner");
      expect(blockProperty("query", "tine.fields")).toContain("owner=text");
    } finally { dispose(); }
  });
});
