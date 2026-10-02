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

function propertyOrMarkerScanners(source: string): string[] {
  const file = ts.createSourceFile("client.ts", source, ts.ScriptTarget.Latest, true);
  const found: string[] = [];
  const visit = (node: ts.Node) => {
    if (ts.isRegularExpressionLiteral(node) && /::|TODO|DOING|\[\^?ABC/.test(node.text)) found.push(node.text);
    if (ts.isNewExpression(node) && node.expression.getText(file) === "RegExp") found.push(node.getText(file));
    if (ts.isCallExpression(node) && ts.isPropertyAccessExpression(node.expression)
        && ["indexOf", "lastIndexOf", "split", "startsWith"].includes(node.expression.name.text)
        && node.arguments.some((arg) => ts.isStringLiteralLike(arg) && /::|TODO|DOING/.test(arg.text))) {
      found.push(node.getText(file));
    }
    ts.forEachChild(node, visit);
  };
  visit(file);
  return found;
}

describe("OG-P12 parser-owned properties, references and header spans", () => {
  it("I-12: property clients use render/parse.ts blockRegions or native property_line_json; imitate sheet/renameField.ts", () => {
    const rename = readFileSync("src/sheet/renameField.ts", "utf8");
    expect(rename).toContain("blockRegions(raw, format).properties");
    expect(rename).not.toMatch(/mdOccurrence|orgOccurrence|PROP_LINE/);
    expect(propertyOrMarkerScanners(rename)).toEqual([]);
    const block = readFileSync("src/render/block.ts", "utf8");
    const lineReader = block.slice(block.indexOf("export function isPropertyLine"), block.indexOf("/** A page-property"));
    expect(lineReader).toContain("acceptedPropertyLine(line)");
    expect(propertyOrMarkerScanners(lineReader)).toEqual([]);
    const props = readFileSync("src/editor/properties.ts", "utf8");
    for (const name of ["readPropertyValue", "upsertPropertyLine", "pagePropertyEntries"]) {
      const start = props.indexOf(`export function ${name}`);
      const next = props.indexOf("\nexport ", start + 1);
      expect(propertyOrMarkerScanners(props.slice(start, next < 0 ? undefined : next))).toEqual([]);
    }
    expect(props).toContain("property_line_json(line)");
    expect(props).toContain("page_regions_json(source, true)");
  });
  it("I-12: reference targets and accepted nested nodes use native block_regions policies; imitate render/pageRefs.ts", () => {
    const frontend = readFileSync("src/render/pageRefs.ts", "utf8");
    expect(frontend).toContain("reference_target_name(");
    expect(frontend).toContain("nested_reference_names(inline.content)");
    expect(frontend).not.toMatch(/decode_page_name|lastIndexOf|new RegExp|\.indexOf\(|\.match(?:All)?\(|\.exec\(|\.test\(/);
    const native = readFileSync("crates/tine-core/src/reference_evidence.rs", "utf8");
    expect(native).toContain("crate::block_regions::reference_target_name(");
    expect(native).not.toMatch(/fn nested_names|fn local_asset/);
    const model = readFileSync("crates/tine-store/src/model.rs", "utf8");
    const references = model.slice(model.indexOf("fn property_refs"), model.indexOf("pub(crate) enum Withdrawal"));
    expect(references).not.toContain("parse_property_line");
    expect(references).toContain("reference_evidence::linkable_property_names(");
    expect(references).toContain("projection.reference_source()");
  });
  it("recognizes planted second property and task grammars", () => {
    for (const source of ['const p = /^foo::/.test(raw)', 'const m = /^(TODO|DOING) /.exec(raw)', 'const p = raw.indexOf("::")', 'const m = raw.startsWith("TODO ")', 'const p = new RegExp(grammar)']) {
      expect(propertyOrMarkerScanners(source)).toHaveLength(1);
    }
  });
});

// OG-P12B: header properties, code fences and task markers each have ONE answerer, the parser.
function fenceRecognizers(source: string): string[] {
  const file = ts.createSourceFile("client.ts", source, ts.ScriptTarget.Latest, true);
  const found: string[] = [];
  const fence = /`{3}|~{3}|BEGIN_(?:SRC|EXAMPLE|QUOTE)/i;
  const visit = (node: ts.Node) => {
    if (ts.isRegularExpressionLiteral(node) && (/[`~]{2,}|[`~]\{3|BEGIN_/i.test(node.text) || /\[`~\]/.test(node.text))) found.push(node.text);
    if (ts.isNewExpression(node) && node.expression.getText(file) === "RegExp") {
      const pattern = node.arguments?.[0];
      const text = !pattern ? "" : ts.isStringLiteralLike(pattern) ? pattern.text
        : ts.isTemplateExpression(pattern) ? pattern.head.text + pattern.templateSpans.map((span) => span.literal.text).join("") : "";
      if (/[`~]\{|[`~]{2,}|BEGIN_/i.test(text) || /\[`~\]/.test(text)) found.push(node.getText(file));
    }
    if (ts.isCallExpression(node) && ts.isPropertyAccessExpression(node.expression)
        && ["indexOf", "lastIndexOf", "split", "startsWith", "endsWith", "match", "matchAll", "test", "exec"].includes(node.expression.name.text)
        && node.arguments.some((arg) => ts.isStringLiteralLike(arg) && fence.test(arg.text))) {
      found.push(node.getText(file));
    }
    ts.forEachChild(node, visit);
  };
  visit(file);
  return found;
}

function priorityOrMarkerRecognizers(source: string): string[] {
  const file = ts.createSourceFile("client.ts", source, ts.ScriptTarget.Latest, true);
  const found: string[] = [];
  const visit = (node: ts.Node) => {
    if (ts.isRegularExpressionLiteral(node) && /\\\[#|\[#|TODO|DOING|NOW|LATER|WAITING/.test(node.text)) found.push(node.text);
    if (ts.isNewExpression(node) && node.expression.getText(file) === "RegExp") found.push(node.getText(file));
    ts.forEachChild(node, visit);
  };
  visit(file);
  return found;
}

describe("OG-P12B one answerer for page headers, fences and task markers", () => {
  it("I-12: the Markdown page header is render/parse.ts-owned page_header_json; imitate editor/properties.ts pageHeader", () => {
    const props = readFileSync("src/editor/properties.ts", "utf8");
    expect(props, "I-12: page-header recognition belongs to the wasm door page_header_json (Rust block_regions::page_header)").toContain("page_header_json(raw)");
    for (const name of ["isPageHeaderPropertiesOnly", "splitPagePreamble", "pagePropertyEntries"]) {
      const start = props.indexOf(`export function ${name}`);
      const next = props.indexOf("\nexport ", start + 1);
      expect(propertyOrMarkerScanners(props.slice(start, next < 0 ? undefined : next)),
        `I-12: ${name} must read the parser's page_header_json, not scan lines`).toEqual([]);
    }
    expect(props).not.toMatch(/function parsePageHeaderPropertyLine/);
    const store = readFileSync("crates/tine-store/src/model.rs", "utf8");
    expect(store, "I-12: the Rust page-header answer is tine_core::block_regions::page_header_only").toContain("tine_core::block_regions::page_header_only(");
    expect(store).not.toMatch(/fn page_header_property_line|fn page_header_properties_only/);
    const query = readFileSync("crates/tine-store/src/query.rs", "utf8");
    expect(query).toContain("tine_core::block_regions::page_header_only(");
    const conflicts = readFileSync("crates/tine-graph-features/src/conflicts.rs", "utf8");
    expect(conflicts, "I-12: conflict preambles are read through block_regions::parse_document").toContain("tine_core::block_regions::parse_document(");
    expect(conflicts).not.toMatch(/fn pre_property\(/);
  });
  it("I-12: code fences come from the parser's literal_blocks (src/editor/fences.ts); imitate editor/codeFence.ts", () => {
    for (const path of ["src/editor/fences.ts", "src/editor/codeFence.ts", "src/editor/calc.ts", "src/editor/autocomplete.ts", "src/render/block.ts"]) {
      expect(fenceRecognizers(readFileSync(path, "utf8")),
        `I-12: ${path} must not recognize fences itself; use codeFences()/caretInFence() in src/editor/fences.ts (blockRegions literal_blocks)`).toEqual([]);
    }
    const fences = readFileSync("src/editor/fences.ts", "utf8");
    expect(fences).toContain("literal_blocks");
    expect(fences).toContain("open_fence");
    const rust = readFileSync("crates/tine-core/src/block_regions.rs", "utf8");
    expect(rust, "the unclosed-fence editor policy has exactly one home").toMatch(/fn open_fence\(/);
  });
  it("I-12: marker and priority spans come from markers.ts headerTokens (the parser's header)", () => {
    for (const path of ["src/markers.ts", "src/render/block.ts", "src/sheet/fields.ts"]) {
      expect(priorityOrMarkerRecognizers(readFileSync(path, "utf8")),
        `I-12: ${path} must not recognize markers/priority; use headerTokens() in src/markers.ts`).toEqual([]);
    }
    const format = readFileSync("src/editor/format.ts", "utf8");
    const setPriority = format.slice(format.indexOf("export function setPriority"), format.indexOf("\nexport ", format.indexOf("export function setPriority") + 1));
    expect(priorityOrMarkerRecognizers(setPriority), "I-12: setPriority must edit the parser's priority span").toEqual([]);
    expect(readFileSync("src/markers.ts", "utf8")).toContain("header_tokens_json(");
  });
  it("detects planted second fence and marker grammars", () => {
    for (const source of ['const f = /^`{3,}/.exec(line);', 'const f = /^\\s*(`{3,}|~{3,})/.exec(line)', 'const s = line.startsWith("#+BEGIN_SRC");', 'const g = new RegExp("^(`{3,}|~{3,})")', 'const t = line.match("```")']) {
      expect(fenceRecognizers(source)).not.toEqual([]);
    }
    for (const source of ['const p = /^\\[#[ABC]\\]/.exec(raw)', 'const m = /^(TODO|DOING)\\b/.exec(raw)']) {
      expect(priorityOrMarkerRecognizers(source)).not.toEqual([]);
    }
  });
});
