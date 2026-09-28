import { beforeAll, beforeEach, describe, expect, it } from "vitest";
import { initParser } from "../render/parse";
import { setGraphMeta } from "../graphSession";
import { beginPageHeaderEdit, indentBlock, resetStore } from "./index";
import { loadSingle } from "./workingSet";
import { pageToDto } from "./convert";
import { doc } from "./model";
import { readPageProperty, setPageProperty, setBlockProperty } from "./edits/properties";
import { rawWithBlockId, existingBlockId } from "./edits/identity";
import { joinProps } from "../editor/properties";

beforeAll(() => initParser());
beforeEach(() => { resetStore(); setGraphMeta(null); });

describe("property mutation across page formats", () => {
  it("writes an Org block's own drawer before body drawer lookalikes", () => {
    const raw = "* Heading\nSCHEDULED: <2026-09-28 Mon>\nBody\n:PROPERTIES:\n:id: body-id\n:END:";
    const own = "* Heading\nSCHEDULED: <2026-09-28 Mon>\n:PROPERTIES:\n:id: own-id\n:END:\nBody\n:PROPERTIES:\n:id: body-id\n:END:";
    expect(rawWithBlockId(raw, "own-id", "org"), "Org drawer rule: rawWithBlockId must not extend the body drawer exemplar").toBe(own);
    expect(existingBlockId(raw, "org")).toBeNull();
    expect(existingBlockId(own, "org")).toBe("own-id");
    expect(joinProps(raw, ":collapsed: true", "org")).toBe(own.replace(":id: own-id", ":collapsed: true"));
  });

  it("sets an Org property in its own drawer, leaving a body drawer alone", () => {
    const raw = "* Heading\nBody\n:PROPERTIES:\n:owner: body\n:END:";
    loadSingle({ name: "Test", kind: "page", title: "Test", pre_block: null, blocks: [{ id: "body", raw, collapsed: false, children: [] }], format: "org" });
    setBlockProperty("body", "owner", "own");
    expect(doc.byId.body.raw).toBe("* Heading\n:PROPERTIES:\n:owner: own\n:END:\nBody\n:PROPERTIES:\n:owner: body\n:END:");
  });
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
