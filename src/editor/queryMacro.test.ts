// The frontend half of the SHARED raw-macro-reader corpus (SPEC §4.3.1, §7.9).
//
// `crates/tine-core/src/query/fixtures/query-macro/extents.json` is read here and by
// `crates/tine-core/tests/query_macro_extents.rs`. Both must recover the same
// `{text, name, argument}` for every case. That agreement is the ONLY reason
// §4.3.1 permits a reader on each side: rendering, in-place rewriting and Export
// are synchronous walks that cannot make an IPC call per macro, so the twin is
// paid for with a fixture set rather than pretended away.
//
// Offsets are deliberately not compared across the pair — Rust reports byte
// offsets, JavaScript UTF-16 code units — so the recovered TEXT is the contract.

import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import {
  QUERY_MACRO_NAMES,
  formFamilyForMacroName,
  isQueryMacroName,
  queryMacroExtent,
  queryMacroExtentAtSpan,
  queryMacroExtents,
} from "./queryMacro";

interface Case {
  why: string;
  raw: string;
  extents: { text: string; name: string; argument: string }[];
}

const FIXTURES = fileURLToPath(
  new URL("../../crates/tine-core/src/query/fixtures/query-macro/extents.json", import.meta.url),
);
const CASES: Case[] = JSON.parse(readFileSync(FIXTURES, "utf8"));

describe("the raw query-macro extent reader", () => {
  it("matches the shared Rust fixtures on every case", () => {
    for (const c of CASES) {
      const found = queryMacroExtents(c.raw);
      expect(found.length, `extent COUNT for ${JSON.stringify(c.raw)} (${c.why})`)
        .toBe(c.extents.length);
      found.forEach((extent, i) => {
        const expected = c.extents[i];
        expect(c.raw.slice(extent.start, extent.end), `extent SLICE for ${JSON.stringify(c.raw)}`)
          .toBe(expected.text);
        expect(extent.name, `macro NAME for ${JSON.stringify(c.raw)}`).toBe(expected.name);
        expect(
          extent.argument,
          `raw ARGUMENT for ${JSON.stringify(c.raw)} (${c.why}) — the byte contract the AST cannot meet`,
        ).toBe(expected.argument);
      });
    }
  });

  it("reads the corpus at all (a silently empty fixture file would pass everything)", () => {
    expect(CASES.length).toBeGreaterThanOrEqual(15);
    expect(CASES.some((c) => c.raw.includes("{{tine-query"))).toBe(true);
  });

  it("finds the first macro, and nothing when there is none", () => {
    expect(queryMacroExtent("not a macro")).toBeNull();
    const raw = "A {{query (task TODO)}} B {{tine-query @block}}";
    expect(queryMacroExtent(raw)?.argument).toBe("(task TODO)");
  });

  it("anchors an AST macro to its extent by SPAN, in the parser's own coordinates", () => {
    const raw = "A {{query (task TODO)}} B {{tine-query @block and [[b]]}}";
    // §4.3.1: "associate an AST query macro by source offset with its full raw
    // extent". lsdoc parses `"- " + raw.trimStart()` and reports UTF-8 byte
    // offsets into THAT, so a macro at index i is reported at span start i + 2
    // (the same `- 2` mapping `render/facets.ts::standaloneSourceLine` uses).
    const span = (index: number): [number, number] => [index + 2, index + 4];
    expect(queryMacroExtentAtSpan(raw, span(raw.indexOf("{{tine-query")))?.name).toBe("tine-query");
    expect(queryMacroExtentAtSpan(raw, span(raw.indexOf("{{query")))?.name).toBe("query");
    // Anchors EXACTLY: an offset INSIDE a macro is not that macro's start, and a
    // span that points at ordinary text resolves to nothing rather than to the
    // first macro.
    expect(queryMacroExtentAtSpan(raw, span(raw.indexOf("{{query") + 1))).toBeNull();
    expect(queryMacroExtentAtSpan(raw, span(0))).toBeNull();
    expect(queryMacroExtentAtSpan(raw, undefined)).toBeNull();
  });

  it("keeps the mapping right past leading whitespace and non-ASCII text", () => {
    // `parseBody` re-bullets `raw.trimStart()`, so leading whitespace is not in
    // the parser's input at all; and the offsets it reports are BYTES, so a
    // multi-byte character before the macro shifts the span but not the index.
    const raw = "  \u00e4\u00f6 {{query (task TODO)}}";
    const index = raw.indexOf("{{query");
    const leadBytes = new TextEncoder().encode("  ").length;
    const spanStart = new TextEncoder().encode(raw.trimStart()).length
      - new TextEncoder().encode("{{query (task TODO)}}").length
      + 2;
    expect(queryMacroExtentAtSpan(raw, [spanStart, spanStart + 2])?.argument).toBe("(task TODO)");
    // The naive reading — treat the span as an index into `raw` — lands
    // somewhere else entirely, which is the defect this mapping exists to avoid.
    expect(spanStart - 2 + leadBytes).not.toBe(index);
  });
});

describe("query macro NAME recognition (§7.9)", () => {
  it("knows exactly the two names, case-insensitively", () => {
    expect([...QUERY_MACRO_NAMES]).toEqual(["query", "tine-query"]);
    for (const name of ["query", "QUERY", "tine-query", "Tine-Query"]) {
      expect(isQueryMacroName(name), name).toBe(true);
    }
    for (const name of ["query-foo", "queryx", "embed", "tinequery", ""]) {
      expect(isQueryMacroName(name), name).toBe(false);
    }
  });

  it("picks the literal family from the name, not from the text", () => {
    // The form inside `{{query}}` is EDN-shaped and inside `{{tine-query}}` is
    // SQL-shaped; this is what makes `'a{b'` literal in one and not the other.
    expect(formFamilyForMacroName("query")).toBe("edn");
    expect(formFamilyForMacroName("tine-query")).toBe("tql");
    expect(formFamilyForMacroName("TINE-QUERY")).toBe("tql");
  });

  it("is not sensitive to the order of QUERY_MACRO_NAMES", () => {
    // The readers take the LONGEST match, so reordering the shared constant
    // cannot change which macro a document scan recognises. If a future reader
    // regressed to "first match wins", `{{tine-query …}}` would be read as a
    // macro named `query` with a `-query …` argument — this is that alarm.
    const extent = queryMacroExtent("{{tine-query @block}}");
    expect(extent?.name).toBe("tine-query");
    expect(extent?.argument).toBe("@block");
  });
});

// Moved from `edn.test.ts` when og's name-blind `edn.queryMacroExtent(s)` was
// retired for the one macro-name-aware scanner (I-12).
describe("queryMacroExtents keeps the retired edn scanner's guarantees", () => {
  const text = (raw: string) => queryMacroExtents(raw).map((e) => raw.slice(e.start, e.end));
  it("ignores }} inside strings and page refs and excludes trailing property lines", () => {
    const tricky = '{{query (and (task TODO)) {:title "Sprint }} board"}}}';
    expect(text(tricky)).toEqual([tricky]);
    expect(text('{{query (todo) {:title "A"}}}\nid:: abc')).toEqual(['{{query (todo) {:title "A"}}}']);
    expect(text('{{query (page [[A }} B]]) {:title "t"}}}\nid:: x')).toEqual(['{{query (page [[A }} B]]) {:title "t"}}}']);
  });
  it("finds every query macro in a block, in order, under either name", () => {
    expect(text("A {{query (task TODO)}} B {{tine-query task = TODO}}")).toEqual([
      "{{query (task TODO)}}",
      "{{tine-query task = TODO}}",
    ]);
    expect(queryMacroExtents("no queries here")).toEqual([]);
  });
});
