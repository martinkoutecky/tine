import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("I-12: native preview/count clients use tine_core::projection; imitate query.rs", () => {
  for (const path of ["crates/tine-store/src/query.rs", "crates/tine-graph-features/src/render.rs"]) {
    const source = readFileSync(path, "utf8");
    expect(source).toContain("use tine_core::projection::{block_to_bounded_dto, subtree_node_count}");
    expect(source).not.toMatch(/fn (?:subtree_node_count|block_to_bounded_dto|bounded_preview_dto)\(/);
  }
});

it("I-12: published previews use previewProjection.ts before cloning", () => {
  const source = readFileSync("src/publishedBackend.ts", "utf8");
  const preview = source.slice(source.indexOf("    async previewBlock("), source.indexOf("    // ---- search ----"));
  expect(preview).toContain('previewDtoSubtree(found.block, maxNodes, "owned")');
  expect(preview).not.toMatch(/structuredClone|const copy|const count/);
});
