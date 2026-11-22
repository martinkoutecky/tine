import type { BlockDto } from "./types";

/** Demo mock DTO preview policy: at least one node.
 * O(subtree nodes) to count omissions; copies only the admitted preorder prefix.
 * The returned DTOs share the mock's metadata.
 */
export function previewDtoSubtree(root: BlockDto, maxNodes: number): { blocks: BlockDto[]; truncated: number } {
  let emitted = 0;
  let truncated = 0;
  const count = (block: BlockDto): number => 1 + block.children.reduce((n, b) => n + count(b), 0);
  const copy = (blocks: BlockDto[]): BlockDto[] => {
    const out: BlockDto[] = [];
    for (const block of blocks) {
      if (emitted >= Math.max(1, maxNodes)) {
        truncated += count(block);
        continue;
      }
      emitted++;
      out.push({ ...block, children: copy(block.children) });
    }
    return out;
  };
  const blocks = copy([root]);
  return { blocks, truncated };
}
