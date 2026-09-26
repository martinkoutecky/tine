import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

function releaseViolations(source: string): number[] {
  const lines = source.split("\n");
  return lines.flatMap((line, index) => {
    if (!/\breleaseSourcesFor\(name\)/.test(line) || line.trimStart().startsWith("//")) return [];
    const preceding = lines.slice(Math.max(0, index - 20), index).join("\n");
    return /token\s*(?:===?|!==?)\s*graphToken/.test(preceding) && /stillBound\(binding\)/.test(preceding) ? [] : [index + 1];
  });
}

function assertBoundRelease(source: string): void {
  const found = releaseViolations(source);
  if (found.length) throw new Error(`I-20: a save may release held sources only in its captured graph binding; exemplar src/persistence.ts doSave guarded success. Lines ${found.join(", ")}`);
}

describe("I-20 held-source release", () => {
  it("keeps both save success paths behind the binding check", () => {
    assertBoundRelease(readFileSync("src/persistence.ts", "utf8"));
  });

  it("fails a planted stale release", () => {
    expect(() => assertBoundRelease("async function save() {\n await backend().savePage();\n releaseSourcesFor(name);\n}"))
      .toThrow(/I-20:.*exemplar src\/persistence\.ts/s);
  });
});
