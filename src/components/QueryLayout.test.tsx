// D-20261028-UILP: query tables stay inside their box, long Markdown cells wrap and
// short ones do not, the board heading keeps its count apart from the label, page
// links in cards and tables are links, the group breakdown folds. Geometry itself
// (jsdom has no layout) is asserted by scripts/e2e-query-layout.mjs.
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import type { PageRow } from "../editor/queryIr";
import { QueryPageRows, QueryStatisticsSummary } from "./QueryResultParts";
import { LONG_CELL_CHARS, longCellClass } from "./TableWrap";
import { renderBlocks } from "../render/body";
import { initParser } from "../render/parse";
import type { Block } from "../render/ast";

beforeAll(initParser);
afterEach(() => { document.body.innerHTML = ""; });

function mount(node: () => JSX.Element, host?: HTMLElement): { root: HTMLElement; dispose: () => void } {
  const root = host ?? document.createElement("div");
  if (!host) document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}
const row = (name: string, properties: [string, string][] = []): PageRow => ({ name, path: `pages/${name}.md`, kind: "page", properties });
const frames = (n = 3) => new Promise<void>((resolve) => { const step = (left: number) => left === 0 ? resolve() : requestAnimationFrame(() => step(left - 1)); step(n); });

describe("TableWrap viewport", () => {
  const rows = [row("Alpha", [["status", "open"]])];
  function pane(inQueryBox: boolean): HTMLElement {
    const main = document.createElement("div");
    main.className = "main-content";
    const parent = inQueryBox ? Object.assign(document.createElement("div"), { className: "query-block" }) : document.createElement("div");
    main.append(parent);
    document.body.append(main);
    return parent;
  }
  it("measures a table in the page column (the bleed owner)", async () => {
    const host = pane(false);
    const { root, dispose } = mount(() => <QueryPageRows rows={rows} view="table" />, host);
    try {
      await frames();
      const wrap = root.querySelector<HTMLElement>(".md-table-wrap")!;
      expect(wrap.style.getPropertyValue("--table-bleed-width")).not.toBe("");
    } finally { dispose(); }
  });
  it("never bleeds a table inside a query box: the box is its viewport", async () => {
    const host = pane(true);
    const { root, dispose } = mount(() => <QueryPageRows rows={rows} view="table" />, host);
    try {
      await frames();
      const wrap = root.querySelector<HTMLElement>(".md-table-wrap")!;
      expect(wrap.style.getPropertyValue("--table-bleed-width")).toBe("");
      expect(wrap.style.getPropertyValue("--table-bleed-shift")).toBe("");
    } finally { dispose(); }
  });
});

describe("Markdown table cells", () => {
  it("marks only long text as wrappable, never a number or a short label", () => {
    expect(longCellClass("16.7")).toBeUndefined();
    expect(longCellClass("x".repeat(LONG_CELL_CHARS))).toBeUndefined();
    expect(longCellClass("x".repeat(LONG_CELL_CHARS + 1))).toBe("md-cell-long");
    const text = (s: string) => [{ k: "plain", text: s }] as never;
    const table: Block = { kind: "table", header: [text("Name"), text("Value")], aligns: [],
      rows: [[text("A fairly long description of this entry that runs past forty characters"), text("16.7")]] } as never;
    const div = document.createElement("div");
    const dispose = render(() => renderBlocks([table]), div);
    try {
      const cells = [...div.querySelectorAll("tbody td")];
      expect(cells.map((c) => c.classList.contains("md-cell-long"))).toEqual([true, false]);
      expect(div.querySelectorAll("thead th.md-cell-long")).toHaveLength(0);
    } finally { dispose(); }
  });
});

describe("board and table presentation", () => {
  const rows = [row("Alpha", [["area", "one"]]), row("Beta", [["area", "one"]]), row("Gamma", [["area", "two"]])];
  it("keeps the column count in its own element, apart from the label", () => {
    const { root, dispose } = mount(() => <QueryPageRows rows={rows} view="board" groupBy="prop:area" />);
    try {
      const heading = root.querySelector<HTMLElement>(".query-board-column > h4.query-board-heading")!;
      expect(heading.querySelector(".query-board-label")?.textContent).toBe("one");
      expect(heading.querySelector(".query-board-count")?.textContent).toBe("2");
      expect(heading.children).toHaveLength(2);
      expect(root.querySelectorAll(".query-board-card > button.query-page-link")).toHaveLength(3);
    } finally { dispose(); }
  });
});

describe("group breakdown", () => {
  const statistics = {
    count: 3, aggregates: [["", "count"]] as [string, string][], group_by: "prop:status",
    overall: [{ kind: "number", value: 3, skipped: 0 }],
    groups: [{ key: "open", count: 2, cells: [{ kind: "number", value: 2, skipped: 0 }] }, { key: "done", count: 1, cells: [{ kind: "number", value: 1, skipped: 0 }] }],
    grouping_status: "exact",
  } as never;
  it("folds behind its Grouped by line, still in the DOM for assistive technology", () => {
    const { root, dispose } = mount(() => <QueryStatisticsSummary statistics={statistics} />);
    try {
      const details = root.querySelector<HTMLDetailsElement>("details.query-summary-details")!;
      expect(details.open).toBe(false);
      expect(details.querySelector("summary")?.textContent).toBe("Grouped by status");
      expect(details.querySelectorAll(".query-summary-table tbody tr")).toHaveLength(2);
      expect(root.textContent?.match(/Grouped by/g)).toHaveLength(1);
    } finally { dispose(); }
  });
});
