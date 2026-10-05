import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import ts from "typescript";
import { callsImported } from "./tests/sourceOwnership";
const read = (path: string) => readFileSync(path, "utf8");
function regexes(source: string): string[] {
  const file = ts.createSourceFile("reader.ts", source, ts.ScriptTarget.Latest, true);
  const out: string[] = [];
  const visit = (node: ts.Node) => {
    if (ts.isRegularExpressionLiteral(node)) out.push(node.text);
    ts.forEachChild(node, visit);
  };
  visit(file); return out;
}
describe("OG-DUPD1 parser ownership", () => {
  it("I-12: annotation metadata comes from facetsOf and pageHeaderProperties; imitate editor/annotation.ts", () => {
    const source = read("src/editor/annotation.ts");
    expect(regexes(source), "I-12: only asset basename separators may be regex-scanned; metadata belongs to the parser").toEqual(["/[\\\\/]/"]);
    expect(callsImported("src/editor/annotation.ts", "facetsOf", "src/render/facets"), "I-12: block annotation metadata is read through facetsOf").toBe(true);
    expect(callsImported("src/components/Block.tsx", "annotationInfo", "src/editor/annotation"), "I-12: Block reads annotation metadata through annotationInfo").toBe(true);
    expect(callsImported("src/editor/annotation.ts", "pageHeaderProperties", "src/document"), "I-12: page annotations read pageHeaderProperties").toBe(true);
  });
  it("I-12: loaded collision and merge identity read accepted ids; imitate blockIdentity.ts existingBlockId", () => {
    const model = read("src/document/model.ts");
    expect(regexes(model), "I-12: loaded identities must use existingBlockId, never raw ID patterns").toEqual([]);
    expect(callsImported("src/document/model.ts", "acceptedBlockIdentityClaims", "src/blockIdentity"), "I-12: loaded identity claims come from blockIdentity").toBe(true);
    expect(model, "I-12: loaded identity claims belong to each live node, even after AST eviction").toContain("identityClaimsByNode = new WeakMap");
    expect(model).toContain("claims.raw !== node.raw || claims.format !== format");
    const blocks = read("src/document/edits/blocks.ts");
    expect(blocks).not.toMatch(/const idPresent|const idLine/);
    expect(callsImported("src/blockIdentity.ts", "blockRegions", "src/render/parse"), "I-12: accepted ids come from the parser's blockRegions").toBe(true);
  });
  it("I-12: caret link recognition belongs to parseBlock AST spans; imitate editor/nearestLink.ts", () => {
    const source = read("src/editor/nearestLink.ts");
    expect(regexes(source), "I-12: caret links/tags may not have a second lexical grammar").toEqual([]);
    expect(callsImported("src/editor/nearestLink.ts", "parseBlock", "src/render/parse"), "I-12: caret links come from parseBlock").toBe(true);
    expect(source).toContain("inline.span");
    expect(regexes("const scan = /#\\S+/g;")).toHaveLength(1);
  });
});
