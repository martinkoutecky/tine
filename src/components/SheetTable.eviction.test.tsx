// og 15a K17a: an open sheet prop-cell draft pins its row's page in the working
// set, so loading many other pages cannot evict the row and silently discard
// the typed draft. Eviction runs through the real ensurePageLoaded path.
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { SheetTable } from "./SheetTable";
import { initParser } from "../render/parse";
import { ensurePageLoaded, pageByName, resetStore } from "../document";
import { doc, setDoc, type FeedPage, type Node as StoreNode } from "../document/model";
import type { PageDto, RefGroup } from "../types";

beforeAll(async () => { await initParser(); });
afterEach(() => { resetStore(); document.body.innerHTML = ""; });

const page = (name: string, roots: string[]): FeedPage =>
  ({ name, kind: "page", title: name, preBlock: null, roots, format: "md", readOnly: false, guide: false });
const node = (id: string, raw: string, pageName: string): StoreNode =>
  ({ id, raw, collapsed: false, parent: null, page: pageName, children: [] });
const filler = (name: string): PageDto =>
  ({ name, kind: "page", title: name, pre_block: null, blocks: [{ id: `${name}-b`, raw: `${name} body`, collapsed: false, children: [] }] } as PageDto);

describe("sheet prop-cell draft and working-set eviction (K17a)", () => {
  it("keeps the row's page loaded while its cell draft is open, and the draft commits", () => {
    setDoc({ byId: { table: node("table", "Table\ntine.view:: table", "Sheet"), r1: node("r1", "Row one\nowner:: old", "Remote") },
      pages: [page("Sheet", ["table"]), page("Remote", ["r1"])], feed: ["Sheet"], loaded: true });
    const groups: RefGroup[] = [{ page: "Remote", kind: "page", blocks: [{ id: "r1", raw: "Row one\nowner:: old", collapsed: false, children: [] }] }];
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <SheetTable ownerId="table" rowSource="query" groups={groups} />, root);
    const cell = root.querySelector('.sheet-cell[data-sheet-grid-id="table"][data-row="0"][data-col="1"]') as HTMLElement;
    cell.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, cancelable: true, button: 0 }));
    const input = root.querySelector("input.sheet-prop-input") as HTMLInputElement;
    expect(input, "prop input opened").not.toBeNull();
    input.value = "Martin";
    input.dispatchEvent(new Event("input", { bubbles: true }));

    for (let i = 0; i < 90; i++) ensurePageLoaded(filler(`Filler ${i}`));

    expect(pageByName("Remote"), "the drafted row's page was evicted").toBeTruthy();
    const open = root.querySelector("input.sheet-prop-input") as HTMLInputElement | null;
    expect(open?.value, "the typed draft is still open").toBe("Martin");
    open!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    expect(doc.byId.r1.raw).toBe("Row one\nowner:: Martin");
    dispose();
  });
});
