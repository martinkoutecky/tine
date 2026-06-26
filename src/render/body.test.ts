import { describe, it, expect } from "vitest";
import { segmentBody, parseList } from "./body";

describe("in-block markdown lists (+/* /ordered, NOT - which is the outline bullet)", () => {
  it("groups + list lines into a list segment, leaves - lines as plain", () => {
    expect(segmentBody(["here's a list", "+ [ ] a", "+ [x] b"])).toEqual([
      { kind: "lines", lines: ["here's a list"] },
      { kind: "list", items: ["+ [ ] a", "+ [x] b"] },
    ]);
    // `-` is the outliner's own bullet, never in-content → stays a plain line
    expect(segmentBody(["- not a list line"])).toEqual([{ kind: "lines", lines: ["- not a list line"] }]);
  });
  it("on Org pages, `-` IS a plain-list bullet and `*` is NOT (it's a headline)", () => {
    // Org plain lists use - / + (org-mode + Logseq); * at line start is a headline.
    expect(segmentBody(["- milk", "- eggs", "+ also"], "org")).toEqual([
      { kind: "list", items: ["- milk", "- eggs", "+ also"] },
    ]);
    expect(parseList(["- milk", "- eggs"], "org").items.map((i) => i.text)).toEqual(["milk", "eggs"]);
    // A leading `*` is not an in-block bullet in org.
    expect(segmentBody(["* not a bullet in org"], "org")).toEqual([
      { kind: "lines", lines: ["* not a bullet in org"] },
    ]);
    // Markdown is unchanged: `-` stays plain there.
    expect(segmentBody(["- milk"], "md")).toEqual([{ kind: "lines", lines: ["- milk"] }]);
  });
  it("parses checkboxes, ordered-ness and nesting by indent", () => {
    const n = parseList(["+ [ ] a", "+ [x] b"]);
    expect(n.ordered).toBe(false);
    expect(n.items.map((i) => i.checkbox)).toEqual(["unchecked", "checked"]);
    expect(n.items.map((i) => i.text)).toEqual(["a", "b"]);
    expect(parseList(["1. one", "2. two"]).ordered).toBe(true);
    const nested = parseList(["+ groceries", "  + milk", "  + eggs", "+ hardware"]);
    expect(nested.items.length).toBe(2);
    expect(nested.items[0].children?.items.map((i) => i.text)).toEqual(["milk", "eggs"]);
    expect(nested.items[1].children).toBe(null);
  });
});

describe("segmentBody block constructs", () => {
  it("blockquote run", () => {
    expect(segmentBody(["> a", "> b"])).toEqual([{ kind: "quote", lines: ["a", "b"] }]);
  });

  it("callout (> [!NOTE] title)", () => {
    expect(segmentBody(["> [!NOTE] Heads up", "> body"])).toEqual([
      { kind: "callout", kind2: "note", title: "Heads up", lines: ["body"] },
    ]);
  });

  it("callout with no title or body", () => {
    expect(segmentBody(["> [!WARNING]"])).toEqual([
      { kind: "callout", kind2: "warning", title: "", lines: [] },
    ]);
  });

  it("horizontal rule", () => {
    expect(segmentBody(["---"])).toEqual([{ kind: "hr" }]);
    expect(segmentBody(["***"])).toEqual([{ kind: "hr" }]);
  });

  it("table with alignment", () => {
    const segs = segmentBody(["| a | b | c |", "|:--|:-:|--:|", "| 1 | 2 | 3 |"]);
    expect(segs).toEqual([
      {
        kind: "table",
        rows: [
          ["a", "b", "c"],
          ["1", "2", "3"],
        ],
        aligns: ["left", "center", "right"],
      },
    ]);
  });

  it("quote does not swallow following plain lines", () => {
    expect(segmentBody(["> q", "plain"])).toEqual([
      { kind: "quote", lines: ["q"] },
      { kind: "lines", lines: ["plain"] },
    ]);
  });

  it("org admonition #+BEGIN_NOTE → callout (case-insensitive)", () => {
    expect(segmentBody(["#+begin_NOTE", "be careful", "#+END_note"])).toEqual([
      { kind: "callout", kind2: "note", title: "", lines: ["be careful"] },
    ]);
  });

  it("org #+BEGIN_QUOTE → blockquote, not a callout", () => {
    expect(segmentBody(["#+BEGIN_QUOTE", "a quote", "#+END_QUOTE"])).toEqual([
      { kind: "quote", lines: ["a quote"] },
    ]);
  });

  it("unterminated #+BEGIN_ falls through to plain lines", () => {
    expect(segmentBody(["#+BEGIN_NOTE", "dangling"])).toEqual([
      { kind: "lines", lines: ["#+BEGIN_NOTE", "dangling"] },
    ]);
  });
});
