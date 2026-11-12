import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import ts from "typescript";
import { describe, expect, it } from "vitest";

// A resource whose fetcher reads through `graphOwner` has its answer retired
// whenever graphEpoch moves. Unless its key moves with the same epoch, nothing
// asks again and the retired (empty or older) answer stays final; a view that
// outlives a graph switch keeps the previous graph's answer
// (UI-SWITCHER-SEARCH-OUTLIVES-EPOCH-1111, UI-GRAPH-RESOURCE-OUTLIVES-EPOCH-1112).
const RULE = "a createResource whose fetcher (or a same-file function it calls) reads through graphOwner must be "
  + "createGraphResource (src/graphResource.ts), which keys it on the same graphEpoch. Otherwise classify it below "
  + "with the key that already carries the epoch";

// Shrink-only: each entry names where its key carries graphEpoch.
const CLASSIFIED: Record<string, string> = {
  "src/resultMembership.ts": "createMembershipResource is its own keyed mechanism: `currentScope` (bindingIdentity + graphEpoch + identity) is part of every request token, so an epoch move re-reads",
};

type Sources = Map<string, string>;
function productionSources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(dir, entry.name);
    if (entry.isDirectory()) return entry.name === "tests" ? [] : productionSources(file);
    return /\.tsx?$/.test(entry.name) && !/\.test\.tsx?$|\.d\.ts$/.test(entry.name) ? [file] : [];
  });
}
function repoSources(): Sources {
  return new Map(productionSources("src").map((file) => [file.split(path.sep).join("/"), readFileSync(file, "utf8")]));
}

/** Every `createResource` call whose fetcher reaches `graphOwner(` directly or
 * through a top-level function of the same file. */
export function graphOwnedPlainResources(sources: Sources = repoSources()): string[] {
  const found: string[] = [];
  for (const [file, text] of sources) {
    if (!/\bcreateResource\s*\(/.test(text)) continue;
    const tree = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true);
    const functions = new Map<string, string>();
    const collect = (node: ts.Node) => {
      if (ts.isFunctionDeclaration(node) && node.name) functions.set(node.name.text, node.getText(tree));
      if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && node.initializer
        && (ts.isArrowFunction(node.initializer) || ts.isFunctionExpression(node.initializer))) {
        functions.set(node.name.text, node.initializer.getText(tree));
      }
      ts.forEachChild(node, collect);
    };
    collect(tree);
    const visit = (node: ts.Node) => {
      if (ts.isCallExpression(node) && node.expression.getText(tree) === "createResource") {
        const fetcher = node.arguments.length >= 2 && !ts.isObjectLiteralExpression(node.arguments[1])
          ? node.arguments[1] : node.arguments[0];
        const body = fetcher?.getText(tree) ?? "";
        const owned = /\bgraphOwner\s*\(/.test(body) || [...functions].some(([name, fn]) =>
          new RegExp(`\\b${name}\\s*\\(`).test(body) && /\bgraphOwner\s*\(/.test(fn));
        if (owned && !CLASSIFIED[file]) found.push(`${file}:${tree.getLineAndCharacterOfPosition(node.getStart()).line + 1}`);
      }
      ts.forEachChild(node, visit);
    };
    visit(tree);
  }
  return found;
}

describe("graph-owned resources are keyed on the graph epoch", () => {
  it("every createResource reading through graphOwner is a createGraphResource or classified", () => {
    expect(graphOwnedPlainResources(), RULE).toEqual([]);
  });

  it("classified entries still exist (shrink-only)", () => {
    const sources = repoSources();
    for (const file of Object.keys(CLASSIFIED)) expect(sources.get(file), file).toMatch(/\bcreateResource\s*\(/);
  });

  it("catches a planted direct and an indirect graphOwner fetcher", () => {
    const planted: Sources = new Map([
      ["src/plantedDirect.tsx", "const [r] = createResource(key, async () => readOwned(graphOwner(), work()));"],
      ["src/plantedIndirect.tsx", "async function load() { return readOwned(graphOwner(), work()); }\nconst [r] = createResource(key, () => load());"],
      ["src/plantedOnce.tsx", "const [r] = createResource(async () => readOwned(graphOwner(), work()));"],
      ["src/plantedFine.tsx", "const [r] = createGraphResource(key, async () => readOwned(graphOwner(), work()));\nconst [p] = createResource(platformKind);"],
    ]);
    expect(graphOwnedPlainResources(planted)).toEqual(["src/plantedDirect.tsx:1", "src/plantedIndirect.tsx:2", "src/plantedOnce.tsx:1"]);
  });
});
