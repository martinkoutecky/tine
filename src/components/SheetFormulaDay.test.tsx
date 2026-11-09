// Card CBEnA: a sheet whose formulas read the date (`today()`, `now()`) shows
// the new day's answer once the local date changes in a window left open, in
// table cells, board grouping and `tine.filter`; a sheet that never reads the
// date is not evaluated again. The one date-change signal is `currentDayKey()`
// (journal.ts), the same one queries follow (hB3Lv8). The clock is faked
// across midnight; the second instant is 26 hours after the first so that both
// the local and the UTC calendar date differ in every time zone (the evaluator
// reads `today()` from the instant's UTC date).
import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { SheetTable } from "./SheetTable";
import { SheetBoard } from "./SheetBoard";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { setDoc, type FeedPage, type Node } from "../document/model";
import * as formula from "../sheet/formula";
import { localDayKey, resyncCurrentDayForTest, setCurrentDayKeyForTest } from "../journal";

const FIRST = new Date(2030, 6, 15, 23, 0);
const SECOND = new Date(FIRST.getTime() + 26 * 60 * 60 * 1000);
const day = (instant: Date) => instant.toISOString().slice(0, 10);

beforeAll(initParser);
beforeEach(() => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(FIRST);
  setCurrentDayKeyForTest(localDayKey());
});
afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  resetStore();
  setCurrentDayKeyForTest(localDayKey());
  document.body.innerHTML = "";
});

function sheet(owner: string, rows: string[]): void {
  const ids = rows.map((_, i) => `r${i}`);
  const make = (id: string, raw: string, parent: string | null, children: string[] = []): Node =>
    ({ id, raw, parent, children, page: "Sheet", collapsed: false });
  const page: FeedPage = { name: "Sheet", kind: "page", title: "Sheet", preBlock: null,
    roots: ["sheet"], format: "md", readOnly: false, guide: false };
  setDoc({ byId: {
    sheet: make("sheet", owner, null, ids),
    ...Object.fromEntries(rows.map((raw, i) => [ids[i], make(ids[i], raw, "sheet")])),
  }, pages: [page], feed: [page.name], loaded: true });
}

function mount(view: () => JSX.Element) {
  const root = document.createElement("div");
  document.body.append(root);
  return { root, dispose: render(view, root) };
}

function passMidnight(): void {
  vi.setSystemTime(SECOND);
  resyncCurrentDayForTest();
}

it("a table's today() cell shows the new day after midnight", () => {
  sheet("Table\ntine.formula.day:: today()", ["Row\nscore:: 1"]);
  const { root, dispose } = mount(() => <SheetTable ownerId="sheet" rowSource="children" />);
  expect(root.textContent).toContain(day(FIRST));
  passMidnight();
  expect(root.textContent).toContain(day(SECOND));
  expect(root.textContent).not.toContain(day(FIRST));
  dispose();
});

it("a table filter on today() keeps the rows the new day allows", () => {
  sheet("Table\ntine.filter:: due >= today()", [`Keepable\ndue:: ${day(FIRST)}`]);
  const { root, dispose } = mount(() => <SheetTable ownerId="sheet" rowSource="children" />);
  expect(root.textContent).toContain("Keepable");
  passMidnight();
  expect(root.textContent).not.toContain("Keepable");
  dispose();
});

it("a board grouped by a today() formula regroups under the new day", () => {
  sheet("Board\ntine.view:: board\ntine.formula.day:: today()", ["Card\nscore:: 1"]);
  const { root, dispose } = mount(() => <SheetBoard ownerId="sheet" rowSource="children" groupBy="formula.day" />);
  const headers = () => [...root.querySelectorAll(".sheet-board-header")].map((h) => h.textContent?.trim() ?? "");
  expect(headers().join()).toContain(day(FIRST));
  passMidnight();
  expect(headers().join()).toContain(day(SECOND));
  expect(headers().join()).not.toContain(day(FIRST));
  dispose();
});

it("a sheet whose formulas never read the date evaluates nothing at midnight", () => {
  sheet("Table\ntine.formula.double:: score * 2\ntine.filter:: score > 0", ["Row\nscore:: 3"]);
  const { root, dispose } = mount(() => <><SheetTable ownerId="sheet" rowSource="children" />
    <SheetBoard ownerId="sheet" rowSource="children" /></>);
  expect(root.textContent).toContain("6");
  const evaluator = vi.spyOn(formula, "evaluate");
  passMidnight();
  expect(evaluator).not.toHaveBeenCalled();
  expect(root.textContent).toContain("6");
  dispose();
});
