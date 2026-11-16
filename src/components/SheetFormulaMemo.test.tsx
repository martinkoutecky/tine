import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { SheetTable } from "./SheetTable";
import { SheetBoard } from "./SheetBoard";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { setDoc, type Node, type FeedPage } from "../document/model";
import * as formula from "../sheet/formula";

beforeAll(initParser);
afterEach(() => { resetStore(); vi.restoreAllMocks(); document.body.innerHTML = ""; });

it("updates table cells and board membership after one row edit without reevaluating other rows", () => {
  const makeNode = (id: string, raw: string, parent: string | null, children: string[] = []): Node =>
    ({ id, raw, parent, children, page: "Sheet", collapsed: false });
  const ids = Array.from({ length: 40 }, (_, i) => `r${i}`);
  const page: FeedPage = { name: "Sheet", kind: "page", title: "Sheet", preBlock: null,
    roots: ["table"], format: "md", readOnly: false, guide: false };
  setDoc({ byId: {
    table: makeNode("table", "Table\ntine.formula.double:: score * 2\ntine.filter:: score > 0", null, ids),
    ...Object.fromEntries(ids.map(id => [id, makeNode(id, "TODO Row\nscore:: 1", "table")])),
  }, pages: [page], feed: [page.name], loaded: true });
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <><SheetTable ownerId="table" rowSource="children" />
    <SheetBoard ownerId="table" rowSource="children" /></>, root);
  const evaluator = vi.spyOn(formula, "evaluate");
  setDoc("byId", "r17", "raw", "TODO Changed\nscore:: 7");
  expect(root.querySelector(".sheet-table")?.textContent).toContain("14");
  expect(evaluator.mock.calls.length).toBeLessThanOrEqual(3);
  evaluator.mockClear();
  setDoc("byId", "r17", "raw", "TODO Changed\nscore:: 0");
  expect(root.querySelector(".sheet-board")?.textContent).not.toContain("Changed");
  expect(root.querySelector(".sheet-table")?.textContent).not.toContain("Changed");
  expect(evaluator.mock.calls.length).toBeLessThanOrEqual(3);
  dispose();
});
