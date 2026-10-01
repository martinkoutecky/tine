import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import ts from "typescript";
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
    expect(source).toContain("block.properties ?? facetsOf(");
    expect(read("src/components/Block.tsx")).toContain("annotationInfo(propertySession.facets(node().raw, pageFmt()).properties)");
    expect(source).toContain("pageHeaderProperties({");
  });
  it("I-12: loaded collision and merge identity read accepted ids; imitate blockIdentity.ts existingBlockId", () => {
    const model = read("src/document/model.ts");
    expect(regexes(model), "I-12: loaded identities must use existingBlockId, never raw ID patterns").toEqual([]);
    expect(model).toContain("acceptedBlockIdentityClaims(node.raw, format)");
    expect(model, "I-12: loaded identity claims belong to each live node, even after AST eviction").toContain("identityClaimsByNode = new WeakMap");
    expect(model).toContain("claims.raw !== node.raw || claims.format !== format");
    const blocks = read("src/document/edits/blocks.ts");
    expect(blocks).not.toMatch(/const idPresent|const idLine/);
    expect(read("src/blockIdentity.ts")).toContain("blockRegions(raw, format).id");
  });
  it("I-12: caret link recognition belongs to parseBlock AST spans; imitate editor/nearestLink.ts", () => {
    const source = read("src/editor/nearestLink.ts");
    expect(regexes(source), "I-12: caret links/tags may not have a second lexical grammar").toEqual([]);
    expect(source).toContain("parseBlock(text,");
    expect(source).toContain("inline.span");
    expect(regexes("const scan = /#\\S+/g;")).toHaveLength(1);
  });
});
