import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import ts from "typescript";

function structuralRegexes(source: string): string[] {
  const file = ts.createSourceFile("paste.ts", source, ts.ScriptTarget.Latest, true);
  const found: string[] = [];
  const visit = (node: ts.Node) => {
    if (ts.isRegularExpressionLiteral(node) && node.text.includes("A-Za-z0-9") && node.text.includes(":")) found.push(node.text);
    ts.forEachChild(node, visit);
  };
  visit(file);
  return found;
}

describe("OG-P11 parser ownership", () => {
  it("I-12: clipboard property acceptance belongs to render/parse.ts blockRegions", () => {
    const source = readFileSync("src/document/edits/paste.ts", "utf8");
    expect(structuralRegexes(source)).toEqual([]);
    expect(source).toContain("blockRegions(raw, format)");
  });
  it("detects an ASCII property scanner planted back in the client", () => {
    expect(structuralRegexes("const key = /^([A-Za-z0-9_]+)::/.exec(raw);")).toHaveLength(1);
  });
  it("I-12: asset targets come from tine_core::render::parse_inline_bounded, never delimiter scanning", () => {
    const source = readFileSync("crates/tine-store/src/model.rs", "utf8");
    const collector = source.slice(source.indexOf("pub(crate) fn collect_asset_refs("), source.indexOf("fn insert_asset_path("));
    expect(collector).toContain("tine_core::render::parse_inline_bounded");
    expect(collector).toContain("Inline::Link");
    expect(collector).not.toMatch(/\.find\(|\.lines\(|Regex|regex/);
    expect(collector).toContain("conservative_asset_mentions(text, into)");
  });
  it("I-12: durability race tests call src-tauri/src/device_io.rs's production updater", () => {
    const store = readFileSync("crates/tine-store/src/model.rs", "utf8");
    const production = readFileSync("src-tauri/src/device_io.rs", "utf8");
    expect(store).not.toContain("fn atomic_update_with_hooks(");
    for (const name of ["atomic_update_retries_on_external_change_without_losing_it", "atomic_update_absent_publish_preserves_a_concurrent_creator"]) {
      expect(store).not.toContain(`fn ${name}`);
      expect(production).toContain(`fn ${name}`);
    }
    expect(production).not.toContain("CONFIG_LOCK");
  });
});


function outlineLiteralScanners(source: string): string[] {
  const file = ts.createSourceFile("outline.ts", source, ts.ScriptTarget.Latest, true);
  const found: string[] = [];
  const visit = (node: ts.Node) => {
    if (ts.isRegularExpressionLiteral(node) && /[`~]|BEGIN_(?:SRC|EXAMPLE|QUOTE)/i.test(node.text)) found.push(node.text);
    if (ts.isNewExpression(node) && node.expression.getText(file) === "RegExp") found.push(node.getText(file));
    if (ts.isStringLiteralLike(node) && /BEGIN_(?:SRC|EXAMPLE|QUOTE)|```|~~~/i.test(node.text)) found.push(node.text);
    ts.forEachChild(node, visit);
  };
  visit(file);
  return found;
}

describe("OG-P11B one parser door", () => {
  it("I-12: outline literals belong to render/parse.ts blockRegions, never a fence/src recognizer", () => {
    const source = readFileSync("src/editor/outline.ts", "utf8");
    expect(source).toContain("blockRegions(normalized).literals");
    expect(outlineLiteralScanners(source)).toEqual([]);
  });
  it("detects planted fence and src recognition in outline paste", () => {
    for (const source of ["const fence = /^`{3,}/.exec(line);", 'const src = line.startsWith("#+BEGIN_SRC");', 'const close = new RegExp(marker);']) {
      expect(outlineLiteralScanners(source)).not.toEqual([]);
    }
  });
  it("I-12: BEGIN_QUERY payloads use query_edn::inspect_begin_query via native and wasm adapters", () => {
    const native = readFileSync("crates/tine-graph-features/src/render.rs", "utf8");
    const wasm = readFileSync("crates/lsdoc-wasm/src/lib.rs", "utf8");
    const live = readFileSync("src/components/BeginQuery.tsx", "utf8");
    expect(native).toContain("tine_core::query_edn::inspect_begin_query(payload)");
    expect(wasm).toContain("query_edn::inspect_begin_query(source)");
    expect(live).toContain('query_edn_json(container[1], "begin_query", "")');
    expect(native).not.toMatch(/fn (?:skip_edn_trivia|edn_\w+_end|parse_begin_query_map|unquote_begin_query_title|has_edn_keyword)\(/);
    expect(live).not.toMatch(/readEdn|ednSlice|unquoteEdnString|function queryMap/);
  });
});
