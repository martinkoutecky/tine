// **The one macro-name list, guarded** (SPEC §7.9, Y1; I-12).
//
// Two guards, in the house pattern of the `hmac::verify` source-count ratchet and
// `src/plugins/capabilityBoundary.test.ts`:
//
//  1. **Doc-code consistency.** The TypeScript constant and the Rust constant are
//     compared by READING the Rust source, so the pair cannot drift silently.
//
//  2. **The source scan.** Every `{{query`-shaped matcher or writer in `src/` and
//     `crates/` must read one of the two shared constants. This is the CHEAP half
//     of a guard pair: a grep cannot prove reachability, so it exists to make a
//     NEW hand-spelled macro name visible in review — which is the failure Y1
//     names. A packet may not write a macro name its own tree cannot recognise,
//     re-edit or export, and the way that happened was a second regex somewhere
//     the first author never looked.
//
// Both list a blessed exemplar to imitate, deliberately: a guard whose message
// only says "the count changed" trains the next agent to re-pin the count.

import { describe, it, expect } from "vitest";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { QUERY_MACRO_NAMES, QUERY_MACRO_SCAFFOLD } from "./editor/queryMacroName";

const REPO = fileURLToPath(new URL("..", import.meta.url));
const SRC = join(REPO, "src");
const CRATES = join(REPO, "crates");
const RUST_CONSTANT_FILE = join(CRATES, "tine-core/src/query/macro_names.rs");

const RULE =
  `I-12 / SPEC §7.9 (Y1) — a query macro name is spelled in ONE place per language.\n`
  + `  TypeScript: QUERY_MACRO_NAMES in src/editor/queryMacroName.ts\n`
  + `  Rust:       QUERY_MACRO_NAMES in crates/tine-core/src/query/macro_names.rs\n`
  + `A packet may not write a macro name its own tree cannot recognise, re-edit or\n`
  + `export — which is what happens when a new matcher spells "query" inline and the\n`
  + `{{tine-query}} half of the pair silently falls through to literal text.\n`
  + `\n`
  + `EXEMPLARS TO IMITATE (copy one of these, do not invent a third shape):\n`
  + `  • recognising a macro by NAME:   src/editor/queryMacro.ts::isQueryMacroName\n`
  + `                                   crates/tine-core/src/query/macro_text.rs::is_query_macro_name\n`
  + `  • finding macros in RAW source:  src/editor/queryMacro.ts::queryMacroExtents\n`
  + `                                   crates/tine-core/src/query/macro_text.rs::query_macro_extents\n`
  + `  • WRITING a macro:               src/components/Macro.tsx::rewriteMacro (reads the authored name)\n`
  + `                                   src/editor/queryMacroName.ts::QUERY_MACRO_SCAFFOLD (new empty macro)\n`
  + `\n`
  + `If your file legitimately contains a {{query …}} literal as DATA — a Guide page,\n`
  + `a sample graph, a test corpus — add it to DATA_FILES below with the reason.`;

/** Files whose `{{query …}}` occurrences are content, not code: they do not
 *  RECOGNISE or WRITE a macro name, they contain one the way a document does.
 *  Listed rather than pattern-matched so a new one is a review decision. */
const DATA_FILES: readonly { file: string; why: string }[] = [
  {
    file: "src/mock.ts",
    why: "the browser dev-preview's sample graph: block bodies, the same as any user's notes.",
  },
  {
    file: "crates/tine-core/src/guide.rs",
    why: "the shipped Guide pages' Markdown, plus the assertions that they copy verbatim.",
  },
  {
    file: "crates/tine-graph-features/src/render.rs",
    why: "og debt, not data: the published-HTML export's own {{query}} dispatch. Published export is "
      + "X-class for the og campaign (12-common) and was not ported; it renders only `query`. "
      + "This row may only be removed, never joined by another.",
  },
];

function sourceFiles(dir: string, extensions: RegExp, acc: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    // `crates/*/tests` and `crates/*/examples` are Rust test/bench code by
    // construction — the same exclusion `.test.ts` gets on the TypeScript side.
    if (entry === "node_modules" || entry === "target" || entry === "vendor") continue;
    if (entry === "tests" || entry === "examples" || entry === "benches") continue;
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) {
      sourceFiles(full, extensions, acc);
      continue;
    }
    if (!extensions.test(entry)) continue;
    // Test sources are excluded: a test's job is to spell literal document bytes.
    // A Rust `X_tests.rs` is the body of `#[cfg(test)] #[path = "X_tests.rs"] mod
    // tests;`, test code by the same convention the tine-core census uses
    // (`projection_producer_census::scan_production_rust`).
    if (/\.test\.tsx?$/.test(entry) || /_tests\.rs$/.test(entry)) continue;
    acc.push(full);
  }
  return acc;
}

/** Rust `#[cfg(test)]` items are test code in a production file. Strip each such
 *  item and nothing else. A one-line item (`use …;`, `mod tests;`) ends at its
 *  line. A block item (`mod tests {`, `fn … {`) ends at the next column-0 `}`,
 *  which is where rustfmt closes a top-level item. Cutting from the first
 *  `#[cfg(test)]` to end of file instead would hide every production line after a
 *  test-only `use` near the top, and the guard would pass blind (I-11). */
export function productionRust(text: string): string {
  const lines = text.split("\n");
  const kept: string[] = [];
  for (let i = 0; i < lines.length; i++) {
    if (lines[i].trim() !== "#[cfg(test)]") {
      kept.push(lines[i]);
      continue;
    }
    let j = i + 1;
    while (j < lines.length && /^\s*#\[/.test(lines[j])) j++;
    if (j < lines.length && /[{]\s*$/.test(lines[j])) {
      const indent = /^\s*/.exec(lines[j])![0];
      while (j < lines.length && lines[j] !== `${indent}}`) j++;
    } else {
      while (j < lines.length && !/;\s*$/.test(lines[j])) j++;
    }
    i = j;
  }
  return kept.join("\n");
}

/** A `{{query`-shaped matcher or writer: a literal macro opening, or a regex
 *  anchored on the bare macro name the dispatch used to use. */
const SHAPES = [
  /\{\{ ?query\b/i,
  /\{\{ ?tine-query\b/i,
  /\\\{\\\{ ?query/i,
  /\^query\\b/i,
  /"query"\s*=>/,
];

/** Whether a file reads one of the two shared constants (directly, or through a
 *  helper whose whole job is to read it). */
const READS_SHARED = [
  "QUERY_MACRO_NAMES",
  "QUERY_MACRO_SCAFFOLD",
  "isQueryMacroName",
  "is_query_macro_name",
  "queryMacroExtent",
  "query_macro_extent",
  "macroNameForDialect",
  "macroTextDialect",
  "macroPrintDialect",
];

describe("the one query macro-name list (§7.9)", () => {
  it("keeps the TypeScript and Rust constants byte-identical in content", () => {
    const rust = readFileSync(RUST_CONSTANT_FILE, "utf8");
    const match = /pub const QUERY_MACRO_NAMES: \[&str; (\d+)\] = \[([^\]]*)\];/.exec(rust);
    expect(
      match,
      `No \`pub const QUERY_MACRO_NAMES\` in ${relative(REPO, RUST_CONSTANT_FILE)}.\n\n${RULE}`,
    ).not.toBeNull();
    const declaredLength = Number(match![1]);
    const names = match![2]
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean)
      .map((s) => {
        const literal = /^"(.*)"$/.exec(s);
        expect(literal, `Rust QUERY_MACRO_NAMES entry ${s} is not a plain string literal`).not.toBeNull();
        return literal![1];
      });
    expect(names.length, "the Rust array's declared length disagrees with its contents")
      .toBe(declaredLength);
    // Compared as SETS, then as sorted lists: §7.9 fixes the two names, and both
    // readers take the longest match, so the array ORDER is deliberately not
    // semantics. Requiring identical order anyway would invite a future author to
    // "fix" one side's order and think they had changed behaviour.
    expect(
      [...names].sort(),
      `The Rust and TypeScript macro-name lists disagree.\n\n${RULE}`,
    ).toEqual([...QUERY_MACRO_NAMES].sort());
    expect([...QUERY_MACRO_NAMES].sort()).toEqual(["query", "tine-query"]);
  });

  it("derives the insert scaffold from the list rather than spelling it", () => {
    // Two entry points insert an empty macro (the slash menu and the visual
    // builder). Deriving both from the constant is what keeps them from drifting
    // apart from each other and from the readers.
    expect(QUERY_MACRO_SCAFFOLD).toBe(`{{${QUERY_MACRO_NAMES[0]} }}`);
  });

  it("has no {{query}}-shaped matcher or writer that does not read the shared list", () => {
    const files = [
      ...sourceFiles(SRC, /\.tsx?$/),
      ...sourceFiles(CRATES, /\.rs$/),
    ];
    const dataFiles = new Set(DATA_FILES.map((row) => row.file));
    const offenders: string[] = [];
    for (const file of files) {
      const name = relative(REPO, file).replaceAll("\\", "/");
      if (dataFiles.has(name)) continue;
      const raw = readFileSync(file, "utf8");
      const text = file.endsWith(".rs") ? productionRust(raw) : raw;
      // Comments and doc comments describe the macro constantly; they match no
      // bytes at runtime, so they are not matchers.
      const code = text
        .split("\n")
        .filter((line) => !/^\s*(\/\/|\/\*|\*|#!|\/\/!|\/\/\/)/.test(line))
        .join("\n");
      if (!SHAPES.some((shape) => shape.test(code))) continue;
      if (READS_SHARED.some((symbol) => raw.includes(symbol))) continue;
      offenders.push(name);
    }
    expect(
      offenders.sort(),
      `These files match or write a query macro name without reading the shared list:\n`
      + `  ${offenders.join("\n  ")}\n\n${RULE}`,
    ).toEqual([]);
  });

  it("keeps the DATA_FILES census honest (an entry that stopped matching is stale)", () => {
    // Equality in both directions: a data file that no longer contains a macro
    // literal must leave the census, so the list cannot quietly become a
    // permanent exemption nobody rechecks.
    for (const row of DATA_FILES) {
      const text = readFileSync(join(REPO, row.file), "utf8");
      expect(
        SHAPES.some((shape) => shape.test(text)),
        `${row.file} is censused as containing macro-name DATA (${row.why}) but no longer does.\n`
        + `Remove the row.\n\n${RULE}`,
      ).toBe(true);
    }
  });
});
