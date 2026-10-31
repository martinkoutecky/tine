import { createRoot } from "solid-js";
import { afterEach, beforeAll, expect, it } from "vitest";
import { resetStore } from "../document";
import { setDoc } from "../document/model";
import { initParser } from "./parse";
import { createQuotedSourceRanges } from "./quoteHighlight";

beforeAll(initParser);
afterEach(resetStore);

it("tracks quote creation, reanchoring and removal without reacting to comment prose", () => {
  setDoc({ loaded: true, feed: ["P"], pages: [{ name: "P", kind: "page", title: "P", preBlock: null,
    roots: ["parent"], format: "md", readOnly: false, guide: false }], byId: {
    parent: { id: "parent", raw: "parent passage tail", collapsed: false, parent: null, page: "P", children: ["child"] },
    child: { id: "child", raw: "ordinary child", collapsed: false, parent: "parent", page: "P", children: [] },
  } });
  createRoot(dispose => {
    const ranges = createQuotedSourceRanges(() => "parent", () => "md");
    expect(ranges()).toEqual([]);
    setDoc("byId", "child", "raw", "comment\nquote:: passage");
    expect(ranges()).toEqual([[9, 16, "child"]]);
    const previous = ranges();
    setDoc("byId", "child", "raw", "comment with more prose\nquote:: passage");
    expect(ranges()).toBe(previous);
    setDoc("byId", "parent", "raw", "new parent passage tail");
    expect(ranges()).toEqual([[13, 20, "child"]]);
    setDoc("byId", "parent", "children", []);
    expect(ranges()).toEqual([]);
    setDoc("byId", "parent", "children", ["child"]);
    expect(ranges()).toEqual([[13, 20, "child"]]);
    setDoc("byId", "child", "raw", "ordinary child again");
    expect(ranges()).toEqual([]);
    dispose();
  });
});
