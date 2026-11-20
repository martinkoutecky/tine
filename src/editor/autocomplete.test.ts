import { describe, it, expect, vi, type MockInstance } from "vitest";
import hljs from "highlight.js/lib/common";
import {
  COMMANDS,
  ADVANCED_BLOCK_COMMANDS,
  advancedBlockInsertion,
  commandScore,
  detectTrigger,
  applyCompletion,
  completionReplacementEnd,
  withRefCompletionSpace,
  autoPairEdit,
  fullWidthRefReplace,
  pageInsert,
  tagInsert,
  filterCommands,
  filterAdvancedBlockCommands,
  fuzzyScore,
  orderAcItems,
  codeLanguageItems,
  COMMON_CODE_LANGUAGES,
  propertyKeyFold,
} from "./autocomplete";
import slashFixtureManifest from "./fixtures/slash-ranking-prefix-base-15bbddc/manifest.json";
import slashFixturePart1 from "./fixtures/slash-ranking-prefix-base-15bbddc/part-1.json";
import slashFixturePart2 from "./fixtures/slash-ranking-prefix-base-15bbddc/part-2.json";
import slashFixturePart3 from "./fixtures/slash-ranking-prefix-base-15bbddc/part-3.json";
import slashFixturePart4 from "./fixtures/slash-ranking-prefix-base-15bbddc/part-4.json";
import { setActiveLocale } from "../i18n";
import * as wasm from "../render/wasm/lsdoc_wasm.js";

it("keeps Unicode command input active and matches localized < labels (GH #107)", async () => {
  for (const [raw, kind, query] of [["/表", "command", "表"], ["text /待办", "command", "待办"], ["<引用", "advanced-command", "引用"], ["/e\u0301", "command", "e\u0301"]] as const) {
    expect(detectTrigger(raw, raw.length)).toMatchObject({ kind, query });
  }
  for (const raw of ["/表 ", "/表\n", "word/表", "<引用 ", "text <引用"]) expect(detectTrigger(raw, raw.length)).toBeNull();
  const english = filterAdvancedBlockCommands("qu").map(command => command.matchLabel);
  await setActiveLocale("zh-CN");
  try {
    expect(filterCommands("表").some(command => command.matchLabel === "Table")).toBe(true);
    expect(filterAdvancedBlockCommands("引用").map(command => command.matchLabel)).toContain("Quote");
    expect(filterAdvancedBlockCommands("qu").map(command => command.matchLabel)).toEqual(english);
  } finally { await setActiveLocale("en"); }
});

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
  it("normalizes block references and preserves surrounding text", () => {
    expect(fullWidthRefReplace("（（", 2)).toEqual({ value: "(())", caret: 2 });
    expect(fullWidthRefReplace("see （（ suffix", 6)).toEqual({ value: "see (()) suffix", caret: 6 });
    expect(fullWidthRefReplace("（", 1)).toBeNull();
    expect(fullWidthRefReplace("((", 2)).toBeNull();
  });

  it("normalizes the full-width angle command opener", () => {
    expect(fullWidthRefReplace("《〈", 2)).toEqual({ value: "<", caret: 1 });
    expect(fullWidthRefReplace("《", 1)).toBeNull();
    expect(fullWidthRefReplace("〈〈", 2)).toBeNull();
  });

  it("leaves full-width openers literal inside an existing matching reference", () => {
    expect(fullWidthRefReplace("((a（（b))", 5)).toBeNull();
    expect(fullWidthRefReplace("[[a【【b]]", 5)).toBeNull();
    expect(fullWidthRefReplace("((old)) （（", 10)).toEqual({ value: "((old)) (())", caret: 10 });
  });

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
  it("keeps native substring and fuzzy above expansions in every completion policy", () => {
    for (const policy of ["adaptive", "existing", "typed"] as const) {
      for (const [query, names] of [["beijing", ["x beijing", "北京"]], ["bj", ["bjmarker", "beijing", "北京", "北景"]]] as const) {
        const rows = names.map((name) => ({ name, item: name, matchClass: name === "北京" || name === "北景" ? "transliteration" as const : "fuzzy" as const }));
        const ordered = orderAcItems(rows, { name: query, item: "CREATE" }, { query, policy });
        const native = ordered.filter((name) => name !== "CREATE" && !["北京", "北景"].includes(name));
        for (const name of native) expect(ordered.indexOf(name)).toBeLessThan(ordered.indexOf("北京"));
        expect(ordered.indexOf("CREATE")).toBeLessThan(ordered.indexOf("北京"));
      }
    }
  });
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

  it("keeps prefix and substring pages after an exact CJK match without offering Create", () => {
    const cjk = [
      { name: "我是小明", item: "substring" },
      { name: "小明", item: "exact" },
      { name: "小明的家", item: "prefix" },
    ];
    const createCjk = { name: "小明", item: "CREATE" };
    for (const policy of ["adaptive", "existing", "typed"] as const) {
      expect(orderAcItems(cjk, createCjk, { query: "小明", policy }))
        .toEqual(["exact", "prefix", "substring"]);
    }
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

  it("keeps NFC identity but compatibility-folds autocomplete ranking", () => {
    const widthDistinct = [
      { name: "\uff21", item: "fullwidth" },
      { name: "Alpha", item: "alpha" },
    ];
    expect(orderAcItems(widthDistinct, { name: "a", item: "create" }, { query: "a", policy: "adaptive" }))
      .toEqual(["fullwidth", "create", "alpha"]);
    expect(orderAcItems([{ name: "Café", item: "cafe-page" }], { name: "cafe", item: "create" }, { query: "cafe", policy: "adaptive" }))
      .toEqual(["cafe-page", "create"]);
    expect(orderAcItems([{ name: "Café", item: "cafe-page" }], { name: "cafe", item: "create" }, { query: "cafe", policy: "adaptive", removeAccents: false }))
      .toEqual(["create", "cafe-page"]);
  });
});

describe("detectTrigger", () => {
  it("detects a line-start < advanced-section trigger and rejects mid-word <", () => {
    expect(detectTrigger("<qu", 3)).toEqual({
      kind: "advanced-command", query: "qu", start: 0, end: 3,
    });
    expect(detectTrigger("prose<qu", 8)).toBeNull();
  });

  it("advanced Src follows OG ->block: md fence, org section, caret on the opening line", () => {
    const src = ADVANCED_BLOCK_COMMANDS.find((c) => c.label === "Src")!;
    // OG commands.cljs:159-177 @ 6e7afa8eb: markdown Src becomes a fence.
    expect(advancedBlockInsertion(src, "md")).toEqual({ insert: "```\n\n```", caret: 3 });
    const org = advancedBlockInsertion(src, "org");
    expect(org.insert).toBe("#+BEGIN_SRC\n\n#+END_SRC");
    expect(org.caret).toBe("#+BEGIN_SRC".length); // end of opening line, not middle
    const quote = ADVANCED_BLOCK_COMMANDS.find((c) => c.label === "Quote")!;
    expect(advancedBlockInsertion(quote, "md")).toEqual({
      insert: quote.insert, caret: "#+BEGIN_QUOTE\n".length, // blank middle line
    });
  });

  it("keeps the existing command, fence, ref, and tag trigger families unchanged", () => {
    expect(detectTrigger("/que", 4)).toEqual({ kind: "command", query: "que", start: 0, end: 4 });
    expect(detectTrigger("```js", 5)).toEqual({ kind: "code-language", query: "js", start: 3, end: 5 });
    expect(detectTrigger("see [[log", 9)).toEqual({ kind: "page", query: "log", start: 4, end: 9 });
    expect(detectTrigger("a #pro", 6)).toEqual({ kind: "tag", query: "pro", start: 2, end: 6 });
  });

  it("opens property-name completion only for a fresh logical property line", () => {
    expect(detectTrigger("::", 2)).toEqual({
      kind: "property-name", query: "", start: 0, end: 2,
    });
    expect(detectTrigger("before\nAlpha_value::", 20)).toEqual({
      kind: "property-name", query: "Alpha_value", start: 7, end: 20,
    });
    expect(detectTrigger("::", 0)).toEqual({
      kind: "property-name", query: "", start: 0, end: 2,
    });
    expect(detectTrigger("before\nAlpha_value::", 18)).toEqual({
      kind: "property-name", query: "Alpha_value", start: 7, end: 20,
    });

    expect(detectTrigger("ordinary prose ::", 17)).toBeNull();
    expect(detectTrigger("[[reference]]::", 15)).toBeNull();
    expect(detectTrigger("```\n::", 6)).toBeNull();
  });

  // C3X X6 (L13): the trigger's key alphabet was ASCII while the persisted parser's is Unicode.
  it("opens property completion for a Unicode key, the same alphabet the parser stores", () => {
    expect(detectTrigger("stav_úkolu::", 12)).toEqual({ kind: "property-name", query: "stav_úkolu", start: 0, end: 12 });
    expect(detectTrigger("статус:: ", 9)).toEqual({
      kind: "property-value", query: "", start: 9, end: 9, property: "статус",
    });
    expect(detectTrigger("ordinary prose ::", 17)).toBeNull();
  });

  it("keeps any parser-accepted property's value span separate from its key and delimiter (intentional OG difference)", () => {
    expect(detectTrigger("alpha:: ", 8)).toEqual({
      kind: "property-value", query: "", start: 8, end: 8, property: "alpha",
    });
    expect(detectTrigger("alpha:: one", 11)).toEqual({
      kind: "property-value", query: "one", start: 8, end: 11, property: "alpha",
    });
    expect(detectTrigger("other:: one", 11)?.property).toBe("other");
    expect(detectTrigger("alpha:: one, two", 16)).toEqual({
      kind: "property-value", query: "two", start: 13, end: 16, property: "alpha",
      propertyValues: ["one"],
    });
  });

  it.each([
    ["", "[[", "page", "pro", "]]"],
    ["", "[[", "page", "pro", ""],
    ["other, ", "[[", "page", "pro", ""],
    ["", "((", "block", "pro", "))"],
    ["", "#", "tag", "pro", ""],
  ])("prefers reference search in a property value with prefix %j and opener %s", (prefix, opener, kind, query, closer) => {
    const before = `Body\nproject:: ${prefix}`;
    const end = before.length + opener.length + query.length;
    const raw = `${before}${opener}${query}${closer}`;
    expect(detectTrigger(raw, end)).toEqual({ kind, query, start: before.length, end,
      ...(closer ? { replaceEnd: raw.length } : {}) });
  });

  it("ends page search after the closing brackets and offers ordinary property values again", () => {
    const raw = "project:: [[Programming]] more";
    expect(detectTrigger(raw, raw.length)).toEqual({
      kind: "property-value", property: "project", query: "[[Programming]] more", start: 10, end: raw.length,
    });
  });

  it.each(["`", "``", "$", "$$"].flatMap(delimiter =>
    ["[[pro", "((pro", "#[[pro", "#pro"].map(marker => [delimiter, marker] as const),
  ))("uses property search for the %s literal containing %s", (delimiter, marker) => {
    const beforeCloser = `project:: ${delimiter}text ${marker}`;
    const raw = beforeCloser + delimiter;
    expect([beforeCloser.length, raw.length].map(caret => detectTrigger(raw, caret)?.kind))
      .toEqual(["property-value", "property-value"]);
  });

  it.each([
    "é😀 \\[[pro| text]]", "\\((pro| text))",
    "https://example.org/[[pro|", "https://example.org/((pro|",
    "<https://example.org/[[pro|>", "<https://example.org/((pro|>",
    "<https://example.org/[[pro>|", "<https://example.org/((pro>|",
    "`[[pro`| text]]",
  ])("uses property search for the literal marker in %s", marked => {
    const caret = "project:: ".length + marked.indexOf("|");
    const raw = `project:: ${marked.replace("|", "")}`;
    expect(detectTrigger(raw, caret)).toMatchObject({ kind: "property-value", property: "project" });
  });

  it.each(["\\\\", "\\* "])("still searches a genuine reference after %j", prefix => {
    for (const [marker, closer, kind] of [["[[", "]]", "page"], ["((", "))", "block"]]) {
      const before = `project:: ${prefix}${marker}pro`;
      expect(detectTrigger(before + closer, before.length)).toMatchObject({ kind, query: "pro" });
    }
  });

  it.each([
    "Body\n``text\n[[pro|``", "Body\n``text ((pro|\nend``", "Body\n$$\ntext #pro|\n$$",
  ])("keeps reference search out of the parser's multiline literal in %j", marked => {
    const caret = marked.indexOf("|");
    expect(detectTrigger(marked.replace("|", ""), caret)).toBeNull();
  });

  it.each([["~[[pro~", 6], ["=x #pro=", 7], ["\\[[pro]]", 6], ["\\((pro))", 6]])(
    "keeps reference search out of the Org literal %j", (raw, caret) => {
      expect(detectTrigger(raw, caret, "org")).toBeNull();
    },
  );

  it.each(["[[", "(("])("keeps an escaped unfinished Org %s marker out of the picker", marker => {
    const escaped = `\\${marker}pro`;
    const active = `\\${escaped}`;
    expect(detectTrigger(escaped, escaped.length, "org")).toBeNull();
    expect(detectTrigger(active, active.length, "org")).toMatchObject({ query: "pro", start: 2 });
  });

  it.each(["[[Math ((notation]]", "#[[Math ((notation]]", "((alpha [[beta))", "((text #inner))"])(
    "resumes property values after the closed reference %s",
    value => {
      const raw = `project:: ${value}`;
      expect(detectTrigger(raw, raw.length)).toMatchObject({ kind: "property-value", property: "project", query: value });
    },
  );

  it("still searches a fresh reference after a closed one with an inner marker", () => {
    const raw = "project:: [[Math ((notation]], ((new";
    expect(detectTrigger(raw, raw.length)).toMatchObject({ kind: "block", query: "new" });
  });

  it.each(["[[a ((b]]", "#[[a ((b]]", "((a [[b))"])(
    "searches a fresh tag after %s", value => {
      const raw = `project:: ${value} #fresh`;
      expect(detectTrigger(raw, raw.length)).toMatchObject({ kind: "tag", query: "fresh" });
    },
  );

  it.each([
    "{{embed [[a ((b]]|}}",
    "{{embed [[a ((b]]}}|",
    "[label]([[a ((b]]|)",
    "[label](((a [[b))|)",
    "[label]([[a ((b]])|",
  ])("resumes property values after leaving the reference in %s", marked => {
    const raw = `project:: ${marked.replace("|", "")}`;
    const caret = "project:: ".length + marked.indexOf("|");
    expect(detectTrigger(raw, caret)).toMatchObject({ kind: "property-value", property: "project" });
  });

  it("returns to the enclosing page reference after leaving a nested child", () => {
    const raw = "project:: [[outer [[inner ((b]] tail]]";
    const caret = raw.indexOf("]]") + 2;
    expect(detectTrigger(raw, caret)).toMatchObject({ kind: "page", query: "outer [[inner ((b]] tail", start: "project:: ".length });
  });

  it("excludes parser-rejected property lookalikes and Org drawers from value search", () => {
    for (const raw of ["prose alpha:: ", "alpha::one", "  alpha:: one", "[[alpha]]:: one", "```\nalpha:: one", "#alpha:: one"]) {
      expect(detectTrigger(raw, raw.length)?.kind).not.toBe("property-value");
    }
    const org = ":PROPERTIES:\n:alpha: one\n:END:";
    expect(detectTrigger(org, org.indexOf("one") + 3, "org")).toBeNull();
    const raw = "Body\nстатус:: один, д";
    expect(detectTrigger(raw, raw.length)).toMatchObject({ kind: "property-value", property: "статус", query: "д", propertyValues: ["один"] });
    expect(detectTrigger("alpha:: ", 3)?.kind).not.toBe("property-value");
  });

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

describe("propertyKeyFold", () => {
  it("uses the established property identity for new keys", () => {
    expect(propertyKeyFold("  Alpha value_name  ")).toBe("alpha-value-name");
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

describe("completion replacement (OG accept-range parity)", () => {
  function expectLineParses(inline: MockInstance<typeof wasm.parse_inline_json>, line: string, titleStart: number) {
    expect(inline.mock.calls.map(([source]) => Buffer.byteLength(source)))
      .toEqual([Buffer.byteLength(line), Buffer.byteLength(line.slice(titleStart))]);
  }

  it.each(["[[", "#[["])("bounds the %s pick to two inline parses of the caret line", opener => {
    const before = `project:: ${opener}pro`;
    const references = `${before}, ${opener}Existing]]`;
    const tail = "x".repeat(1024 * 1024);
    const inline = vi.spyOn(wasm, "parse_inline_json");
    try {
      for (const separator of [" ", "\n", "\r\n"]) {
        inline.mockClear();
        const raw = references + separator + tail;
        expect(detectTrigger(raw, before.length)).toEqual({
          kind: "page", query: "pro", start: before.length - "[[pro".length, end: before.length,
        });
        expectLineParses(inline, separator === " " ? raw : references, before.length - "pro".length);
      }
    } finally {
      inline.mockRestore();
    }
  });

  it.each([
    ["`prior` [[pro", " title]]", "`later`", "pro title"],
    ["[[pro", " gr_am]]", "end_", "pro gr_am"],
  ])("parses the full line around %s%s with a later delimiter", (before, after, later, query) => {
    const raw = before + after + " x".repeat(512 * 1024) + " " + later;
    const inline = vi.spyOn(wasm, "parse_inline_json");
    try {
      expect(detectTrigger(raw, before.length)).toMatchObject({ kind: "page", query, replaceEnd: before.length + after.length });
      expectLineParses(inline, raw, before.length - "pro".length);
    } finally {
      inline.mockRestore();
    }
  });

  describe.each(["md", "org"] as const)("PICK-853c in %s", format => {
    it.each([
      ["((", " https://x.test/(keep)) tail"],
      ["((", " http://x.test/(keep)) tail"],
      ["((", " ftp://x.test/(keep)) tail"],
      ["((", " https://x.test/?q=(keep)) tail"],
      ["((", " https://x.test/#(keep)) tail"],
      ["((", " https://x.test/[keep))] tail"],
      ["((", " <https://x.test/(keep))> tail"],
      ["((", " #keep))suffix"],
      ["[[", " [x] #tag]], [[Existing]]"],
      ["[[", "ramming #x]]"],
      ["#[[", "ramming #x]]"],
      ["[[", " **Keep**]]"],
      ...["[[", "#[[", "(("].flatMap(opener => {
        const closer = opener === "((" ? "))" : "]]";
        return [
          ` x^{Keep${closer} intact}`,
          ...(format === "md" ? [` x_{Keep${closer} intact}`] : []),
          ...(format === "md" ? ["*", "_", "~~"] : ["*", "**", "/", "+"]).map(delimiter =>
            ` ${delimiter}Keep ${closer} a\\${delimiter} b${delimiter}`),
        ].map(suffix => [opener, suffix]);
      }),
      ...(format === "org" ? [
        ["[[", " /Keep ]] intact/"],
        ["#[[", " /Keep ]] intact/"],
        ["((", " /Keep )) intact/"],
      ] : []),
    ])("preserves markup after %spro before %s", (opener, suffix) => {
      for (const newline of ["\n", "\r\n"]) {
        const start = `Body${newline}project:: ${opener === "#[[" ? "#" : ""}`.length;
        const prefix = `Body${newline}project:: ${opener}pro`;
        const caret = prefix.length;
        const raw = prefix + suffix + newline + "later";
        const kind = opener === "((" ? "block" : "page";
        const insert = kind === "block" ? "((programming-id))" : "[[Programming]]";
        const trigger = detectTrigger(raw, caret, format)!;
        expect(trigger).toEqual({ kind, query: "pro", start, end: caret });
        expect(applyCompletion(raw, start, completionReplacementEnd(raw, trigger, insert), insert).raw)
          .toBe(raw.slice(0, start) + insert + suffix + newline + "later");
      }
    });

    it.each([
      ["[[pro", " gr_am]]", "a/b ", "pro gr_am"],
      ["[[pro", " [x] title]]", "a[bc] ", "pro [x] title"],
    ])("bounds parser calls and bytes for %s%s before a 1 MiB %j tail", (prefix, title, pattern, query) => {
      const inline = vi.spyOn(wasm, "parse_inline_json");
      const block = vi.spyOn(wasm, "parse_block_bundle_json");
      try {
        const before = `project:: ${prefix}`;
        const reference = before + title;
        const tail = pattern.repeat(Math.ceil(1024 * 1024 / pattern.length)).slice(0, 1024 * 1024);
        // Org reads `_am` as a subscript.
        const plain = format !== "org" || !title.includes("_");
        for (const separator of [" ", "\n", "\r\n"]) {
          inline.mockClear();
          block.mockClear();
          expect(detectTrigger(reference + separator + tail, before.length, format)).toEqual({
            kind: "page", query: plain ? query : "pro", start: "project:: ".length, end: before.length,
            ...(plain ? { replaceEnd: reference.length } : {}),
          });
          expect(block).not.toHaveBeenCalled();
          expectLineParses(inline, separator === " " ? reference + separator + tail : reference, before.length - "pro".length);
        }
      } finally {
        block.mockRestore();
        inline.mockRestore();
      }
    });

    it.each([
      ["((Alpha [[be|ta))", "block", "Alpha [[be", true] as const,
      ...["[[", "#[["].flatMap(opener => [
        [`${opener}Math ((not|ation]]`, "page", "Math ((notation", true],
        [`${opener}Math ((not|ation)) tail]]`, "page", "Math ((not", false],
        [`${opener}Math ((nota)) ti|tle]]`, "page", "Math ((nota)) title", true],
      ] as const),
    ])("uses the enclosing reference at %s", (marked, kind, query, plain) => {
      for (const newline of ["\n", "\r\n"]) {
        const prefix = `Body${newline}project:: `;
        const reference = marked.replace("|", "");
        const start = prefix.length + Number(marked.startsWith("#"));
        const caret = prefix.length + marked.indexOf("|");
        const replaceEnd = prefix.length + reference.length;
        const raw = prefix + reference + ", rest";
        const trigger = detectTrigger(raw, caret, format)!;
        expect(trigger).toEqual({ kind, query, start, end: caret, ...(plain ? { replaceEnd } : {}) });
        const insert = kind === "page" ? "[[Math]]" : "((target))";
        expect(applyCompletion(raw, start, completionReplacementEnd(raw, trigger, insert), insert).raw)
          .toBe(raw.slice(0, start) + insert + raw.slice(plain ? replaceEnd : caret));
      }
    });

    it.each([["[[Math ((not", "ation]]", "))", "page"], ["((Alpha [[be", "ta))", "]]", "block"]])(
      "keeps the text through a distant closer in %s%s", (before, after, laterCloser, kind) => {
        const raw = before + after + " x".repeat(512 * 1024) + laterCloser;
        const inline = vi.spyOn(wasm, "parse_inline_json");
        try {
          expect(detectTrigger(raw, before.length, format)).toEqual({ kind, query: before.slice(2), start: 0, end: before.length });
          expectLineParses(inline, raw, 2);
        } finally {
          inline.mockRestore();
        }
      },
    );

    it.each(["`[[`", "~[[~", "=[[=", "$[[$", "[[old\\]]"].flatMap(prefix =>
      [" b", ""].map(tail => [prefix, tail]),
    ))("finds the block reference after %s with %j before its closer (F4)", (prefix, tail) => {
      for (const newline of ["\n", "\r\n"]) {
        const before = `Body${newline}project:: ${prefix} ((a\\[[b`;
        const suffix = `a\\]]${tail}))`;
        const raw = before + suffix;
        const trigger = detectTrigger(raw, before.length, format)!;
        const start = before.lastIndexOf("((");
        expect(trigger).toEqual({ kind: "block", query: "a\\[[b", start, end: before.length });
        const insert = "((target))";
        expect(applyCompletion(raw, start, completionReplacementEnd(raw, trigger, insert), insert).raw)
          .toBe(raw.slice(0, start) + insert + suffix);
      }
    });

    it.each(["[[pro", "#[[pro", "((pro", "#pro"])("ignores %s on the opening line of multiline math", marker => {
      for (const newline of ["\n", "\r\n"]) {
        const before = `$$ text ${marker}`;
        expect(detectTrigger(`${before}${newline}more${newline}$$`, before.length, format)).toBeNull();
      }
    });

    it.each([
      ["1 < 2, [[", " title]]", ">", "page", "pro title"],
      ["a $ [[", " title]]", "$", "page", "pro title"],
      ...["*", "_", "$"].flatMap(delimiter => ["[[", "#[[", "(("].map(opener => [
        `project:: ${opener}`, ` ${delimiter} title${opener === "((" ? "))" : "]]"}`, delimiter,
        opener === "((" ? "block" : "page", opener === "((" ? "pro" : `pro ${delimiter} title`,
      ])),
    ])("bounds input after %spro%s with distant %s (F5)", (prefix, suffix, delimiter, kind, query) => {
      const before = prefix + "pro";
      const reference = before + suffix;
      const raw = `${reference}${" x".repeat(512 * 1024)} ${delimiter}`;
      const inline = vi.spyOn(wasm, "parse_inline_json");
      const block = vi.spyOn(wasm, "parse_block_bundle_json");
      try {
        expect(detectTrigger(raw, before.length, format)).toMatchObject({ kind, query, replaceEnd: reference.length });
        expectLineParses(inline, raw, prefix.length);
        expect(block).not.toHaveBeenCalled();
      } finally {
        block.mockRestore();
        inline.mockRestore();
      }
    });

    it.each(format === "md" ? ["$$", "``"] : ["$$"])("skips block parsing after closed %s on the first line (F5)", delimiter => {
      const inline = vi.spyOn(wasm, "parse_inline_json");
      const block = vi.spyOn(wasm, "parse_block_bundle_json");
      try {
        const prefix = `${delimiter}x${delimiter} [[`;
        const before = prefix + "pro";
        const line = before + " title]]";
        const tail = " x".repeat(512 * 1024) + " " + delimiter;
        for (const newline of ["\n", "\r\n"]) {
          inline.mockClear();
          expect(detectTrigger(line + newline + tail, before.length, format))
            .toMatchObject({ kind: "page", query: "pro title", replaceEnd: line.length });
          expectLineParses(inline, line, prefix.length);
          expect(block).not.toHaveBeenCalled();
        }
      } finally {
        block.mockRestore();
        inline.mockRestore();
      }
    });

    it.each([80_000, 160_000])("bounds input before %i right parentheses (F6)", count => {
      const inline = vi.spyOn(wasm, "parse_inline_json");
      try {
        const title = "pro" + ")".repeat(count);
        const line = `[[${title}]]`;
        expect(detectTrigger(line, 5, format)).toEqual({ kind: "page", query: title, start: 0, end: 5, replaceEnd: line.length });
        expectLineParses(inline, line, 2);
      } finally {
        inline.mockRestore();
      }
    });

    it.each([...(format === "md" ? ["`[[` ", "``[[`` "] : ["~[[~ ", "=[[= "]),
      "$x[[y$ ", "$$x[[y$$ ", "\\(x[[y\\) ", "<https://x.test/[[> ",
    ])("bounds input after 4,000 %s spans (F7)", literal => {
      const inline = vi.spyOn(wasm, "parse_inline_json");
      try {
        const prefix = literal.repeat(4_000);
        const line = prefix + "[[pro title]]";
        expect(detectTrigger(line, prefix.length + 5, format))
          .toMatchObject({ kind: "page", query: "pro title", start: prefix.length, replaceEnd: line.length });
        expectLineParses(inline, line, prefix.length + 2);
      } finally {
        inline.mockRestore();
      }
    });
  });

  it("uses only the authored line for an unfinished reference", () => {
    const inline = vi.spyOn(wasm, "parse_inline_json");
    try {
      const line = "project:: [[é😀 suffix";
      expect(detectTrigger(line, "project:: [[é😀".length)).toMatchObject({ kind: "page", query: "é😀" });
      expect(inline.mock.calls.map(([source]) => Buffer.byteLength(source))).toEqual([Buffer.byteLength(line)]);
    } finally {
      inline.mockRestore();
    }
  });

  it.each(["md", "org"] as const)("consults block regions for %s multiline math on its opening and interior lines", format => {
    const block = vi.spyOn(wasm, "parse_block_bundle_json");
    try {
      for (const newline of ["\n", "\r\n"]) {
        const before = "$$ opening [[pro";
        const opening = before + newline + "more" + newline + "$$";
        const inside = `Body${newline}$$${newline}interior #pro`;
        const interior = inside + newline + "$$";
        block.mockClear();
        expect(detectTrigger(before, before.length, format)).toMatchObject({ kind: "page", query: "pro" });
        expect(block).not.toHaveBeenCalled();
        expect(detectTrigger(opening, before.length, format)).toBeNull();
        expect(detectTrigger(interior, inside.length, format)).toBeNull();
        expect(detectTrigger(interior, inside.length, format)).toBeNull();
        expect(block.mock.calls.map(([source]) => Buffer.byteLength(source)))
          .toEqual([Buffer.byteLength(opening), Buffer.byteLength(interior)]);
      }
    } finally {
      block.mockRestore();
    }
  });

  it.each(["[keep](https://x.test/DEST/abc)", "![keep](https://x.test/DEST/abc)",
    "[keep](<https://x.test/DEST/abc>)", "[keep](relative/DEST/abc)",
    ...["https://x.test", "relative"].flatMap(base => ["", "!"].map(image => `${image}[keep](${base}/a<z/DEST/abc)`)),
    "[keep](<relative/a<z/DEST/abc>)"])(
    "uses the format's parser for Markdown-style destination %s", link => {
      for (const [marker, kind] of [["[[pro|]]", "page"], ["[[pro|", "page"], ["#[[pro|]]", "page"],
        ["#[[pro|", "page"], ["((pro|))", "block"], ["((pro|", "block"], [" #pro|", "tag"]]) {
        for (const newline of ["\n", "\r\n"]) {
          const marked = `Body${newline}project:: ${link.replace("DEST", marker)}${newline}later`;
          const raw = marked.replace("|", "");
          expect(detectTrigger(raw, marked.indexOf("|"), "md"))
            .toMatchObject({ kind: "property-value", property: "project" });
          expect(detectTrigger(raw, marked.indexOf("|"), "org")).toMatchObject({ kind });
        }
      }
    },
  );

  it.each(["[[pro|", "#[[pro|", "((pro|))", " #pro|"])("ignores %s in a native Org link destination", marker => {
    for (const newline of ["\n", "\r\n"]) {
      const org = `Body${newline}[[https://example.org/${marker}/abc][keep]]`;
      expect(detectTrigger(org.replace("|", ""), org.indexOf("|"), "org")).toBeNull();
    }
  });

  it.each([["{{embed [[pro|}}", "page"], ["{{embed ((pro|))}}", "block"], ["[label]([[pro|]])", "page"]])(
    "keeps the reference picker available when authoring %s", (marked, kind) => {
      expect(detectTrigger(marked.replace("|", ""), marked.indexOf("|"))).toMatchObject({ kind, query: "pro" });
    },
  );

  it("retires an unfinished embed reference after leaving the argument", () => {
    const raw = "{{embed [[pro}}";
    expect(detectTrigger(raw, raw.length)).toBeNull();
  });

  it("keeps the wrapper when picking a block reference used as a Markdown link destination", () => {
    const raw = "[label](((old id)))";
    const trigger = detectTrigger(raw, raw.indexOf(" id"))!;
    expect(trigger).toMatchObject({ kind: "block", query: "old" });
    const insert = "((selected))";
    expect(applyCompletion(raw, trigger.start, completionReplacementEnd(raw, trigger, insert), insert).raw)
      .toBe("[label](((selected)))");
  });

  it.each([
    ["project:: [[pro|, [[Existing]]", "[[Programming]]", "page"],
    ["project:: [[pro|, Other, [[Existing]]", "[[Programming]]", "page"],
    ["project:: #[[pro|, #[[Existing]]", "[[Programming]]", "page"],
    ["project:: ((pro|, ((existing))", "((programming-id))", "block"],
    ["project:: ((pro|, [[Keep )) intact]]", "((programming-id))", "block"],
    ["project:: ((pro|, #[[Keep )) intact]]", "((programming-id))", "block"],
    ["project:: ((pro|, **[[Keep )) intact]]**", "((programming-id))", "block"],
    ["project:: [[pro|, ((Keep ]] intact))", "[[Programming]]", "page"],
    ["project:: #pro|, [[Existing]]", "#[[Programming notes]]", "tag"],
    ["project:: #pro|, #[[Existing]]", "#[[Programming notes]]", "tag"],
    ["See [[pro|, [[Existing]]", "[[Programming]]", "page"],
  ])("preserves later references when accepting in %s", (marked, insert, kind) => {
    const caret = marked.indexOf("|");
    const suffix = marked.slice(caret + 1);
    const raw = marked.slice(0, caret) + suffix;
    const trigger = detectTrigger(raw, caret)!;
    const start = caret - (kind === "tag" ? "#pro".length : "[[pro".length);
    expect(trigger).toMatchObject({ kind, query: "pro", start });
    const end = completionReplacementEnd(raw, trigger, insert);
    expect(applyCompletion(raw, trigger.start, end, insert).raw).toBe(raw.slice(0, start) + insert + suffix);
  });

  it.each([
    ["[[", " ((Keep ]] intact))", "[[Programming]]"],
    ["#[[", " ((Keep ]] intact))", "[[Programming]]"],
    ["[[", "\t((Keep ]] intact))", "[[Programming]]"],
    ["[[", "; ((Keep ]] intact))", "[[Programming]]"],
    ["[[", " ((Keep ]] intact))\nmore", "[[Programming]]"],
    ["[[", " ((Keep ]] intact))\r\nmore", "[[Programming]]"],
    ["[[", " [keep](((Keep ]] intact)))", "[[Programming]]"],
    ["[[", " {{embed [[Keep]]}}", "[[Programming]]"],
    ["[[", " {{embed #[[Keep]]}}", "[[Programming]]"],
    ["[[", " {{embed ((Keep ]] intact))}}", "[[Programming]]"],
    ["[[", " {{embed Keep ]] intact}}", "[[Programming]]"],
    ["[[", ` {{embed [[Keep]]${"x".repeat(64)}}}`, "[[Programming]]"],
    ["[[", " `Keep ]] intact`", "[[Programming]]"],
    ["[[", " $Keep ]] intact$", "[[Programming]]"],
    ["[[", " <https://example.org/Keep]]/intact>", "[[Programming]]"],
    ["[[", " after the cursor", "[[Programming]]"],
    ["[[", "", "[[Programming]]"],
    ["((", " {{embed ((Keep))}}", "((programming-id))"],
    ["((", " after the cursor", "((programming-id))"],
    ["[[", "]] intact`", "[[Programming]]", "pro `Keep "],
    ["((", ")) intact$", "((programming-id))", "pro $Keep "],
  ])("finishes the unfinished %s at the caret before %j (FORK-853b)", (opener, suffix, insert, query = "pro") => {
    const prefix = `project:: ${opener}${query}`;
    const raw = prefix + suffix;
    const caret = prefix.length;
    const start = caret - query.length - 2;
    const trigger = detectTrigger(raw, caret)!;
    const end = completionReplacementEnd(raw, trigger, insert);
    expect(applyCompletion(raw, start, end, insert)).toEqual({
      raw: raw.slice(0, start) + insert + suffix, caret: start + insert.length,
    });
    expect(trigger).toEqual({ kind: opener === "((" ? "block" : "page", query, start, end: caret });
  });

  it("keeps the page picker active after a single bracket inside the name", () => {
    const raw = "project:: [[pro [x] title]]";
    expect(detectTrigger(raw, raw.indexOf(" title"))).toMatchObject({
      kind: "page", query: "pro [x] title", replaceEnd: raw.length,
    });
  });

  it.each([
    " [x] title",
    " [ title",
    " \\]] title",
  ])("ranks and replaces the same whole page reference containing %s", suffix => {
    const prefix = "project:: [[pro";
    const raw = `${prefix}${suffix}]], [[Existing]]`;
    const trigger = detectTrigger(raw, prefix.length)!;
    expect(trigger).toMatchObject({ kind: "page", query: `pro${suffix}` });
    const end = completionReplacementEnd(raw, trigger, "[[Programming]]");
    expect(applyCompletion(raw, trigger.start, end, "[[Programming]]").raw).toBe("project:: [[Programming]], [[Existing]]");
  });

  it.each([" [x] #tag", " #[[child]] title", " [[child]] title"])(
    "ranks the prefix and keeps markup in the page reference containing %s (PICK-853c)", suffix => {
      const prefix = "project:: [[pro";
      const following = `${suffix}]], [[Existing]]`;
      const raw = prefix + following;
      const trigger = detectTrigger(raw, prefix.length)!;
      expect(trigger).toEqual({ kind: "page", query: "pro", start: "project:: ".length, end: prefix.length });
      const insert = "[[Programming]]";
      expect(applyCompletion(raw, trigger.start, completionReplacementEnd(raw, trigger, insert), insert).raw)
        .toBe(`project:: ${insert}${following}`);
    },
  );

  it("ranks and replaces the whole bracketed tag without changing its marker", () => {
    const raw = "project:: #[[pro [x] title]], [[Existing]]";
    const caret = raw.indexOf(" [x]");
    const trigger = detectTrigger(raw, caret)!;
    expect(trigger).toMatchObject({ kind: "page", query: "pro [x] title" });
    const insert = "[[Programming]]";
    expect(applyCompletion(raw, trigger.start, completionReplacementEnd(raw, trigger, insert), insert).raw)
      .toBe("project:: #[[Programming]], [[Existing]]");
  });

  it("keeps the pre-#853 slash macro boundary when another opener precedes the closer", () => {
    const raw = "/embed {{old}}";
    expect(completionReplacementEnd(raw, detectTrigger(raw, 6)!, "{{embed [[]]}}")).toBe(raw.length);
  });

  it("queries and replaces an entire property page reference whose name contains a comma", () => {
    const raw = "project:: [[Sat, 07.11.2026]], [[Existing]]";
    const trigger = detectTrigger(raw, raw.indexOf(",") + 1)!;
    expect(trigger).toMatchObject({ kind: "page", query: "Sat, 07.11.2026" });
    const insert = "[[Saturday]]";
    const end = completionReplacementEnd(raw, trigger, insert);
    expect(applyCompletion(raw, trigger.start, end, insert).raw).toBe("project:: [[Saturday]], [[Existing]]");
  });

  it("swallows the next page-ref closer when completing mid-ref", () => {
    const raw = "[[first second]]";
    expect(completionReplacementEnd(raw, detectTrigger(raw, 8)!, "[[first]]")).toBe(16);
  });

  it("swallows a page-ref closer immediately after the caret", () => {
    const raw = "[[first second]]";
    expect(completionReplacementEnd(raw, detectTrigger(raw, 14)!, "[[first second]]")).toBe(16);
  });

  it("leaves an unclosed page ref unchanged", () => {
    const raw = "[[first";
    expect(completionReplacementEnd(raw, detectTrigger(raw, 7)!, "[[first]]")).toBe(7);
  });

  it("swallows the next block-ref closer when completing mid-ref", () => {
    const raw = "((abcabc))";
    expect(completionReplacementEnd(raw, detectTrigger(raw, 6)!, "((u))")).toBe(10);
  });

  it("does not swallow a closer on the next line", () => {
    const raw = "[[fir\nx]]";
    expect(completionReplacementEnd(raw, detectTrigger(raw, 5)!, "[[first]]")).toBe(5);
  });

  it("leaves non-ref completions unchanged", () => {
    const raw = "/tod";
    expect(completionReplacementEnd(raw, detectTrigger(raw, 4)!, "TODO ")).toBe(4);
  });

  it("replaces the full reporter-case ref instead of leaving a stray closer", () => {
    // Necessity gate (verified pre-fix): end = 8 (the immediate-only range)
    // produces "[[first]]second]]", so this assertion fails without this helper.
    const raw = "[[first second]]";
    const insert = pageInsert("first");
    const result = applyCompletion(raw, 0, completionReplacementEnd(raw, detectTrigger(raw, 8)!, insert), insert);
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
      // The fixture predates this deliberately additive command; keep using it
      // to freeze all of the old command/template rankings.
      ...COMMANDS.filter((command) => command.label !== "Heading (Auto)" && command.label !== "Embed Youtube timestamp" && command.label !== "That day" && !["Tomorrow", "Yesterday", "Date picker"].includes(command.label)).map((command) => ({
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
    // §7.3 unified the two query commands into one (master 2617ff194): the
    // retired label is dropped from the expectation, never regenerated.
    for (const row of fixtureRows) {
      expect(mergedRanking(row.query)).toEqual(row.labels.filter((label) => label !== "Query (visual builder)"));
    }
  });

  it("filters by label substring", () => {
    expect(filterCommands("head").map((c) => c.label)).toEqual([
      "Heading 1",
      "Heading 2",
      "Heading 3",
      "Heading 4",
      "Heading (Auto)",
    ]);
    // §7.3: there is ONE query command, and it opens the block in the builder.
    expect(filterCommands("query").map((c) => c.label)).toEqual(["Query"]);
    expect(COMMANDS.find((command) => command.label === "Query")?.action).toBe("query-builder");
    expect(COMMANDS.filter((command) => command.label.startsWith("Query"))).toHaveLength(1);
    // Action commands surface too.
    expect(filterCommands("scheduled").map((c) => c.label)).toEqual(["Scheduled"]);
    expect(filterCommands("upload").map((c) => c.label)).toEqual(["Upload an asset"]);
    expect(filterCommands("youtube").map((c) => c.label)).toEqual(["Embed Youtube timestamp"]);
  });

  it("routes automatic and explicit heading slash commands through heading actions", () => {
    expect(COMMANDS.find((command) => command.label === "Heading (Auto)")?.action).toBe("heading-auto");
    expect(COMMANDS.find((command) => command.label === "Heading 1")?.action).toBe("heading-1");
    expect(COMMANDS.find((command) => command.label === "Heading 1")?.insert).toBeUndefined();
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
      "Heading (Auto)", "Heading 1", "Heading 2", "Heading 3", "Heading 4", "Today", "Tomorrow", "Yesterday", "Date picker", "That day", "Current time",
      "TODO", "DOING", "LATER", "NOW", "DONE", "WAITING", "WAIT", "IN-PROGRESS", "CANCELED", "Scheduled", "Deadline",
      "Priority A", "Priority B", "Priority C", "Grid", "Table", "Board", "Code block", "Calculator", "Quote",
      "Admonition: note", "Admonition: tip", "Admonition: important", "Admonition: warning", "Admonition: caution",
      "Divider", "Query", "Embed", "Embed Youtube timestamp", "Math block", "Page properties",
      "Template var: today", "Template var: yesterday", "Template var: tomorrow", "Template var: current page", "Template var: time", "Template var: date…",
    ]);
  });
});

describe("filterAdvancedBlockCommands", () => {
  it("keeps OG's Quote and Query matches for <qu", () => {
    expect(filterAdvancedBlockCommands("qu").map((command) => command.label)).toEqual(["Quote", "Query"]);
    expect(ADVANCED_BLOCK_COMMANDS.map((command) => command.label)).toContain("Comment");
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

// GH #888 (D-20261109-LK): inside a complete link the page query is the whole
// link text. A deliberate difference from Logseq, which searches `[[` to the
// caret (components/editor.cljs:105-121 page-search, 6e7afa8eb).
describe("detectTrigger inside a complete link (GH #888)", () => {
  it("queries the whole link text, through the closer a pick replaces", () => {
    const raw = "see [[Sat, 07.11.2026]] later";
    const caret = raw.indexOf(",") + 1;
    expect(detectTrigger(raw, caret)).toEqual({ kind: "page", query: "Sat, 07.11.2026", start: 4, end: caret, replaceEnd: 23 });
    // A tag's bracketed form shares the page path.
    expect(detectTrigger("a #[[multi word]]", 11)).toMatchObject({ kind: "page", query: "multi word" });
  });

  it("leaves a link being typed as it was", () => {
    expect(detectTrigger("[[Sat]]", 5)).toMatchObject({ kind: "page", query: "Sat" }); // ]] auto-paired after the caret
    expect(detectTrigger("[[Sat", 5)).toMatchObject({ kind: "page", query: "Sat" }); // no ]] yet
    expect(detectTrigger("[[Sa and [[b]]", 4)).toMatchObject({ kind: "page", query: "Sa" }); // a later link is not this one
    expect(detectTrigger("[[Sa\nt]]", 4)).toMatchObject({ kind: "page", query: "Sa" }); // never across lines
  });

  it("keeps block references at the caret: their text is an id, not a page name", () => {
    expect(detectTrigger("((ab cd))", 4)).toEqual({ kind: "block", query: "ab", start: 0, end: 4, replaceEnd: 9 });
  });
});

// CM6-D9 (Martin 2026-11-09): no menu opens inside a fenced code block, in
// either editor. A deliberate difference from Logseq, whose `[[` and `/`
// handlers ignore code (handler/editor.cljs:1606 and :1894, 6e7afa8eb).
describe("detectTrigger inside code (CM6-D9)", () => {
  it.each([["page", "[[Top"], ["block", "((Top"], ["tag", "#Top"], ["command", "/Top"], ["advanced", "<Top"], ["property", "alpha::"]])(
    "opens no %s menu in a fence body or on its closer line", (_kind, typed) => {
      for (const raw of [`\`\`\`\n${typed}`, `\`\`\`js\nx = 1\n${typed}\n\`\`\``, `intro\n~~~\n${typed}\n~~~`])
        expect(detectTrigger(raw, raw.indexOf(typed) + typed.length)).toBeNull();
      expect(detectTrigger(`\`\`\`\nx\n\`\`\`${typed}`, 8 + typed.length)).toBeNull();
    });

  it("still opens menus outside the fence, in inline code, and the fence's language picker", () => {
    expect(detectTrigger("```\nx\n```\n[[Top", 15)).toMatchObject({ kind: "page", query: "Top" });
    expect(detectTrigger("`[[Top", 6)).toMatchObject({ kind: "page", query: "Top" });
    expect(detectTrigger("```js", 5)).toEqual({ kind: "code-language", query: "js", start: 3, end: 5 });
  });
});
