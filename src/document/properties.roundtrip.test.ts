import { beforeAll, beforeEach, describe, expect, it } from "vitest";
import { initParser } from "../render/parse";
import { setGraphMeta } from "../graphSession";
import { beginPageHeaderEdit, indentBlock, resetStore } from "./index";
import { loadSingle } from "./workingSet";
import { pageToDto } from "./convert";
import { doc } from "./model";
import { readPageProperty, setPageProperty, setBlockProperty } from "./edits/properties";

beforeAll(() => initParser());
beforeEach(() => { resetStore(); setGraphMeta(null); });

describe("property mutation across page formats", () => {
  it("writes, reads, updates and removes Org page directives", () => {
    loadSingle({ name: "Test", kind: "page", title: "Test", pre_block: "#+TITLE: Book", blocks: [{ id: "body", raw: "* Body", collapsed: false, children: [] }], format: "org" });
    setPageProperty("Test", "klíč", "old");
    expect(pageToDto("Test")?.pre_block).toBe("#+klíč: old\n#+TITLE: Book");
    loadSingle(pageToDto("Test")!);
    expect(readPageProperty("Test", "klíč")).toBe("old");
    setPageProperty("Test", "klíč", "new");
    expect(pageToDto("Test")?.pre_block).toBe("#+klíč: new\n#+TITLE: Book");
    setPageProperty("Test", "klíč", null);
    expect(pageToDto("Test")?.pre_block).toBe("#+TITLE: Book");
  });

  it("replaces and removes a Unicode Markdown block key in place", () => {
    loadSingle({ name: "Test", kind: "page", title: "Test", pre_block: null, blocks: [{ id: "body", raw: "Body\nklíč:: old\ntags:: x", collapsed: false, children: [] }], format: "md" });
    setBlockProperty("body", "klíč", "new");
    expect(doc.byId.body.raw).toBe("Body\nklíč:: new\ntags:: x");
    setBlockProperty("body", "klíč", null);
    expect(doc.byId.body.raw).toBe("Body\ntags:: x");
  });

  it("keeps children reachable when clearing a transient Markdown header", () => {
    loadSingle({ name: "Test", kind: "page", title: "Test", pre_block: "klíč:: old", blocks: [{ id: "body", raw: "Body", collapsed: false, children: [] }], format: "md" });
    const header = beginPageHeaderEdit("Test")!;
    indentBlock("body", 0);
    expect(doc.byId[header].children).toEqual(["body"]);
    setPageProperty("Test", "klíč", null);
    expect(doc.byId[header]?.children).toEqual(["body"]);
    expect(pageToDto("Test")).toBeNull();
  });
});
