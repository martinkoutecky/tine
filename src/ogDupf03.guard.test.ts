import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("I-12: native preview/count clients use tine_core::projection; imitate query.rs", () => {
  for (const path of ["crates/tine-store/src/query.rs", "crates/tine-graph-features/src/render.rs"]) {
    const source = readFileSync(path, "utf8");
    expect(source).toContain("use tine_core::projection::{block_to_bounded_dto, subtree_node_count}");
    expect(source).not.toMatch(/fn (?:subtree_node_count|block_to_bounded_dto|bounded_preview_dto)\(/);
  }
});

it("I-12: published previews forward to the core's bounded projection", () => {
  const source = readFileSync("src/publishedBackend.ts", "utf8");
  expect(source).toContain('read("preview_block_json", uuid, maxNodes)');
  const core = readFileSync("crates/lsdoc-wasm/src/browser_graph.rs", "utf8");
  expect(core).toContain("read::preview_block(&self.view, uuid, max_nodes)");
});

it("I-12: the demo mock previews through previewProjection.ts", () => {
  const source = readFileSync("src/mock.ts", "utf8");
  const preview = source.slice(source.indexOf("    async previewBlock("), source.indexOf("    async readAsset("));
  expect(preview).toContain("previewDtoSubtree(group.blocks[0], maxNodes)");
  expect(preview).not.toMatch(/const copy|const count/);
});
