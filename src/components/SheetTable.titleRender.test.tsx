import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
import * as resultActions from "./QueryResultTask";
import * as router from "../router";
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
      { id: "r4", raw: "Mentions [[Target]] and more words", properties: [], children: [], collapsed: false },
      { id: "r5", raw: Array.from({ length: 20 }, (_, i) => `Paragraph ${i + 1}`).join("\n\n"), properties: [], children: [], collapsed: false },
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
      expect(titles).toHaveLength(5);
      expect([...titles[0].querySelectorAll("ul li")].map((li) => li.textContent)).toEqual(["apples", "pears"]);
      expect([...titles[1].querySelectorAll("ol li")].map((li) => li.textContent)).toEqual(["first", "second"]);
      expect(m.root.textContent).not.toMatch(/\* apples|1\. first/);
      expect(titles[0].textContent).toContain("Shopping notes");
      expect(titles[2].textContent).toContain("A plain title");
    } finally { m.dispose(); }
  });

  it("keeps block content out of a <button>: the open-result control is a role=button element", () => {
    const m = mountQueryTable();
    try {
      const opens = [...m.root.querySelectorAll<HTMLElement>(".query-result-open")];
      expect(opens).toHaveLength(5);
      for (const open of opens) {
        expect(open.tagName).toBe("DIV");
        expect(open.getAttribute("role")).toBe("button");
        expect(open.tabIndex).toBe(0);
      }
      expect(m.root.querySelector("button.query-result-open")).toBeNull();
      expect(m.root.querySelector(".query-result-open ul")).not.toBeNull();
    } finally { m.dispose(); }
  });

  it("a click on a ref inside a result title opens the ref only; a click elsewhere, and Enter/Space on the row, open the result", () => {
    const open = vi.spyOn(resultActions, "openQueryResult").mockImplementation(() => {});
    const openPage = vi.spyOn(router, "openPage").mockImplementation(() => {});
    const m = mountQueryTable();
    try {
      const title = m.root.querySelectorAll<HTMLElement>(".sheet-title-cell")[3];
      const ref = title.querySelector<HTMLElement>("a[data-ref]")!;
      expect(ref).not.toBeNull();
      ref.click();
      expect(openPage).toHaveBeenCalledTimes(1);
      expect(open).not.toHaveBeenCalled();

      // Plain text next to the ref is not interactive: the click opens the result.
      const row = title.querySelector<HTMLElement>(".query-result-open")!;
      row.click();
      expect(open).toHaveBeenCalledTimes(1);
      expect(openPage).toHaveBeenCalledTimes(1);

      row.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
      row.dispatchEvent(new KeyboardEvent("keydown", { key: " ", bubbles: true, cancelable: true }));
      expect(open).toHaveBeenCalledTimes(3);
      // A key pressed on the inner ref is the ref's own business, not the result's.
      ref.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
      expect(open).toHaveBeenCalledTimes(3);
    } finally { m.dispose(); open.mockRestore(); openPage.mockRestore(); }
  });

  it("a long multi-paragraph block sits inside a clamped title body, so the row height stays bounded", () => {
    const m = mountQueryTable();
    try {
      const long = m.root.querySelectorAll<HTMLElement>(".sheet-title-cell")[4];
      const body = long.querySelector<HTMLElement>(".sheet-title-body")!;
      expect(body).not.toBeNull();
      expect(body.textContent).toContain("Paragraph 1");
      // jsdom has no layout: pin the clamp in the stylesheet (the real-app journey measures the height).
      const css = readFileSync("src/styles/app.css", "utf8");
      expect(css).toMatch(/\.sheet-title-body\s*\{[^}]*max-height:\s*[\d.]+em[^}]*overflow:\s*hidden/);
    } finally { m.dispose(); }
  });
});
