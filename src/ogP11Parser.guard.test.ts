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
