import { describe, it, expect } from "vitest";
import {
  detectTrigger,
  applyCompletion,
  pageInsert,
  tagInsert,
  filterCommands,
} from "./autocomplete";

describe("detectTrigger", () => {
  it("detects [[ page trigger", () => {
    const t = detectTrigger("see [[log", 9);
    expect(t).toEqual({ kind: "page", query: "log", start: 4, end: 9 });
  });

  it("no page trigger once brackets closed", () => {
    expect(detectTrigger("see [[Page]] more", 17)).toBeNull();
  });

  it("detects # tag trigger", () => {
    const t = detectTrigger("a #pro", 6);
    expect(t).toEqual({ kind: "tag", query: "pro", start: 2, end: 6 });
  });

  it("tag requires start or whitespace before #", () => {
    expect(detectTrigger("email@x#y", 9)).toBeNull();
  });

  it("detects / command trigger at block start", () => {
    const t = detectTrigger("/que", 4);
    expect(t).toEqual({ kind: "command", query: "que", start: 0, end: 4 });
  });

  it("detects a trigger on a later line with correct absolute indices", () => {
    // The trigger is on the 2nd line; indices must be offset back into the full
    // string (the line-prefix optimization must not break absolute positions).
    const raw = "first line\nsee [[log";
    const t = detectTrigger(raw, raw.length);
    expect(t).toEqual({ kind: "page", query: "log", start: 15, end: raw.length });
    // And a # tag at the very start of a non-first line.
    const raw2 = "intro\n#pro";
    expect(detectTrigger(raw2, raw2.length)).toEqual({
      kind: "tag", query: "pro", start: 6, end: raw2.length,
    });
  });

  it("no trigger when an open [[ is on a previous line (can't span newline)", () => {
    // "[[" then a newline before the caret → not an active page trigger.
    expect(detectTrigger("a [[\nbcd", 8)).toBeNull();
  });
});

describe("applyCompletion", () => {
  it("inserts a page ref and places caret after it", () => {
    const t = detectTrigger("see [[log", 9)!;
    const insert = pageInsert("logseq-claude");
    const r = applyCompletion("see [[log", t.start, t.end, insert);
    expect(r.raw).toBe("see [[logseq-claude]]");
    expect(r.caret).toBe(r.raw.length);
  });

  it("preserves text after the caret", () => {
    const raw = "see [[log rest";
    const t = detectTrigger("see [[log", 9)!; // query stops at caret 9
    const r = applyCompletion(raw, t.start, t.end, pageInsert("Logseq"));
    expect(r.raw).toBe("see [[Logseq]] rest");
  });

  it("tag with spaces uses #[[...]]", () => {
    expect(tagInsert("multi word")).toBe("#[[multi word]]");
    expect(tagInsert("simple")).toBe("#simple");
  });
});

describe("filterCommands", () => {
  it("filters by label substring", () => {
    expect(filterCommands("head").map((c) => c.label)).toEqual([
      "Heading 1",
      "Heading 2",
      "Heading 3",
      "Heading 4",
    ]);
    expect(filterCommands("query").map((c) => c.label)).toEqual(["Query"]);
    // Action commands surface too.
    expect(filterCommands("scheduled").map((c) => c.label)).toEqual(["Scheduled"]);
    expect(filterCommands("upload").map((c) => c.label)).toEqual(["Upload an asset"]);
  });
});
