import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { SheetTable } from "./SheetTable";
import { ContextMenu } from "./ContextMenu";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { setDoc } from "../document/model";
import type { ViewSettings } from "../editor/queryIr";
import type { RefGroup } from "../types";
import { setContextMenu } from "../ui";

beforeAll(() => initParser());
afterEach(() => { resetStore(); setContextMenu(null); document.body.innerHTML = ""; vi.restoreAllMocks(); });

function mount(view: ViewSettings) {
  setDoc({ byId: { query: { id: "query", raw: "{{query (task TODO)}}", children: [], parent: null, page: "Sheet", collapsed: false } },
    pages: [{ name: "Sheet", title: "Sheet", kind: "page", roots: ["query"], preBlock: null, format: "md", readOnly: false, guide: false }], feed: ["Sheet"], loaded: true });
  const groups: RefGroup[] = [{ page: "Results", kind: "page", blocks: [{ id: "r1", raw: "Visible row", properties: [["cost", "2"]], children: [], collapsed: false }] }];
  let current!: () => ViewSettings;
  let setCurrent!: (next: ViewSettings) => ViewSettings;
  const apply = vi.fn((next: ViewSettings) => setCurrent(next));
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => {
    [current, setCurrent] = createSignal(view);
    return <><SheetTable ownerId="query" rowSource="query" groups={groups}
    queryDisplay={{ get view() { return current(); }, apply }} /><ContextMenu /></>;
  }, root);
  return { root, dispose, apply, current };
}

describe("query table local sorting", () => {
  it("labels an unsavable title sort as table-only and clears it through the label", () => {
    const m = mount({ columns: ["cost"] });
    try {
      m.root.querySelector<HTMLElement>(".sheet-title-header")!.click();
      const label = m.root.querySelector<HTMLButtonElement>(".sheet-table-only-sort");
      expect(label?.textContent).toContain("Table-only sort: Title");
      expect(m.apply).not.toHaveBeenCalled();
      label!.click();
      expect(m.root.querySelector(".sheet-table-only-sort")).toBeNull();
    } finally { m.dispose(); }
  });

  // OG-C5-Q D11/D8: a header click is browsing; it must not write the query,
  // even for a field that could be saved as the query's sort.
  it("sorts a saveable field locally on a header click and never applies a display change", () => {
    const m = mount({ columns: ["cost"] });
    try {
      const header = [...m.root.querySelectorAll<HTMLElement>(".sheet-header-cell")]
        .find((cell) => cell.textContent?.includes("cost"))!;
      header.click();
      expect(m.apply).not.toHaveBeenCalled();
      expect(header.textContent).toContain("\u25B2");
      expect(m.root.querySelector(".sheet-table-only-sort")?.textContent).toContain("Table-only sort: cost");
      header.click();
      expect(header.textContent).toContain("\u25BC");
      header.click();
      expect(header.textContent).not.toMatch(/[\u25B2\u25BC]/);
      expect(m.apply).not.toHaveBeenCalled();
    } finally { m.dispose(); }
  });
});
