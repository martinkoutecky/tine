import { describe, it, expect } from "vitest";
import {
  detectTrigger,
  applyCompletion,
  withRefCompletionSpace,
  autoPairEdit,
  fullWidthRefReplace,
  pageInsert,
  tagInsert,
  filterCommands,
  fuzzyScore,
  orderAcItems,
} from "./autocomplete";

describe("autoPairEdit (OG-style [[ ]] auto-pairing)", () => {
  // `value`/`caret` are the POST-input textarea state; `typed` is the char.
  it("auto-closes [[ → [[]] with caret left between", () => {
    // user typed the 2nd '[' so value is "[[" caret 2
    expect(autoPairEdit("[[", 2, "[")).toEqual({ value: "[[]]", caret: 2 });
  });

  it("auto-closes [[ mid-text", () => {
    // "see [[" caret 6
    expect(autoPairEdit("see [[", 6, "[")).toEqual({ value: "see [[]]", caret: 6 });
  });

  it("does NOT auto-close a lone [ (first bracket)", () => {
    expect(autoPairEdit("a[", 2, "[")).toBeNull();
  });

  it("does NOT auto-close when a ] already follows (editing inside an existing ref)", () => {
    // caret between the brackets of "[[]]" then typed '[' → "[[[]]" — guard off
    expect(autoPairEdit("[[[]]", 3, "[")).toBeNull();
  });

  it("types THROUGH a ] typed right before an existing ] (no doubling)", () => {
    // "[[Foo]]" caret 5, user types ']' → browser yields "[[Foo]]]" caret 6
    expect(autoPairEdit("[[Foo]]]", 6, "]")).toEqual({ value: "[[Foo]]", caret: 6 });
  });

  it("type-through is idempotent over a full manual ]] close", () => {
    // second ']' keystroke: "[[Foo]]" caret 6 → browser "[[Foo]]]" caret 7
    expect(autoPairEdit("[[Foo]]]", 7, "]")).toEqual({ value: "[[Foo]]", caret: 7 });
  });

  it("leaves a literal ] alone when no ] follows", () => {
    expect(autoPairEdit("a]", 2, "]")).toBeNull();
  });
});

describe("fullWidthRefReplace (Chinese IME full-width page refs)", () => {
  it("normalizes full-width double brackets to an auto-paired page ref", () => {
    expect(fullWidthRefReplace("【【", 2)).toEqual({ value: "[[]]", caret: 2 });
    expect(fullWidthRefReplace("see 【【", 6)).toEqual({ value: "see [[]]", caret: 6 });
  });

  it("ignores a lone full-width opening bracket", () => {
    expect(fullWidthRefReplace("a【", 2)).toBeNull();
  });

  it("leaves the existing ASCII [[ path to autoPairEdit", () => {
    expect(fullWidthRefReplace("[[", 2)).toBeNull();
    expect(autoPairEdit("[[", 2, "[")).toEqual({ value: "[[]]", caret: 2 });
  });
});

describe("orderAcItems (autocomplete default action)", () => {
  const matches = ["m1", "m2"];
  const create = "CREATE";

  it("OG default (linkFirst off): Create leads, matches follow", () => {
    expect(orderAcItems(matches, create, { hasQuery: true, exact: false, linkFirst: false })).toEqual([
      "CREATE",
      "m1",
      "m2",
    ]);
  });

  it("linkFirst on: first match leads, Create trails", () => {
    expect(orderAcItems(matches, create, { hasQuery: true, exact: false, linkFirst: true })).toEqual([
      "m1",
      "m2",
      "CREATE",
    ]);
  });

  it("no Create option for a blank query or an exact match (either mode)", () => {
    for (const linkFirst of [false, true]) {
      expect(orderAcItems(matches, create, { hasQuery: false, exact: false, linkFirst })).toEqual(matches);
      expect(orderAcItems(matches, create, { hasQuery: true, exact: true, linkFirst })).toEqual(matches);
    }
  });

  it("no matches → just Create (so Enter still works), either mode", () => {
    expect(orderAcItems([], create, { hasQuery: true, exact: false, linkFirst: false })).toEqual(["CREATE"]);
    expect(orderAcItems([], create, { hasQuery: true, exact: false, linkFirst: true })).toEqual(["CREATE"]);
  });
});

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

  it("detects (( block trigger", () => {
    const t = detectTrigger("see ((lemma", 11);
    expect(t).toEqual({ kind: "block", query: "lemma", start: 4, end: 11 });
  });

  it("no block trigger once parens closed", () => {
    expect(detectTrigger("see ((abcd)) more", 17)).toBeNull();
  });

  it("(( inside {{embed ...}} still fires a block trigger", () => {
    const raw = "{{embed ((foo";
    expect(detectTrigger(raw, raw.length)).toEqual({
      kind: "block", query: "foo", start: 8, end: raw.length,
    });
  });

  it("the opener closest to the caret wins (( after [[ )", () => {
    // A `((` typed after an (unclosed) `[[` on the same line — the nearer opener
    // (the block ref) is the active trigger.
    const raw = "[[Page ((blk";
    expect(detectTrigger(raw, raw.length)).toEqual({
      kind: "block", query: "blk", start: 7, end: raw.length,
    });
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
    // Exact/shorter "Query" ranks ahead of the longer "Query (visual builder)".
    expect(filterCommands("query").map((c) => c.label)).toEqual(["Query", "Query (visual builder)"]);
    // Action commands surface too.
    expect(filterCommands("scheduled").map((c) => c.label)).toEqual(["Scheduled"]);
    expect(filterCommands("upload").map((c) => c.label)).toEqual(["Upload an asset"]);
  });

  it("ranks best matches first (OG-style); /A surfaces Priority A", () => {
    // The reported bug: /A used to return LATER/WAITING/WAIT first.
    expect(filterCommands("a")[0].label).toBe("Priority A");
    expect(filterCommands("b")[0].label).toBe("Priority B");
    expect(filterCommands("c")[0].label).toBe("Priority C");
    // The label still matches, so /priority keeps working.
    expect(filterCommands("priority").map((c) => c.label)).toEqual([
      "Priority A",
      "Priority B",
      "Priority C",
    ]);
  });

  it("/kanban surfaces Board via its key alias", () => {
    expect(filterCommands("kanban")[0]?.label).toBe("Board");
    expect(filterCommands("kan")[0]?.label).toBe("Board");
  });

  it("a bare slash (empty query) lists every command in defined order", () => {
    const all = filterCommands("");
    expect(all.length).toBeGreaterThan(20);
    expect(all[0].label).toBe("TODO");
  });
});

describe("fuzzyScore", () => {
  it("a full-length exact match outranks longer partial matches", () => {
    // Why /A → "A" wins in OG: same-length match gets the max length-distance.
    expect(fuzzyScore("a", "A")).toBeGreaterThan(fuzzyScore("a", "LATER"));
    expect(fuzzyScore("a", "LATER")).toBeGreaterThan(0);
  });
  it("a contiguous substring outranks a scattered subsequence", () => {
    expect(fuzzyScore("opus", "opus tag")).toBeGreaterThan(fuzzyScore("opus", "Opinion Diffusion"));
  });
  it("non-subsequence scores 0", () => {
    expect(fuzzyScore("xyz", "Priority A")).toBe(0);
  });
});

describe("withRefCompletionSpace (GH #35)", () => {
  // Simulate a page-ref completion: `foo [[Page]]` with caret right after `]]` (11).
  const rawPage = "foo [[Page]]";
  const caretPage = rawPage.length; // 12, right after ]]

  it("inserts a space after ]] when enabled", () => {
    const r = withRefCompletionSpace(rawPage, caretPage, "[[Page]]", true);
    expect(r.raw).toBe("foo [[Page]] ");
    expect(r.caret).toBe(caretPage + 1);
  });

  it("inserts a space after )) for block refs", () => {
    const raw = "see ((abc-123))";
    const r = withRefCompletionSpace(raw, raw.length, "((abc-123))", true);
    expect(r.raw).toBe("see ((abc-123)) ");
    expect(r.caret).toBe(raw.length + 1);
  });

  it("is a no-op when disabled (OG behavior — caret stays after ]])", () => {
    const r = withRefCompletionSpace(rawPage, caretPage, "[[Page]]", false);
    expect(r.raw).toBe(rawPage);
    expect(r.caret).toBe(caretPage);
  });

  it("never doubles an existing space", () => {
    const raw = "foo [[Page]] bar";
    const r = withRefCompletionSpace(raw, 12, "[[Page]]", true);
    expect(r.raw).toBe(raw);
    expect(r.caret).toBe(12);
  });

  it("does nothing for non-ref completions (e.g. a timestamp or query)", () => {
    const raw = "at 09:30";
    const r = withRefCompletionSpace(raw, raw.length, "09:30", true);
    expect(r.raw).toBe(raw);
    expect(r.caret).toBe(raw.length);
  });

  it("mid-text insertion keeps following content after the new space", () => {
    // `[[Page]]` accepted with text after it already present.
    const raw = "a [[Page]]tail";
    const r = withRefCompletionSpace(raw, 10, "[[Page]]", true);
    expect(r.raw).toBe("a [[Page]] tail");
    expect(r.caret).toBe(11);
  });
});
