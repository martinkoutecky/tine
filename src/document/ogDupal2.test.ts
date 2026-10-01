import { beforeAll, afterEach, expect, it } from "vitest";
import { initParser } from "../render/parse";
import { resetStore } from ".";
import { setDoc, type Node, type FeedPage } from "./model";
import { visibleData, pageVisibleOrder, scopedVisibleOrder } from "./tree";

beforeAll(() => initParser());
afterEach(resetStore);
it("feed, capture page and scoped navigation share collapsed and opaque-sheet gates, retaining their missing-id policies", () => {
  const node = (id: string, children: string[] = [], raw = id): Node =>
    ({ id, raw, parent: null, children, page: "A", collapsed: id === "fold" });
  const page: FeedPage = { name: "A", title: "A", kind: "page", roots: ["open", "fold", "sheet", "missing"],
    preBlock: null, format: "md", readOnly: false, guide: false };
  setDoc({ pages: [page], feed: ["A"], loaded: true, byId: {
    open: node("open", ["child"]), child: node("child"),
    fold: node("fold", ["hidden"]), hidden: node("hidden"),
    sheet: node("sheet", ["row"], "Sheet\ntine.view:: table"), row: node("row"),
  } });
  const expected = ["open", "child", "fold", "sheet", "missing"];
  expect(pageVisibleOrder("A")).toEqual(expected);
  expect(visibleData().order).toEqual(expected);
  expect([...visibleData().index.keys()]).toEqual(expected);
  expect(scopedVisibleOrder({ roots: page.roots })).toEqual(expected.slice(0, -1));
  expect(scopedVisibleOrder({ roots: page.roots, forceExpandedRoot: "fold" })).toEqual(["open", "child", "fold", "hidden", "sheet"]);
  expect(scopedVisibleOrder({ roots: page.roots, collapsed: () => true })).toEqual(["open", "fold", "sheet"]);
});
