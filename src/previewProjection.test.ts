import { expect, it } from "vitest";
import { previewDtoSubtree } from "./previewProjection";
import type { BlockDto } from "./types";

it("keeps browser node-only policy, preorder and exact omitted counts", () => {
  const leaf = (id: string): BlockDto => ({ id, raw: "x", collapsed: false, children: [] });
  const root = { ...leaf("root"), raw: "x".repeat(1024 * 1024), children: [{ ...leaf("a"), children: [leaf("b")] }, leaf("c")] };
  for (const limit of [-1, 0, 1, 2, 3, 4, 10]) {
    const result = previewDtoSubtree(root, limit, "owned");
    const ids = (blocks: BlockDto[]): string[] => blocks.flatMap(b => [b.id, ...ids(b.children)]);
    const count = Math.min(4, Math.max(1, limit));
    expect(ids(result.blocks)).toEqual(["root", "a", "b", "c"].slice(0, count));
    expect(result.truncated).toBe(4 - count);
    expect(result.blocks[0].raw).toBe(root.raw);
  }
});


it("names snapshot ownership without increasing the mock's shallow-copy allocation", () => {
  const root: BlockDto = {id: "root", raw: "x", collapsed: false, children: [], tags: ["tag"]};
  expect(previewDtoSubtree(root, 1, "borrowed").blocks[0].tags).toBe(root.tags);
  expect(previewDtoSubtree(root, 1, "owned").blocks[0].tags).not.toBe(root.tags);
});
