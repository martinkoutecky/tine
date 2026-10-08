import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
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
  if (found.length) throw new Error(`I-20: a save may release held sources only in its captured graph binding; exemplar src/document/save/engine.ts doSave guarded success. Lines ${found.join(", ")}`);
}

function documentSources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(dir, entry.name);
    return entry.isDirectory() ? documentSources(file) : /\.tsx?$/.test(file) && !/\.test\.tsx?$/.test(file) ? [file] : [];
  });
}

describe("I-20 held-source release", () => {
  it("keeps both save success paths behind the binding check", () => {
    for (const file of documentSources("src/document")) assertBoundRelease(readFileSync(file, "utf8"));
  });

  it("fails a planted stale release", () => {
    expect(() => assertBoundRelease("async function save() {\n await backend().savePages();\n releaseSourcesFor(name);\n}"))
      .toThrow(/I-20:.*exemplar src\/document\/save\/engine\.ts/s);
  });
});
