import { expect, it } from "vitest";
import { readFileSync } from "node:fs";

it("I-12: namespace identity calls pageIdentityKey (exemplar src/pageIdentity.ts)", () => {
  const source = readFileSync("src/components/Namespace.tsx", "utf8");
  expect(source).toContain("pageIdentityKey(");
  expect(source).not.toMatch(/identityFold\(|toLowerCase\(/);
});

it("I-12: published name, block and evidence answers come from the browser core", () => {
  const source = readFileSync("src/publishedBackend.ts", "utf8");
  expect(source).toContain("graph.resolve_page_json(");
  expect(source).toContain("graph.resolve_blocks_json(");
  expect(source).toContain("graph.run_graph_search_json(");
  expect(source).not.toMatch(/blockRegions\(|raw\.includes\(|searchSubstringSpans\(|pageIdentityKey\(/);
});


it("I-22: the table render uses the width answerer tested on populated 150k-row fixtures (exemplar src/render/tableV2.tsx)", () => {
  const source = readFileSync("src/render/tableV2.tsx", "utf8");
  expect(source, "TableV2 must call the bounded width answerer, I-22 exemplar tableColumnCount").toContain("const columnCount = tableColumnCount(props.table)");
  expect(source).not.toMatch(/Math\.max\([^;]*\.\.\./);
});
