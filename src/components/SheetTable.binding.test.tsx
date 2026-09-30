import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { SheetTable, resetSheetRowVirtualizationForTests } from "./SheetTable";
import { resetStore } from "../document";
import { setDoc } from "../document/model";
import { initParser } from "../render/parse";
import { resetNearObserverForTests } from "../lazyObserve";
beforeAll(initParser);
afterEach(() => { vi.unstubAllGlobals(); resetNearObserverForTests(); resetStore(); document.body.innerHTML = ""; });
function load() {
  setDoc({ byId: {
    table: { id: "table", raw: "Table", collapsed: false, parent: null, page: "Sheet", children: ["row"] },
    row: { id: "row", raw: "**row body**", collapsed: false, parent: "table", page: "Sheet", children: [] },
  }, pages: [{ name: "Sheet", kind: "page", title: "Sheet", preBlock: null, roots: ["table"], format: "md", readOnly: false, guide: false }], feed: ["Sheet"], loaded: true });
}
function mount() {
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <SheetTable ownerId="table" rowSource="children" />, root);
  return { root, dispose };
}
it("retains visited sheet rows on remount, but defers colliding IDs in the next graph", () => {
  resetSheetRowVirtualizationForTests(); load();
  const first = mount(); expect(first.root.querySelector("strong")?.textContent).toBe("row body"); first.dispose();
  vi.stubGlobal("IntersectionObserver", class { observe() {} unobserve() {} disconnect() {} });
  const same = mount(); expect(same.root.querySelector("strong")?.textContent).toBe("row body"); same.dispose();
  resetStore(); load();
  const next = mount(); expect(next.root.querySelector("strong")).toBeNull(); next.dispose();
});

it("renders a shared-reference DAG and inherited names as numbers or cell errors", () => {
  resetSheetRowVirtualizationForTests(); load();
  const definitions = ["tine.formula.f0:: 1", "tine.formula.bad:: 'x'.__proto__"];
  for (let i = 1; i <= 32; i++) definitions.push(`tine.formula.f${i}:: formula.f${i - 1} + formula.f${i - 1}`);
  setDoc("byId", "table", "raw", ["Table", ...definitions].join("\n"));
  const sheet = mount();
  expect(sheet.root.textContent).toContain("4294967296");
  expect(sheet.root.querySelector(".sheet-formula-error")?.getAttribute("title")).toContain("Unknown property __proto__");
  sheet.dispose();
});
