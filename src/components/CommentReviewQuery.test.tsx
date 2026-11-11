// The open-threads review as a right-sidebar query item (DESIGN §12): it asks the existing
// query engine the printed question, shows the answer as the ordinary live result blocks, and a
// result edits in place. No comments panel, no second comment index.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { RightSidebar } from "./RightSidebar";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { doc, setDoc, type FeedPage, type Node as StoreNode } from "../document/model";
import type { RefGroup } from "../types";
import type { ParsedQuery } from "../editor/queryIr";
import { blockRunResult } from "../tests/queryReadingsTestkit";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { applySidebarSession, rightSidebar, setRightSidebar, type SidebarQuery } from "../ui";
import { openThreadsQuery, openCommentReviewInSidebar } from "../commentReviewQuery";
import { editingId, endEdit } from "../editorController";
import { maybeEditorIn } from "../tests/editorDriver";
import { resetNearObserverForTests } from "../lazyObserve";

const saveFixture = vi.hoisted(() => ({ actual: null as Record<string, unknown> | null }));
vi.mock("../document/host/wiring", () => new Proxy({}, {
  get: (_target, key) => key === "then" ? undefined : key === "markDirty" ? vi.fn() : key === "persistTransfer" ? vi.fn(async () => true) : saveFixture.actual?.[key as string],
  has: () => true,
}));

beforeAll(async () => { saveFixture.actual = await vi.importActual("../document/host/wiring"); await initParser(); });
afterEach(() => {
  endEdit("page-navigation"); resetNearObserverForTests(); setRightSidebar([]); applySidebarSession({ right: false, items: [] });
  resetSharedQueryResultsForTests(); resetStore(); localStorage.clear(); document.body.innerHTML = ""; vi.restoreAllMocks();
});

const readQuery = (text: string): ParsedQuery => ({
  query: { anchor: "block", filter: { kind: "raw", text, diagnostic_kind: "not_applicable" }, diagnostics: [], source: { kind: "og", original: text, og_options: "" } },
  view: {}, legacy_table: false,
} as ParsedQuery);

const node = (id: string, raw: string, parent: string | null, children: string[] = []): StoreNode => ({ id, raw, collapsed: false, parent, page: "Notes", children });
const feedPage = (roots: string[]): FeedPage => ({ name: "Notes", kind: "page", title: "Notes", preBlock: null, roots, format: "md", readOnly: false, guide: false });
const group = (...ids: string[]): RefGroup[] => [{ page: "Notes", kind: "page", blocks: ids.map((id) => ({ id, raw: doc.byId[id].raw, collapsed: false, children: [], properties: [] })) }];

function load() {
  setDoc({
    byId: {
      body: node("body", "Alpha passage", null, ["open", "done"]),
      open: node("open", "Open thread\ntine.quote:: Alpha", "body"),
      done: node("done", "Settled thread\ntine.quote:: passage\ntine.resolved:: true", "body"),
    },
    pages: [feedPage(["body"])], feed: ["Notes"], loaded: true,
  });
}
function mount() {
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <RightSidebar />, root);
  return { root, dispose };
}
const settle = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

beforeEach(() => {
  vi.spyOn(backend(), "parseQuery").mockImplementation(async (text) => readQuery(text));
  vi.spyOn(backend(), "getBacklinks").mockResolvedValue([]);
  vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([]);
  vi.spyOn(backend(), "getBlockRefCounts").mockResolvedValue({});
});

describe("open-comments review item", () => {
  it.each([
    ["page", { scope: "page", page: "Notes", pageKind: "page" }, { name: "Notes", pageKind: "page" as const }],
    ["graph", { scope: "graph", page: "", pageKind: "page" }, undefined],
  ] as const)("asks the engine the %s-scoped question and lists the answer as live blocks", async (scope, item, target) => {
    load();
    const run = vi.spyOn(backend(), "queryRun").mockImplementation(async () => blockRunResult(group("open")));
    openCommentReviewInSidebar(scope, target);
    expect(rightSidebar()[0]).toMatchObject(item);
    const { root, dispose } = mount();
    try {
      await vi.waitFor(() => expect(root.querySelector('.query-group [data-block-id="open"]')).not.toBeNull());
      const asked = run.mock.calls.map(([query]) => (query.source as { original: string }).original);
      expect(asked).toEqual([openThreadsQuery(rightSidebar()[0] as SidebarQuery)]);
      expect(asked[0]).toContain("(not (property tine.resolved true))");
      expect(root.querySelector('[data-block-id="done"]')).toBeNull();
      expect(root.querySelector(".rs-item-title")?.textContent).toBe(scope === "page" ? "Open comments on Notes" : "Open comments in the graph");
      expect(root.querySelector(".rs-item")?.getAttribute("data-comment-review")).toBe(scope);
    } finally { dispose(); }
  });

  it("filters by author: a new author is a new question", async () => {
    load();
    const run = vi.spyOn(backend(), "queryRun").mockImplementation(async () => blockRunResult(group("open")));
    openCommentReviewInSidebar("graph");
    const { root, dispose } = mount();
    try {
      await vi.waitFor(() => expect(run).toHaveBeenCalledTimes(1));
      const input = root.querySelector<HTMLInputElement>("[data-comment-review-author]")!;
      input.value = "Ada"; input.dispatchEvent(new Event("change", { bubbles: true }));
      expect((rightSidebar()[0] as SidebarQuery).author).toBe("Ada");
      await vi.waitFor(() => expect(run).toHaveBeenCalledTimes(2));
      expect((run.mock.calls[1][0].source as { original: string }).original).toContain('(property tine.author "Ada")');
      expect(root.querySelector<HTMLInputElement>("[data-comment-review-author]")!.value).toBe("Ada");
    } finally { dispose(); }
  });

  it("edits a result in place: the one underlying block changes, not a copy", async () => {
    load();
    vi.spyOn(backend(), "queryRun").mockImplementation(async () => blockRunResult(group("open")));
    openCommentReviewInSidebar("graph");
    const { root, dispose } = mount();
    try {
      await vi.waitFor(() => expect(root.querySelector('.query-group [data-block-id="open"]')).not.toBeNull());
      const content = root.querySelector<HTMLElement>('[data-block-id="open"] .block-content')!;
      for (const type of ["mousedown", "mouseup", "click"]) content.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, button: 0 }));
      const editor = await vi.waitFor(() => { const found = maybeEditorIn(root); expect(found).not.toBeNull(); return found!; });
      expect(editingId()).toBe("open");
      editor.input("Open thread, now with an answer\ntine.quote:: Alpha", 31, { inputType: "insertText", data: "r" });
      endEdit("page-navigation");
      expect(doc.byId.open.raw).toContain("now with an answer");
      expect(doc.byId.open.raw).toContain("tine.quote:: Alpha");
    } finally { dispose(); }
  });

  it("collapsing the item mounts no query; expanding runs it", async () => {
    load();
    const run = vi.spyOn(backend(), "queryRun").mockImplementation(async () => blockRunResult(group("open")));
    setRightSidebar([{ kind: "query", scope: "graph", page: "", pageKind: "page", collapsed: true }]);
    applySidebarSession({ right: true, items: rightSidebar() });
    const { root, dispose } = mount();
    try {
      await settle();
      expect(run).not.toHaveBeenCalled();
      root.querySelector<HTMLButtonElement>("[data-right-sidebar-item-toggle]")!.click();
      await vi.waitFor(() => expect(run).toHaveBeenCalledTimes(1));
    } finally { dispose(); }
  });
});
