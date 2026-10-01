import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = (path: string) => readFileSync(path, "utf8");
describe("OG-DUPAL1 shared computation owners", () => {
  it("F04 I-12: leaf classification belongs to editor/queryIr.ts and native visitation to Filter::visit_leaves", () => {
    for (const path of ["src/editor/queryBuilder.ts", "src/components/querySheetParts.tsx"]) {
      expect(source(path), `I-12: ${path} must import isLeafLike from editor/queryIr.ts`).toContain('import { isLeafLike }');
      expect(source(path)).not.toMatch(/function isLeafLike/);
    }
    const native = source("crates/tine-core/src/query/ir.rs");
    for (const name of ["any_leaf", "for_each_leaf"]) {
      const start = native.indexOf(`pub fn ${name}`);
      const end = native.indexOf("\n    }", start);
      expect(native.slice(start, end), "I-12: use Filter::visit_leaves; preserve ControlFlow short-circuiting").toContain("self.visit_leaves(");
    }
  });
  it("F06 I-12: sanitizer inventories belong to fixtures/html-sanitize-policy.json", () => {
    expect(source("src/render/htmlSanitize.ts")).toContain('import policy from "../../fixtures/html-sanitize-policy.json"');
    expect(source("crates/tine-core/src/html_sanitize.rs")).toContain('include_str!("../../../fixtures/html-sanitize-policy.json")');
    expect(source("src/render/htmlSanitize.ts")).not.toMatch(/RAW_HTML_(?:TAGS|ATTRS) = \[/);
    expect(source("crates/tine-core/src/html_sanitize.rs")).not.toMatch(/const TAGS|tag_attrs\.insert/);
  });
  it("F09 I-12: browser URL formatting belongs to render/urlDest.ts", () => {
    for (const path of ["src/render/inline.tsx", "src/render/renderedText.ts"]) {
      expect(source(path)).toContain('import { urlDest } from "./urlDest"');
      expect(source(path)).not.toMatch(/function urlDest/);
    }
  });
  it("F11 I-12: schema guard mechanics belong to schemaGuards.ts; schemas own diagnostics and plain-text policy", () => {
    for (const path of ["src/plugins/manifest.ts", "src/plugins/settings.ts", "src/plugins/registry.ts", "src/themes/manifest.ts"]) {
      expect(source(path)).toContain("schemaGuards(");
      expect(source(path)).not.toMatch(/function (?:record|object|knownKeys|stringField|text)\(/);
    }
  });
});
