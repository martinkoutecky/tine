import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { SheetTable } from "./SheetTable";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { setDoc } from "../document/model";
import type { RefGroup } from "../types";

beforeAll(() => initParser());
afterEach(() => { resetStore(); document.body.innerHTML = ""; });

// GH #551 item 3 (EllisMorrow): Markdown lists inside query results showed as source text. The List view
// draws a result as the page draws it; the Table view joined the block's lines into one inline string,
// so `* apples` came out as the characters "* apples * pears".
function mountQueryTable() {
  setDoc({
    byId: { query: { id: "query", raw: "{{query [[lists]]}}", children: [], parent: null, page: "Sheet", collapsed: false } },
    pages: [{ name: "Sheet", title: "Sheet", kind: "page", roots: ["query"], preBlock: null, format: "md", readOnly: false, guide: false }],
    feed: ["Sheet"], loaded: true,
  });
  const groups: RefGroup[] = [{
    page: "Lists", kind: "page",
    blocks: [
      { id: "r1", raw: "Shopping notes #lists\n* apples\n* pears", properties: [], children: [], collapsed: false },
      { id: "r2", raw: "Numbered steps #lists\n1. first\n2. second", properties: [], children: [], collapsed: false },
      { id: "r3", raw: "A plain title", properties: [], children: [], collapsed: false },
    ],
  }];
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <SheetTable ownerId="query" rowSource="query" groups={groups} />, root);
  return { root, dispose };
}

describe("Table view of query results draws a block as the page does", () => {
  it("renders a Markdown bullet list and a numbered list as lists, not as source text", () => {
    const m = mountQueryTable();
    try {
      const titles = [...m.root.querySelectorAll<HTMLElement>(".sheet-title-cell")];
      expect(titles).toHaveLength(3);
      expect([...titles[0].querySelectorAll("ul li")].map((li) => li.textContent)).toEqual(["apples", "pears"]);
      expect([...titles[1].querySelectorAll("ol li")].map((li) => li.textContent)).toEqual(["first", "second"]);
      expect(m.root.textContent).not.toMatch(/\* apples|1\. first/);
      expect(titles[0].textContent).toContain("Shopping notes");
      expect(titles[2].textContent).toContain("A plain title");
    } finally { m.dispose(); }
  });
});
