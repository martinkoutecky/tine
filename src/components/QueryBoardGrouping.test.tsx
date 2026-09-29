// The grouping a query BOARD draws is the engine's one answer (`view.group_by`,
// resolved by `resolve_query_grouping`: `tine.group-field` first, then the legacy
// `tine.group-by`, then the lifted directive), not a second reading of
// `tine.group-by` in the component (I-12; master P5B). Before this, a Display
// grouping on a board wrote `tine.group-field` and the board ignored it.
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { Block } from "./Block";
import { ContextMenu } from "./ContextMenu";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { blockProperty, resetStore, setBlockProperty } from "../document";
import { setDoc, type FeedPage, type Node as StoreNode } from "../document/model";
import type { BlockDto, RefGroup } from "../types";
import type { ViewSettings } from "../editor/queryIr";
import { blockRunResult } from "../tests/queryReadingsTestkit";

beforeAll(async () => { await initParser(); });
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

const setRawGroup = (field: string) => { setBlockProperty("query", "tine.group-field", field); };

const row = (id: string, raw: string, status: string): BlockDto =>
  ({ id, raw, collapsed: false, children: [], properties: [["status", status]] });
const groups = (): RefGroup[] => [{
  page: "Sheet", kind: "page",
  blocks: [row("todo-a", "TODO A", "open"), row("todo-b", "TODO B", "done")],
}];

function load(raw: string, view: ViewSettings): void {
  const page: FeedPage = { name: "Sheet", kind: "page", title: "Sheet", preBlock: null, roots: ["query"], format: "md", readOnly: false, guide: false };
  const query: StoreNode = { id: "query", raw, collapsed: false, parent: null, page: "Sheet", children: [] };
  setDoc({ byId: { query }, pages: [page], feed: ["Sheet"], loaded: true });
  const parse = backend().parseQuery.bind(backend());
  vi.spyOn(backend(), "parseQuery").mockImplementation(async (...args) => {
    const parsed = await parse(...args);
    return { ...parsed, view: { ...parsed.view, ...view }, block_presentation: "board" };
  });
  vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups()));
}


describe("a query board groups by the engine's resolved grouping", () => {
  it("draws the columns of a Display grouping (tine.group-field), not the task-marker default", async () => {
    load("{{query (task TODO)}}\ntine.view:: board\ntine.group-field:: prop:status", { group_by: "prop:status" });
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await vi.waitFor(() => expect(root.querySelector(".sheet-board")).not.toBeNull());
      const text = root.querySelector(".sheet-board")!.textContent ?? "";
      expect(text).toContain("open");
      expect(text).toContain("done");
      expect(root.querySelector<HTMLSelectElement>(".sheet-board-groupby")!.value).toBe("prop:status");
    } finally { dispose(); }
  });

  it("still groups by the legacy tine.group-by when the engine states nothing", async () => {
    load("{{query (task TODO)}}\ntine.view:: board\ntine.group-by:: prop:status", {});
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await vi.waitFor(() => expect(root.querySelector(".sheet-board")).not.toBeNull());
      expect(root.querySelector<HTMLSelectElement>(".sheet-board-groupby")!.value).toBe("prop:status");
    } finally { dispose(); }
  });

  it("moves the Display grouping when the toolbar dropdown changes it", async () => {
    load("{{query (task TODO)}}\ntine.view:: board\ntine.group-field:: prop:status", { group_by: "prop:status" });
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await vi.waitFor(() => expect(root.querySelector(".sheet-board")).not.toBeNull());
      const select = root.querySelector<HTMLSelectElement>(".sheet-board-groupby")!;
      select.value = "priority";
      select.dispatchEvent(new Event("change", { bubbles: true }));
      // The engine reads `tine.group-field` first: leaving it behind would make the
      // choice a no-op.
      expect(blockProperty("query", "tine.group-field")).toBe("priority");
    } finally { dispose(); }
  });

  it("draws ONE column of every result for an explicit No grouping", async () => {
    // FAIL-BEFORE: `""` fell through `boardGroupField` to the task-marker default, so
    // clearing the grouping brought the state columns back.
    load("{{query (task TODO)}}\ntine.view:: board\ntine.group-field::", { group_by: "" });
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await vi.waitFor(() => expect(root.querySelector(".sheet-board")).not.toBeNull());
      const columns = root.querySelectorAll(".sheet-board-column");
      expect(columns).toHaveLength(1);
      expect(columns[0].textContent).toContain("All results");
      expect(columns[0].querySelectorAll(".sheet-board-card")).toHaveLength(2);
      expect(root.querySelector<HTMLSelectElement>(".sheet-board-groupby")!.value).toBe("");
    } finally { dispose(); }
  });

  it("writes an explicit clear from the toolbar and from the context menu", async () => {
    load("{{query (task TODO)}}\ntine.view:: board\ntine.group-by:: prop:status", { group_by: "prop:status" });
    const { root, dispose } = mount(() => (<><Block id="query" /><ContextMenu /></>));
    try {
      await vi.waitFor(() => expect(root.querySelector(".sheet-board")).not.toBeNull());
      const select = root.querySelector<HTMLSelectElement>(".sheet-board-groupby")!;
      expect([...select.options].some((option) => option.value === "")).toBe(true);
      select.value = "";
      select.dispatchEvent(new Event("change", { bubbles: true }));
      expect(blockProperty("query", "tine.group-field")).toBe("");
      expect(blockProperty("query", "tine.group-by")).toBeNull();

      // the same writer from the context menu
      setRawGroup("prop:status");
      root.querySelector(".sheet-board")!.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, clientX: 10, clientY: 10 }));
      const item = [...document.querySelectorAll<HTMLElement>(".ctx-submenu-menu .ctx-item")]
        .find((el) => (el.textContent ?? "").replace("✓ ", "").trim() === "No grouping");
      expect(item, "a Group by -> No grouping action").toBeTruthy();
      item!.click();
      expect(blockProperty("query", "tine.group-field")).toBe("");
    } finally { dispose(); }
  });
});
