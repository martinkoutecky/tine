import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("I-12: query macro extents compile the native reader; exemplar query/macro_extent.rs", () => {
  const source = (path: string) => readFileSync(new URL(path, import.meta.url), "utf8");
  const frontend = source("./queryMacro.ts");
  expect(frontend).toContain("query_macro_extents_json(raw)");
  expect(frontend).not.toMatch(/function\*? (?:scanBraces|macroAt|ednStringEnd|tqlStringEnd|pageRefEnd|queryMacroExtentFrom)/);
  expect(source("../../crates/lsdoc-wasm/src/lib.rs")).toContain("../../tine-core/src/query/macro_extent.rs");
  expect(source("../../crates/tine-core/src/query/macro_text.rs")).toContain("pub use super::macro_extent::");
});
