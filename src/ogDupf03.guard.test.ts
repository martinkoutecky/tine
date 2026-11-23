import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("I-12: native preview/count clients use tine_core::projection; imitate query.rs", () => {
  for (const path of ["crates/tine-store/src/query.rs", "crates/tine-graph-features/src/render.rs"]) {
    const source = readFileSync(path, "utf8");
    expect(source).toContain("use tine_core::projection::{block_to_bounded_dto, subtree_node_count}");
    expect(source).not.toMatch(/fn (?:subtree_node_count|block_to_bounded_dto|bounded_preview_dto)\(/);
  }
});

it("I-12: published and mock previews forward to the core's bounded projection", () => {
  const source = readFileSync("src/graphForwards.ts", "utf8");
  expect(source).toContain('read("preview_block_json", uuid, maxNodes)');
  for (const backend of ["src/publishedBackend.ts", "src/mock.ts"]) {
    expect(readFileSync(backend, "utf8")).toMatch(/(\.\.\.|= )graphReads\(/);
  }
  const core = readFileSync("crates/lsdoc-wasm/src/browser_graph.rs", "utf8");
  expect(core).toContain("read::preview_block(&self.view, uuid, max_nodes)");
});
