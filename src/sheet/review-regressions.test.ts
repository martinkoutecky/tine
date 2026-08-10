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

/** The DTO's CONTENT, without the editor identity it now also carries.
 *
 *  `activation` names the live editor instance, not the page's contents, and an
 *  editor legitimately acquires one when it first saves. These assertions are
 *  about a content round-trip, so including identity would report an editor
 *  acquiring its own name as though the undo had failed. (GH #254 increment 3.) */
const content = (dto: unknown) => {
  const { activation: _activation, ...rest } = (dto ?? {}) as Record<string, unknown>;
  return rest;
};

it("insertOutlineAfter refuses read-only pages (file-drop choke point)", () => {
  setDoc({
    byId: { anchor: node("anchor", "Anchor", "Sheet") },
    pages: [page("Sheet", "page", ["anchor"], true)],
    feed: ["Sheet"],
    loaded: true,
  });
  const before = content(pageToDto("Sheet"));

  insertOutlineAfter("anchor", [{ raw: "Dropped", children: [] }]);

  expect(content(pageToDto("Sheet"))).toEqual(before);
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
  const before = content(pageToDto(today));

  expect(await appendToTodayJournal("#Tag ")).toBe(true);
  undo();

  expect(content(pageToDto(today))).toEqual(before);
});
