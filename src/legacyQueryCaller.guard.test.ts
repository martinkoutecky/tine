import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

describe("Rule 3: one query answerer", () => {
  it("routes workspace and tag tables through the query IR", () => {
    for (const file of ["src/components/QueryWorkspace.tsx", "src/components/Page.tsx"]) {
      const source = readFileSync(resolve(file), "utf8");
      expect(source, `${file} still calls a legacy query answerer (Rule 3)`)
        .not.toMatch(/\.run(?:Advanced)?Query\s*\(/);
    }
    const backend = readFileSync(resolve("src/backend.ts"), "utf8");
    expect(backend).not.toMatch(/invoke\s*\(\s*["']run_(?:advanced_)?query["']/);
    const commands = readFileSync(resolve("src-tauri/src/commands.rs"), "utf8");
    expect(commands).not.toMatch(/(?:pub\s+)?async\s+fn\s+run_(?:advanced_)?query\b/);
    const registration = readFileSync(resolve("src-tauri/src/lib.rs"), "utf8");
    expect(registration).not.toMatch(/commands::run_(?:advanced_)?query\b/);
  });
});
