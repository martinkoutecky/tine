import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
const source = (path: string) => readFileSync(path, "utf8");
describe("OG-R4A shared boundaries", () => {
  it("isolates read_highlights on its tested worker", () => {
    const command = source("src-tauri/src/commands.rs").split("pub(crate) ").find((part) => /^(async )?fn read_highlights\(/.test(part))!;
    expect(command, "I-4: read_highlights must await its tested panic-isolating worker; imitate open_pdf").toContain("async fn read_highlights");
    expect(command).toContain("read_highlights_worker::read");
    expect(source("src-tauri/src/commands/read_highlights_worker.rs")).toContain("run(move ||");
  });
  it("owns query option structure once in Rust", () => {
    const macro = source("src/components/Macro.tsx");
    expect(macro, "I-12: query options use query_edn.rs spans; imitate editEdnTitle").toContain("editEdnTitle(opts(), title)");
    expect(macro).not.toContain("/:title");
    expect(macro).not.toContain("/:collapsed");
    expect(macro).not.toContain("/:table-view");
    for (const file of ["src/editor/edn.ts", "src/components/BeginQuery.tsx"]) {
      expect(source(file), "I-12: EDN readers belong to query_edn.rs via wasm").not.toMatch(/function (strClose|pageRefEnd|balancedEnd|valueEnd|stringEnd|tokenEnd|skipTrivia)\(/);
    }
    expect(source("crates/lsdoc-wasm/src/lib.rs")).toContain('tine-core/src/query_edn.rs');
    expect(source("crates/tine-core/src/query/macro_text.rs")).toContain("crate::query_edn::options");
  });
});
