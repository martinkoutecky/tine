import { describe, it, expect } from "vitest";
import hljs from "highlight.js/lib/common";
import {
  COMMANDS,
  commandScore,
  detectTrigger,
  applyCompletion,
  refCompletionEnd,
  withRefCompletionSpace,
  autoPairEdit,
  fullWidthRefReplace,
  pageInsert,
  tagInsert,
  filterCommands,
  fuzzyScore,
  orderAcItems,
  codeLanguageItems,
  COMMON_CODE_LANGUAGES,
} from "./autocomplete";
import slashFixtureManifest from "./fixtures/slash-ranking-prefix-base-15bbddc/manifest.json";
import slashFixturePart1 from "./fixtures/slash-ranking-prefix-base-15bbddc/part-1.json";
import slashFixturePart2 from "./fixtures/slash-ranking-prefix-base-15bbddc/part-2.json";
import slashFixturePart3 from "./fixtures/slash-ranking-prefix-base-15bbddc/part-3.json";
import slashFixturePart4 from "./fixtures/slash-ranking-prefix-base-15bbddc/part-4.json";

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
  const matches = [
    { name: "Match One", item: "m1" },
    { name: "Match Two", item: "m2" },
  ];
  const create = { name: "match", item: "CREATE" };

  it("uses OG adaptive prefix ordering and puts Create immediately after the leader", () => {
    expect(orderAcItems(matches, create, { query: "match", policy: "adaptive" })).toEqual(["m1", "CREATE", "m2"]);
  });

  it("offers explicit existing and typed policies without creating an exact duplicate", () => {
    expect(orderAcItems(matches, create, { query: "match", policy: "existing" })).toEqual(["m1", "m2", "CREATE"]);
    expect(orderAcItems(matches, create, { query: "match", policy: "typed" })).toEqual(["CREATE", "m1", "m2"]);
    expect(orderAcItems([{ name: "MATCH", item: "exact" }], create, { query: "match", policy: "typed" })).toEqual(["exact"]);
  });

  it("keeps blank page/tag lifecycles row-free and deterministically orders reverse backend input", () => {
    expect(orderAcItems(matches, create, { query: "", policy: "adaptive" })).toEqual([]);
    const reversed = [
      { name: "Parity Target___Child", item: "child" },
      { name: "Parity Target", item: "target" },
      { name: "Parity Tarp", item: "tarp" },
    ];
    expect(orderAcItems(reversed, { name: "Parity Tar", item: "create" }, { query: "Parity Tar", policy: "adaptive" }))
      .toEqual(["tarp", "create", "target", "child"]);
  });

  it("uses canonical fallback for alias-backed hits whose matched alias is not exposed", () => {
    const aliasHits = [
      { name: "Zulu", item: "zulu" },
      { name: "Alpha", item: "alpha" },
    ];
    expect(orderAcItems(aliasHits, { name: "alias", item: "create" }, { query: "alias", policy: "existing" }))
      .toEqual(["zulu", "alpha", "create"]);
  });

  it("uses NFC identity without compatibility-folding fullwidth names", () => {
    const widthDistinct = [
      { name: "\uff21", item: "fullwidth" },
      { name: "Alpha", item: "alpha" },
    ];
    expect(orderAcItems(widthDistinct, { name: "a", item: "create" }, { query: "a", policy: "adaptive" }))
      .toEqual(["alpha", "create", "fullwidth"]);
  });
});

describe("detectTrigger", () => {
  it("detects language text only on opening backtick and tilde fences", () => {
    expect(detectTrigger("```j", 4)).toEqual({ kind: "code-language", query: "j", start: 3, end: 4 });
    expect(detectTrigger("~~~~py", 6)).toEqual({ kind: "code-language", query: "py", start: 4, end: 6 });
    expect(detectTrigger("```", 3)).toBeNull();
    expect(detectTrigger("```js\ncode\n```p", 16)).toBeNull();
  });

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

  it("keeps bare tag completion active for Unicode text committed by an IME (GH #167)", () => {
    // Logseq OG 6e7afa8, frontend/handler/editor.cljs `handle-last-input` and
    // `close-autocomplete-if-outside`: the marker boundary starts hashtag search,
    // and committed text is not restricted to ASCII keyboard word characters.
    for (const query of ["倘", "かな", "한글", "ไทย", "café", "🧠"]) {
      const raw = `prefix #${query}`;
      expect(detectTrigger(raw, raw.length)).toEqual({
        kind: "tag",
        query,
        start: "prefix ".length,
        end: raw.length,
      });
    }
  });

  it("tag requires start or whitespace before #", () => {
    expect(detectTrigger("email@x#y", 9)).toBeNull();
  });

  it("shares bare-tag hard stops without narrowing namespaces or punctuation inside names", () => {
    const raw = "#team/foo.bar;baz";
    expect(detectTrigger(raw, raw.length)).toEqual({
      kind: "tag", query: "team/foo.bar;baz", start: 0, end: raw.length,
    });
    for (const rawWithStop of ["#tag,", "#tag!", "#tag?", "#tag:", "#tag#"]) {
      expect(detectTrigger(rawWithStop, rawWithStop.length)).toBeNull();
    }
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

describe("codeLanguageItems", () => {
  it("stays in lockstep with the languages and aliases bundled for rendering", () => {
    expect(new Set(COMMON_CODE_LANGUAGES.map((item) => item.id))).toEqual(new Set(hljs.listLanguages()));
    for (const item of COMMON_CODE_LANGUAGES) {
      expect(new Set(item.aliases), item.id).toEqual(new Set(hljs.getLanguage(item.id)?.aliases ?? []));
    }
  });

  it("canonicalizes highlight.js aliases while keeping readable labels", () => {
    expect(codeLanguageItems("js")[0]).toMatchObject({ id: "javascript", label: "JavaScript" });
    expect(codeLanguageItems("ts")[0]).toMatchObject({ id: "typescript", label: "TypeScript" });
    expect(codeLanguageItems("html")[0]).toMatchObject({ id: "xml", label: "HTML, XML" });
  });

  it("offers a bounded full picker for the slash-command path", () => {
    const all = codeLanguageItems("");
    expect(all.length).toBeGreaterThan(20);
    expect(all.length).toBeLessThan(50);
    expect(new Set(all.map((item) => item.id)).size).toBe(all.length);
  });

  it("does not rewrite language identifiers outside the bundled highlighter", () => {
    expect(codeLanguageItems("brainfuck")).toEqual([]);
    expect(codeLanguageItems("calc")).toEqual([]);
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

describe("refCompletionEnd (OG accept-range parity)", () => {
  it("swallows the next page-ref closer when completing mid-ref", () => {
    expect(refCompletionEnd("[[first second]]", 8, "[[first]]")).toBe(16);
  });

  it("swallows a page-ref closer immediately after the caret", () => {
    expect(refCompletionEnd("[[first second]]", 14, "[[first second]]")).toBe(16);
  });

  it("leaves an unclosed page ref unchanged", () => {
    expect(refCompletionEnd("[[first", 7, "[[first]]")).toBe(7);
  });

  it("swallows the next block-ref closer when completing mid-ref", () => {
    expect(refCompletionEnd("((abcabc))", 6, "((u))")).toBe(10);
  });

  it("does not swallow a closer on the next line", () => {
    expect(refCompletionEnd("[[fir\nx]]", 5, "[[first]]")).toBe(5);
  });

  it("leaves non-ref completions unchanged", () => {
    expect(refCompletionEnd("/tod", 4, "TODO ")).toBe(4);
  });

  it("replaces the full reporter-case ref instead of leaving a stray closer", () => {
    // Necessity gate (verified pre-fix): end = 8 (the immediate-only range)
    // produces "[[first]]second]]", so this assertion fails without this helper.
    const raw = "[[first second]]";
    const insert = pageInsert("first");
    const result = applyCompletion(raw, 0, refCompletionEnd(raw, 8, insert), insert);
    expect(result.raw).toBe("[[first]]");
  });
});

describe("filterCommands", () => {
  const fixtureRows = [
    ...slashFixturePart1,
    ...slashFixturePart2,
    ...slashFixturePart3,
    ...slashFixturePart4,
  ];
  const fixtureTemplates = slashFixtureManifest.dynamicTemplates;
  const mergedRanking = (query: string): string[] => {
    const showAllTemplates = !!query && "template".startsWith(query.toLowerCase());
    return [
      ...COMMANDS.map((command) => ({
        label: command.label,
        score: commandScore(query, command),
        index: command.matchTieOrder,
      })).filter((row) => row.score > 0),
      ...fixtureTemplates.map((name, offset) => ({
        label: `Template: ${name}`,
        score: showAllTemplates ? 1 : fuzzyScore(query, name),
        index: COMMANDS.length + offset,
      })).filter((row) => row.score > 0),
    ].sort((a, b) => b.score - a.score || a.index - b.index).map((row) => row.label);
  };

  it("preserves every checked pre-fix nonempty command/template ranking", () => {
    // Generated from `git show 15bbddc:src/editor/autocomplete.ts` before the
    // bare-menu edit.  This deliberately exercises Block's merged command +
    // dynamic-template list, rather than blessing a later filterCommands-only
    // snapshot.  Never regenerate it to approve a typed-ranking change.
    expect(slashFixtureManifest.source.baseRevision).toBe("15bbddc0c5596c3fa72e84c4f3ad90c722db81a0");
    expect(fixtureRows).toHaveLength(343);
    for (const row of fixtureRows) expect(mergedRanking(row.query)).toEqual(row.labels);
  });

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

  it("uses the approved bare-slash group order independently from typed rankings", () => {
    const all = filterCommands("");
    expect(all.map((command) => command.label)).toEqual([
      "Page reference", "Link", "Upload an asset", "Voice recording", "Draw.io diagram",
      "Heading 1", "Heading 2", "Heading 3", "Heading 4", "Today", "Current time",
      "TODO", "DOING", "LATER", "NOW", "DONE", "WAITING", "WAIT", "IN-PROGRESS", "CANCELED", "Scheduled", "Deadline",
      "Priority A", "Priority B", "Priority C", "Grid", "Table", "Board", "Code block", "Calculator", "Quote",
      "Admonition: note", "Admonition: tip", "Admonition: important", "Admonition: warning", "Admonition: caution",
      "Divider", "Query", "Query (visual builder)", "Embed", "Math block", "Page properties",
      "Template var: today", "Template var: yesterday", "Template var: tomorrow", "Template var: current page", "Template var: time", "Template var: date…",
    ]);
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
