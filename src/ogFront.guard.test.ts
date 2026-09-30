import { expect, it } from "vitest";
import { readFileSync } from "node:fs";

it("I-12: namespace and published page identity call pageIdentityKey (exemplar src/pageIdentity.ts)", () => {
  for (const path of ["src/components/Namespace.tsx", "src/publishedBackend.ts"]) {
    const source = readFileSync(path, "utf8");
    expect(source, `${path}: use pageIdentityKey for page/namespace identity; I-12 exemplar src/pageIdentity.ts`).toContain("pageIdentityKey(");
    expect(source).not.toContain("identityFold(");
  }
  expect(readFileSync("src/components/Namespace.tsx", "utf8")).not.toContain("toLowerCase(");
});

it("I-12: published IDs and highlights use the structural/provenance doors (exemplars src/render/parse.ts and src/editor/searchQuery.ts)", () => {
  const source = readFileSync("src/publishedBackend.ts", "utf8");
  expect(source, "Use blockRegions.id, never raw.includes(id::); I-12 exemplar src/render/parse.ts").toContain("blockRegions(block.raw");
  expect(source).not.toMatch(/raw\.includes\([^\n]*id::/);
  expect(source, "Map evidence with searchSubstringSpans; I-12 exemplar src/editor/searchQuery.ts").toContain("searchSubstringSpans(text, needle, 1)");
});
