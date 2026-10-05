import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { callsImported, declaresArrayConstant, declaresFunction, importsFrom } from "./tests/sourceOwnership";

const source = (path: string) => readFileSync(path, "utf8");
describe("OG-DUPAL1 shared computation owners", () => {
  it("F04 I-12: leaf classification belongs to editor/queryIr.ts and native visitation to Filter::visit_leaves", () => {
    for (const path of ["src/editor/queryBuilder.ts", "src/components/querySheetParts.tsx"]) {
      expect(importsFrom(path, "isLeafLike", "src/editor/queryIr"), `I-12: ${path} must import isLeafLike from editor/queryIr.ts`).toBe(true);
      expect(declaresFunction(path, "isLeafLike"), `I-12: ${path} must not declare its own isLeafLike`).toBe(false);
    }
    const native = source("crates/tine-core/src/query/ir.rs");
    for (const name of ["any_leaf", "for_each_leaf"]) {
      const start = native.indexOf(`pub fn ${name}`);
      const end = native.indexOf("\n    }", start);
      expect(native.slice(start, end), "I-12: use Filter::visit_leaves; preserve ControlFlow short-circuiting").toContain("self.visit_leaves(");
    }
  });
  it("F06 I-12: sanitizer inventories belong to fixtures/html-sanitize-policy.json", () => {
    expect(importsFrom("src/render/htmlSanitize.ts", "default", "fixtures/html-sanitize-policy.json")).toBe(true);
    expect(source("crates/tine-core/src/html_sanitize.rs")).toContain('include_str!("../../../fixtures/html-sanitize-policy.json")');
    expect(declaresArrayConstant("src/render/htmlSanitize.ts", /^RAW_HTML_(?:TAGS|ATTRS)$/)).toBe(false);
    expect(source("crates/tine-core/src/html_sanitize.rs")).not.toMatch(/const TAGS|tag_attrs\.insert/);
  });
  it("F09 I-12: browser URL formatting belongs to render/urlDest.ts", () => {
    for (const path of ["src/render/inline.tsx", "src/render/renderedText.ts"]) {
      expect(importsFrom(path, "urlDest", "src/render/urlDest"), `I-12: ${path} must import urlDest from render/urlDest.ts`).toBe(true);
      expect(declaresFunction(path, "urlDest"), `I-12: ${path} must not declare its own urlDest`).toBe(false);
    }
  });
  it("F11 I-12: schema guard mechanics belong to schemaGuards.ts; schemas own diagnostics and plain-text policy", () => {
    for (const path of ["src/plugins/manifest.ts", "src/plugins/settings.ts", "src/plugins/registry.ts", "src/themes/manifest.ts"]) {
      expect(callsImported(path, "schemaGuards", "src/schemaGuards"), `I-12: ${path} must build its guards with schemaGuards()`).toBe(true);
      for (const name of ["record", "object", "knownKeys", "stringField", "text"]) {
        expect(declaresFunction(path, name), `I-12: ${path} must not declare its own ${name}()`).toBe(false);
      }
    }
  });
});
