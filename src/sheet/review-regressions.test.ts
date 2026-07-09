// Regression locks for the Phase-6 adversarial review findings (Jul 2026):
// read-only gating at the insertOutlineAfter choke point and the aggregate
// footer, and one-undo atomicity for the empty-journal append path.
import { beforeAll, beforeEach, expect, it } from "vitest";
import { initParser } from "../render/parse";
import {
  appendToTodayJournal,
  doc,
  insertOutlineAfter,
  pageToDto,
  resetStore,
  setDoc,
  undo,
  type FeedPage,
  type Node,
} from "../store";
import { journalTitle } from "../journal";
import { setColumnAggregate } from "./mutations";

beforeAll(async () => {
  await initParser();
});
beforeEach(() => {
  resetStore();
});

function page(name: string, kind: "page" | "journal", roots: string[], readOnly = false): FeedPage {
  return { name, kind, title: name, preBlock: null, roots, format: "md", readOnly, guide: false };
}
function node(id: string, raw: string, pageName: string, parent: string | null = null, children: string[] = []): Node {
  return { id, raw, collapsed: false, parent, page: pageName, children };
}

it("insertOutlineAfter refuses read-only pages (file-drop choke point)", () => {
  setDoc({
    byId: { anchor: node("anchor", "Anchor", "Sheet") },
    pages: [page("Sheet", "page", ["anchor"], true)],
    feed: ["Sheet"],
    loaded: true,
  });
  const before = pageToDto("Sheet");

  insertOutlineAfter("anchor", [{ raw: "Dropped", children: [] }]);

  expect(pageToDto("Sheet")).toEqual(before);
});

it("setColumnAggregate refuses read-only owners (footer bypassed the gridPage gate)", () => {
  setDoc({
    byId: { table: node("table", "Table\ntine.view:: table", "Sheet") },
    pages: [page("Sheet", "page", ["table"], true)],
    feed: ["Sheet"],
    loaded: true,
  });
  const before = doc.byId.table.raw;

  setColumnAggregate("table", "prop:estimate", "sum");

  expect(doc.byId.table.raw).toBe(before);
});

it("appending to an empty today journal undoes in one step (anchor/insert/delete = one unit)", async () => {
  const today = journalTitle(new Date());
  setDoc({ byId: {}, pages: [page(today, "journal", [])], feed: [today], loaded: true });
  const before = pageToDto(today);

  expect(await appendToTodayJournal("#Tag ")).toBe(true);
  undo();

  expect(pageToDto(today)).toEqual(before);
});
