import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

function sourceFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(dir, entry.name);
    if (entry.isDirectory()) return sourceFiles(file);
    return /\.tsx?$/.test(file) && !/\.test\.tsx?$/.test(file) ? [file] : [];
  });
}

export function errorDispatchViolations(file: string, source: string): string[] {
  const lines = source.split("\n");
  return lines.flatMap((line, index) =>
    /(?:String\s*\([^)]*\)|\b\w+\.message)\s*\.\s*(?:includes|indexOf|startsWith|match)\s*\(/.test(line)
      ? [`${file}:${index + 1}: error prose drives a family branch`] : []
  );
}

function assertErrorDispatch(file: string, source: string): void {
  const violations = errorDispatchViolations(file, source);
  if (violations.length) throw new Error(
    `I-9: dispatch on fixed errorFamily(e) tokens, not substrings of wire prose; exemplar src/persistence.ts doSave.\n${violations.join("\n")}`
  );
}

describe("I-9 error dispatch scan", () => {
  it("keeps application error-family branches off prose substrings", () => {
    const root = process.cwd();
    for (const absolute of sourceFiles(path.join(root, "src"))) {
      assertErrorDispatch(path.relative(root, absolute), readFileSync(absolute, "utf8"));
    }
  });

  it("fails a planted substring dispatch", () => {
    expect(() => assertErrorDispatch("src/planted.ts", 'if (String(e).includes("conflict")) retry();'))
      .toThrow(/I-9.*exemplar src\/persistence\.ts/s);
  });
});
