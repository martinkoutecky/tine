import { describe, it, expect } from "vitest";
import { existingBlockId, rawWithBlockId } from "./store";

describe("existingBlockId", () => {
  it("reads a markdown id:: trailer, case-insensitively", () => {
    expect(existingBlockId("text\nid:: abc-123", "md")).toBe("abc-123");
    expect(existingBlockId("text\nID:: abc-123", "md")).toBe("abc-123");
    expect(existingBlockId("text", "md")).toBeNull();
  });

  it("reads an org :PROPERTIES: drawer :id: line, not a md id::", () => {
    expect(existingBlockId("Title\n:PROPERTIES:\n:id: abc-123\n:END:", "org")).toBe("abc-123");
    expect(existingBlockId("Title\n:PROPERTIES:\n:ID: abc-123\n:END:", "org")).toBe("abc-123");
    // In org, a markdown `id::` line is plain body text — NOT the block's id (GH #25).
    expect(existingBlockId("Title\nid:: abc-123", "org")).toBeNull();
    expect(existingBlockId("Title", "org")).toBeNull();
  });
});

describe("rawWithBlockId", () => {
  it("markdown appends an id:: trailer", () => {
    expect(rawWithBlockId("text", "U", "md")).toBe("text\nid:: U");
    expect(rawWithBlockId("a\nb", "U", "md")).toBe("a\nb\nid:: U");
  });

  it("org with no drawer inserts one right after the title", () => {
    expect(rawWithBlockId("Title", "U", "org")).toBe("Title\n:PROPERTIES:\n:id: U\n:END:");
    expect(rawWithBlockId("Title\nbody", "U", "org")).toBe(
      "Title\n:PROPERTIES:\n:id: U\n:END:\nbody"
    );
  });

  it("org groups the drawer after SCHEDULED/DEADLINE planning lines (OG order)", () => {
    expect(
      rawWithBlockId("TODO t\nSCHEDULED: <2026-06-25 Thu>\nbody", "U", "org")
    ).toBe("TODO t\nSCHEDULED: <2026-06-25 Thu>\n:PROPERTIES:\n:id: U\n:END:\nbody");
    // planning lines are lifted above the drawer even when they trail the body
    expect(rawWithBlockId("TODO t\nbody\nDEADLINE: <2026-06-25 Thu>", "U", "org")).toBe(
      "TODO t\nDEADLINE: <2026-06-25 Thu>\n:PROPERTIES:\n:id: U\n:END:\nbody"
    );
  });

  it("org extends an existing drawer (inserts :id: before :END:)", () => {
    expect(rawWithBlockId("Title\n:PROPERTIES:\n:foo: bar\n:END:", "U", "org")).toBe(
      "Title\n:PROPERTIES:\n:foo: bar\n:id: U\n:END:"
    );
  });
});
