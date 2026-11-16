import { beforeAll, beforeEach, describe, expect, it } from "vitest";
import { initParser } from "../render/parse";
import { setGraphMeta } from "../graphSession";
import { beginPageHeaderEdit, moveBlock, resetStore, setRaw } from "./index";
import { loadSingle } from "./workingSet";
import { pageToDto } from "./convert";
import { doc } from "./model";
import { readPageProperty, setPageProperty, setBlockProperty, setSchedule } from "./edits/properties";
import { rawWithBlockId, existingBlockId } from "./edits/identity";
import { BUILTIN_HIDE, joinProps, splitProps } from "../editor/properties";

beforeAll(() => initParser());
beforeEach(() => { resetStore(); setGraphMeta(null); });

describe("property mutation across page formats", () => {
  it.each(["card-repeats", "hl-page", "created-at", "custom-id", "secret"])("keeps a cleared first block's %s out of the page header", key => {
    setGraphMeta({ block_hidden_properties: ["secret"] } as Parameters<typeof setGraphMeta>[0]);
    const raw = `Question\n${key}:: opaque  `;
    loadSingle({ name: "Test", kind: "page", title: "Test", pre_block: null, blocks: [{ id: "first", raw, collapsed: false, children: [] }], format: "md" });
    const parts = splitProps(raw, BUILTIN_HIDE);
    setRaw("first", joinProps("", parts.hidden, "md", raw));
    const saved = pageToDto("Test")!;
    expect(saved.pre_block).toBeNull();
    expect(saved.blocks).toHaveLength(1);
    expect(saved.blocks[0].raw).toContain(`${key}:: opaque  `);
    loadSingle(saved);
    expect(pageToDto("Test")).toEqual(saved);
  });

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

  it("puts a property and a schedule before a titleless Markdown block's opening logbook", () => {
    // OG frontend.util.property/insert-property prepends when has-title? is false.
    const logbook = ":LOGBOOK:\nCLOCK: [2026-10-01 Wed 10:00]--[2026-10-01 Wed 10:05] =>  00:05:00\n:END:";
    loadSingle({ name: "Test", kind: "page", title: "Test", pre_block: null, blocks: [{ id: "lb", raw: logbook, collapsed: false, children: [] }], format: "md" });
    setBlockProperty("lb", "owner", "me");
    expect(doc.byId.lb.raw).toBe(`owner:: me\n${logbook}`);
    resetStore();
    loadSingle({ name: "Test", kind: "page", title: "Test", pre_block: null, blocks: [{ id: "lb", raw: logbook, collapsed: false, children: [] }], format: "md" });
    setSchedule("lb", "scheduled", { y: 2026, m: 9, d: 3 }); // m is 0-based
    expect(doc.byId.lb.raw).toBe(`SCHEDULED: <2026-10-03 Sat>\n${logbook}`);
  });
  it("replaces and removes a Unicode Markdown block key in place", () => {
    loadSingle({ name: "Test", kind: "page", title: "Test", pre_block: null, blocks: [{ id: "body", raw: "Body\nklíč:: old\ntags:: x", collapsed: false, children: [] }], format: "md" });
    setBlockProperty("body", "klíč", "new");
    expect(doc.byId.body.raw).toBe("Body\nklíč:: new\ntags:: x");
    setBlockProperty("body", "klíč", null);
    expect(doc.byId.body.raw).toBe("Body\ntags:: x");
  });

  it("keeps children reachable when clearing a transient Markdown header", async () => {
    loadSingle({ name: "Test", kind: "page", title: "Test", pre_block: "klíč:: old", blocks: [{ id: "body", raw: "Body", collapsed: false, children: [] }], format: "md" });
    const header = beginPageHeaderEdit("Test")!;
    // GH #638: Tab cannot target a transient header. A direct child move can
    // still produce an invalid draft whose content must survive property edits.
    await moveBlock("body", header, 0);
    expect(doc.byId[header].children).toEqual(["body"]);
    setPageProperty("Test", "klíč", null);
    expect(doc.byId[header]?.children).toEqual(["body"]);
    expect(pageToDto("Test")).toBeNull();
  });
});
